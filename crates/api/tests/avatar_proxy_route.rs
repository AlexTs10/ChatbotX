//! Port of the "avatar media proxy" part of
//! `apps/builder/__tests__/media-proxy-route.test.ts` (`/media/avatar/{token}`,
//! `resolveMediaRequest`) over a migrated database, a scripted Graph API and
//! a `queue.add` spy. The attachment proxy cases are not in this unit.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chatbotx_api::avatar_proxy::serve;
use chatbotx_api::handlers::ApiState;
use chatbotx_business::channel_api::{ChannelHttp, Download};
use chatbotx_business::connection::credentials::{UpstreamReply, UpstreamRequest};
use chatbotx_core::encryption::{MEDIA_TOKEN_TTL_MS, MediaTokenClaims, MediaTokenKind};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use serde_json::{Value, json};

type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const WS: i64 = 301;
const INBOX: i64 = 311;
const CONTACT: i64 = 321;
const CONTACT_INBOX: i64 = 331;
const PLACEHOLDER: &str = "http://app/media/default-avatar.svg";
const PROFILE_PIC: &str = "https://cdn.example/profile-pic.jpg";

/// `queue.add` spy.
#[derive(Default)]
struct Jobs {
    calls: Mutex<Vec<(String, Value, JobOptions)>>,
}

impl Enqueuer for Jobs {
    fn add(
        &self,
        _queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> Fut<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.calls
            .lock()
            .unwrap()
            .push((name.to_owned(), data, opts));
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> Fut<'_, Result<Vec<AddResult>, JobsError>> {
        let out = jobs
            .into_iter()
            .map(|job| AddResult {
                job_id: job.opts.job_id.unwrap_or_default(),
                created: true,
            })
            .collect();
        Box::pin(async move { Ok(out) })
    }
}

/// The Messenger User Profile answer (`None`: a Graph failure).
struct Graph(Option<Value>);

impl ChannelHttp for Graph {
    fn send(&self, _request: UpstreamRequest) -> Fut<'_, UpstreamReply> {
        let reply = match &self.0 {
            Some(body) => Ok((200, body.to_string())),
            None => Err("graph timeout".to_owned()),
        };
        Box::pin(async move { reply })
    }

    fn download(
        &self,
        _url: String,
        _bearer: Option<String>,
    ) -> Fut<'_, Result<Option<Download>, String>> {
        Box::pin(async { Ok(None) })
    }
}

async fn state(avatar: Option<&'static str>, with_integration: bool) -> (ApiState, Arc<Jobs>) {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(move |tx| {
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@avatar.test')"#, [])?;
        tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'ws', 1)"#, [WS])?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Page', 'messenger', 'page-1')"#,
            params![INBOX, WS],
        )?;
        if with_integration {
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                       "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (1, '{"tokens":{"accessToken":"page-token"}}', 'page-1', 'Page', '[]', '[]', '[]', ?1, ?2)"#,
                params![WS, INBOX],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId", "avatar") VALUES (?1, ?2, ?3)"#,
            params![CONTACT, WS, avatar],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
               VALUES (?1, ?2, ?2, ?3, 'messenger', 'direct', 'psid-1')"#,
            params![CONTACT_INBOX, CONTACT, INBOX],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let jobs = Arc::new(Jobs::default());
    let mut state = ApiState::new(Arc::new(db), "http://app");
    state.jobs = Some(jobs.clone() as Arc<dyn Enqueuer>);
    (state, jobs)
}

fn token(kind: MediaTokenKind, ref_id: i64) -> String {
    let claims = MediaTokenClaims {
        workspace_id: WS.to_string(),
        kind,
        ref_id: ref_id.to_string(),
        message_created_at: None,
    };
    chatbotx_business::meta_catalog::process_encryption()
        .sign_media_token(&claims, MEDIA_TOKEN_TTL_MS)
        .expect("sign")
}

fn location(response: &Response) -> Option<String> {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// The mirror jobs, after the fire-and-forget enqueue ran.
async fn mirrors(jobs: &Jobs) -> Vec<(String, Value, JobOptions)> {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    jobs.calls.lock().unwrap().clone()
}

async fn get(state: &ApiState, graph: &Graph) -> Response {
    serve(
        state,
        graph,
        &token(MediaTokenKind::Avatar, CONTACT_INBOX),
        &HeaderMap::new(),
    )
    .await
}

fn profile() -> Graph {
    Graph(Some(json!({ "id": "psid-1", "profile_pic": PROFILE_PIC })))
}

#[tokio::test]
async fn returns_404_for_an_invalid_token_or_an_attachment_token() {
    let (state, jobs) = state(None, true).await;
    let bad = serve(&state, &profile(), "bad", &HeaderMap::new()).await;
    assert_eq!(bad.status(), StatusCode::NOT_FOUND);
    let attachment = token(MediaTokenKind::Attachment, CONTACT_INBOX);
    let wrong = serve(&state, &profile(), &attachment, &HeaderMap::new()).await;
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);
    assert!(mirrors(&jobs).await.is_empty());
}

