//! `apps/builder/__tests__/media-proxy-route.test.ts` (Q.P6-U1), the
//! attachment proxy half: token checks, 410, the mirrored / failed /
//! pending answers, the mirror and restore jobs, the placeholders on the
//! forwarded origin and the rate limit. The Graph re-derivation cases
//! (`resolveFreshMediaUrl`) are not ported (see the module docs of
//! `media_proxy`); the avatar route is not ported.

mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chatbotx_business::media_url::{AttachmentRef, attachment_proxy_url};
use chatbotx_core::encryption::{MEDIA_TOKEN_TTL_MS, MediaTokenClaims, MediaTokenKind};
use chatbotx_files::{LocalUrlSigner, Uploader};
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use handlers_common::{Fixture, WS, fixture};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const AT: i64 = 1_758_326_400_000;

#[derive(Default)]
struct Jobs {
    calls: Mutex<Vec<(QueueName, String, Value, JobOptions)>>,
}

impl Enqueuer for Jobs {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        self.calls
            .lock()
            .expect("lock")
            .push((queue, name.to_owned(), data, opts));
        Box::pin(async {
            Ok(AddResult {
                job_id: String::new(),
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _queue: QueueName,
        _jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

async fn exec(f: &Fixture, sql: String) {
    f.state
        .db
        .write(move |tx| {
            tx.execute_batch(&sql)?;
            Ok(())
        })
        .await
        .expect("seed");
}

/// A messenger message 900 with attachment 1 at `origin_path`.
async fn setup(origin_path: &str) -> (Fixture, Arc<Jobs>) {
    let mut f = fixture().await;
    exec(
        &f,
        format!(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (6, {WS}, 'Mess', 'messenger', 'm-6');
               INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (10, {WS}, 'F');
               INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                 VALUES (10, 10, 10, 6, 'messenger', 'direct', 'src-10');
               INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (10, {WS}, 10);
               INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
                 "messageType", "contentType", "senderType")
                 VALUES (900, {AT}, {AT}, 10, 10, {WS}, 'hi', 'incoming', 'text', 'contact');
               INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath")
                 VALUES (1, {WS}, 10, 900, {AT}, 'image', 'image/jpeg', '{origin_path}');"#
        ),
    )
    .await;
    let jobs = Arc::new(Jobs::default());
    f.state.jobs = Some(jobs.clone() as Arc<dyn Enqueuer>);
    let signer = LocalUrlSigner::new("secret", "https://files.test").expect("signer");
    f.state.files = Some(Arc::new(Uploader::in_memory().with_url_signer(signer)));
    (f, jobs)
}

fn token(attachment_id: i64) -> String {
    let url = attachment_proxy_url(
        "http://app",
        &AttachmentRef {
            workspace_id: WS,
            attachment_id,
            origin_path: "wa-media:1",
            channel: "messenger",
            message_created_at: Some(AT),
        },
        MEDIA_TOKEN_TTL_MS,
    )
    .expect("token");
    url.rsplit('/').next().expect("token part").to_owned()
}

async fn get(
    f: &Fixture,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, Option<String>, Option<String>) {
    let app = chatbotx_api::media_proxy::router(Arc::new(f.state.clone()));
    let mut request = Request::builder().uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let header_of = |name: header::HeaderName| {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
    };
    (
        response.status(),
        header_of(header::LOCATION),
        header_of(header::RETRY_AFTER),
    )
}

fn calls(jobs: &Jobs) -> Vec<(QueueName, String, Value, JobOptions)> {
    jobs.calls.lock().expect("lock").clone()
}

#[tokio::test]
async fn returns_404_for_an_invalid_token() {
    let (f, _) = setup("public/a.jpg").await;
    let (status, ..) = get(&f, "/media/attachment/not-a-token", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn returns_404_when_an_avatar_token_is_used_on_the_attachment_route() {
    let (f, _) = setup("public/a.jpg").await;
    let claims = MediaTokenClaims {
        workspace_id: WS.to_string(),
        kind: MediaTokenKind::Avatar,
        ref_id: "10".into(),
        message_created_at: None,
    };
    let avatar =
        chatbotx_business::media_url::sign_media_token(&claims, MEDIA_TOKEN_TTL_MS).expect("sign");
    let (status, ..) = get(&f, &format!("/media/attachment/{avatar}"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn returns_410_for_a_non_servable_workspace() {
    let (f, _) = setup("public/a.jpg").await;
    exec(
        &f,
        format!(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = {WS};"#),
    )
    .await;
    let (status, ..) = get(&f, &format!("/media/attachment/{}", token(1)), &[]).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn returns_404_for_an_unknown_attachment() {
    let (f, _) = setup("public/a.jpg").await;
    let (status, ..) = get(&f, &format!("/media/attachment/{}", token(2)), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn redirects_mirrored_media_to_a_fresh_presigned_url_without_graph() {
    let (f, jobs) = setup("workspace/workspace-1/media.jpg").await;
    let (status, location, _) = get(&f, &format!("/media/attachment/{}", token(1)), &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    let location = location.expect("location");
    assert!(location.starts_with("https://files.test/"), "{location}");
    assert!(
        location.contains("workspace/workspace-1/media.jpg"),
        "{location}"
    );
    assert!(calls(&jobs).is_empty());
}

#[tokio::test]
async fn retry_on_mirrored_media_enqueues_a_restore_job() {
    let (f, jobs) = setup("workspace/workspace-1/media.jpg").await;
    let (status, location, _) =
        get(&f, &format!("/media/attachment/{}?retry=1", token(1)), &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        location.as_deref(),
        Some("http://localhost:3123/media/unavailable.svg")
    );
    let calls = calls(&jobs);
    assert_eq!(calls.len(), 1);
    let (queue, name, data, opts) = &calls[0];
    assert_eq!(*queue, QueueName::Low);
    assert_eq!(name, "coexistAttachmentDownload");
    assert_eq!(
        data["data"],
        json!({ "attachmentId": "1", "workspaceId": WS.to_string(), "channel": "messenger", "messageCreatedAt": AT, "restore": true })
    );
    assert_eq!(
        opts.dedup.as_ref().map(|d| d.id.as_str()),
        Some("media-restore-1")
    );
    assert_eq!(opts.attempts, Some(3));
    assert_eq!(opts.priority, Some(10));
}

#[tokio::test]
async fn redirects_terminal_media_to_the_unavailable_placeholder() {
    let (f, jobs) = setup("failed:unresolvable").await;
    let (status, location, _) = get(&f, &format!("/media/attachment/{}", token(1)), &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        location.as_deref(),
        Some("http://localhost:3123/media/unavailable.svg")
    );
    assert!(calls(&jobs).is_empty());
}

#[tokio::test]
async fn redirects_pending_media_to_its_cdn_url_and_enqueues_one_message_job() {
    let (f, jobs) = setup("https://lookaside.fbsbx.com/media.jpg").await;
    let (status, location, _) = get(&f, &format!("/media/attachment/{}", token(1)), &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        location.as_deref(),
        Some("https://lookaside.fbsbx.com/media.jpg")
    );
    let calls = calls(&jobs);
    assert_eq!(calls.len(), 1);
    let (_, _, data, opts) = &calls[0];
    // The parent message time lets the sharded lookup find the row.
    assert_eq!(data["data"]["messageCreatedAt"], json!(AT));
    assert_eq!(opts.job_id.as_deref(), Some("media-message-900"));
    assert_eq!(opts.attempts, Some(5));
}

#[tokio::test]
async fn retry_on_a_pending_attachment_does_not_use_the_stored_webhook_url() {
    let (f, jobs) = setup("https://lookaside.fbsbx.com/media.jpg").await;
    let (_, location, _) = get(&f, &format!("/media/attachment/{}?retry=1", token(1)), &[]).await;
    assert_eq!(
        location.as_deref(),
        Some("http://localhost:3123/media/processing.svg")
    );
    assert_eq!(calls(&jobs).len(), 1);
}

#[tokio::test]
async fn redirects_unresolved_pending_media_to_the_processing_placeholder() {
    let (f, jobs) = setup("wa-media:media-1").await;
    let (status, location, _) = get(&f, &format!("/media/attachment/{}", token(1)), &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        location.as_deref(),
        Some("http://localhost:3123/media/processing.svg")
    );
    assert_eq!(calls(&jobs).len(), 1);
}

#[tokio::test]
async fn builds_placeholders_from_the_forwarded_public_origin() {
    let (f, _) = setup("wa-media:media-1").await;
    let (_, location, _) = get(
        &f,
        &format!("/media/attachment/{}", token(1)),
        &[
            ("x-forwarded-host", "chat.example.com"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_eq!(
        location.as_deref(),
        Some("https://chat.example.com/media/processing.svg")
    );
}

#[tokio::test]
async fn rate_limits_replay_by_token_and_ip() {
    let (f, _) = setup("public/a.jpg").await;
    let path = format!("/media/attachment/{}", token(1));
    let ip = [("x-forwarded-for", "203.0.113.10")];
    for _ in 0..20 {
        let (status, ..) = get(&f, &path, &ip).await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    }
    let (status, _, retry_after) = get(&f, &path, &ip).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let retry_after: i64 = retry_after.expect("retry-after").parse().unwrap();
    assert!((1..=10).contains(&retry_after), "{retry_after}");
}