#[tokio::test]
async fn returns_410_for_a_non_servable_workspace() {
    let (state, _) = state(None, true).await;
    state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(get(&state, &profile()).await.status(), StatusCode::GONE);
}

#[tokio::test]
async fn rate_limits_replay_by_token() {
    let (state, _) = state(Some("workspace/avatar.jpg"), true).await;
    let replayed = token(MediaTokenKind::Avatar, CONTACT_INBOX);
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "203.0.113.10".parse().unwrap());
    for _ in 0..20 {
        let ok = serve(&state, &profile(), &replayed, &headers).await;
        assert_eq!(ok.status(), StatusCode::FOUND);
    }
    let limited = serve(&state, &profile(), &replayed, &headers).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().get(header::RETRY_AFTER).is_some());
}

#[tokio::test]
async fn redirects_a_mirrored_avatar_to_its_public_url() {
    let (state, jobs) = state(Some("workspace/workspace-1/avatar.jpg"), true).await;
    let response = get(&state, &Graph(None)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let url = location(&response).unwrap();
    assert!(url.ends_with("/workspace/workspace-1/avatar.jpg"), "{url}");
    assert!(mirrors(&jobs).await.is_empty());
}

#[tokio::test]
async fn redirects_a_fresh_no_avatar_sentinel_to_the_default_placeholder() {
    let sentinel: &'static str = Box::leak(
        chatbotx_core::utils::no_avatar_sentinel::build_no_avatar_sentinel(
            chatbotx_db::repo::now_ms(),
        )
        .into_boxed_str(),
    );
    let (state, jobs) = state(Some(sentinel), true).await;
    let response = get(&state, &profile()).await;
    assert_eq!(location(&response).as_deref(), Some(PLACEHOLDER));
    assert!(mirrors(&jobs).await.is_empty());
}

#[tokio::test]
async fn refetches_a_stale_no_avatar_sentinel_and_enqueues_mirroring() {
    let (state, jobs) = state(Some("public/img/no_avatar.jpg?time=0"), true).await;
    let response = get(&state, &profile()).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(location(&response).as_deref(), Some(PROFILE_PIC));
    assert_eq!(mirrors(&jobs).await.len(), 1);
}

#[tokio::test]
async fn redirects_a_pending_avatar_to_profile_pic_and_enqueues_mirroring() {
    let (state, jobs) = state(None, true).await;
    let response = get(&state, &profile()).await;
    assert_eq!(location(&response).as_deref(), Some(PROFILE_PIC));
    let calls = mirrors(&jobs).await;
    assert_eq!(calls.len(), 1);
    let (name, data, opts) = &calls[0];
    assert_eq!(name, "updateContactAvatar");
    assert_eq!(
        data,
        &json!({ "type": "updateContactAvatar", "data": {
            "workspaceId": WS.to_string(),
            "contactInboxId": CONTACT_INBOX.to_string(),
            "sourceId": "psid-1",
        }})
    );
    assert_eq!(opts.job_id, Some(format!("update-avatar-{CONTACT_INBOX}")));
    assert_eq!(opts.attempts, Some(2));
}

#[tokio::test]
async fn redirects_an_unsupported_avatar_and_enqueues_sentinel_persistence() {
    let (state, jobs) = state(Some("public/img/no_avatar.jpg?time=0"), true).await;
    let response = get(&state, &Graph(Some(json!({ "id": "psid-1" })))).await;
    assert_eq!(location(&response).as_deref(), Some(PLACEHOLDER));
    assert_eq!(mirrors(&jobs).await.len(), 1);
}

#[tokio::test]
async fn redirects_terminal_avatar_failures_to_the_default_placeholder() {
    // No integration: the fetch can never succeed.
    let (state, jobs) = state(None, false).await;
    let response = get(&state, &profile()).await;
    assert_eq!(location(&response).as_deref(), Some(PLACEHOLDER));
    assert_eq!(mirrors(&jobs).await.len(), 1);
}

#[tokio::test]
async fn degrades_a_transient_avatar_failure_to_the_placeholder_and_enqueues_mirroring() {
    let (state, jobs) = state(None, true).await;
    let response = get(&state, &Graph(None)).await;
    assert_eq!(location(&response).as_deref(), Some(PLACEHOLDER));
    assert_eq!(mirrors(&jobs).await.len(), 1);
}

#[tokio::test]
async fn redirects_a_missing_contact_inbox_to_the_placeholder() {
    let (state, jobs) = state(None, true).await;
    let missing = token(MediaTokenKind::Avatar, 999);
    let response = serve(&state, &profile(), &missing, &HeaderMap::new()).await;
    assert_eq!(location(&response).as_deref(), Some(PLACEHOLDER));
    assert!(mirrors(&jobs).await.is_empty());
}
