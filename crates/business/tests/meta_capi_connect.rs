//! Q.P6-U2: the dedicated Conversions API connect (`startMetaCapiOAuth`,
//! `completeMetaCapiConnect`). Ports `apps/builder/__tests__/
//! start-meta-capi-oauth.test.ts` and `meta-capi-oauth-callback.test.ts`
//! over a migrated database and a scripted Graph API. The original state
//! (base64 JSON + a nonce cookie) is a signed connect session here: a
//! malformed state cannot reach the store, and the nonce check is the
//! session binding (another user = `notFound`, a used session =
//! `sessionExpired`). The audit record is written by the API callback.

mod common;

use std::sync::Mutex;

use chatbotx_business::ads::connect::{
    Caller, CapiOutcome, Flow, complete_meta_capi, flow_of, start,
};
use chatbotx_business::connect_session;
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::meta_catalog::process_encryption;
use chatbotx_core::encryption::{EncryptedData, Encryption, Keys};
use chatbotx_db::repo::partials::credential::platform_credential_aad;
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};

struct Script {
    replies: Mutex<Vec<(u16, Value)>>,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Script {
    fn with(replies: Vec<(u16, Value)>) -> Script {
        Script {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }
    fn urls(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.url.clone())
            .collect()
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().expect("lock").push(request);
        match self.replies.lock().expect("lock").pop() {
            Some((status, body)) => Ok((status, body.to_string())),
            None => Err("no scripted reply".into()),
        }
    }
}

const APP: &str = "https://app.test";
const USER: i64 = 1001;
const MESSENGER: i64 = 7;
const INSTAGRAM: i64 = 9;

fn encryption() -> Encryption {
    Encryption::new(Keys::new(&"a1".repeat(32)).expect("test key"))
}

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    let mut credentials = Vec::new();
    for (id, kind) in [(5001, "messenger"), (5003, "instagramFacebook")] {
        let config = json!({ "clientId": format!("{kind}-app"), "clientSecret": "secret", "version": "v25.0" });
        let blob = encryption()
            .encrypt_object(&config, Some(&platform_credential_aad(kind)))
            .expect("encrypt");
        credentials.push((id, kind, serde_json::to_string(&blob).expect("json")));
    }
    setup(&db, move |tx| {
        for (id, kind, value) in &credentials {
            tx.execute(
                r#"INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value") VALUES (?1, NULL, ?2, '{}', ?3)"#,
                params![id, kind, value],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (71, ?1, 'Page', 'messenger', 'page-1'), (91, ?1, 'IG', 'instagram', 'ig-1'), (92, ?1, 'IG2', 'instagram', 'ig-2')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
               VALUES (?1, '{}', 'page-1', 'Shop', '[]', '[]', '[]', ?2, 71)"#,
            params![MESSENGER, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "conversationStarters", "persistentMenus", "workspaceId", "inboxId", "type")
               VALUES (?1, '{}', 'ig-1', 'page-9', 'IG', 'ig', '[]', '[]', ?2, 91, 'facebook'),
                      (10, '{}', 'ig-2', 'page-10', 'IG2', 'ig2', '[]', '[]', ?2, 92, 'instagram')"#,
            params![INSTAGRAM, WS],
        )?;
        Ok(())
    })
    .await;
    db
}

fn flow(channel: &str, integration_id: i64) -> Flow {
    Flow::MetaCapi {
        channel: channel.into(),
        integration_id,
    }
}

async fn begin(
    db: &common::TestDb,
    channel: &str,
    id: i64,
) -> (String, connect_session::SessionRow) {
    let segment = if channel == "instagram" {
        "instagrams"
    } else {
        "messengers"
    };
    let url = start(
        db,
        &encryption(),
        APP,
        WS,
        USER,
        flow(channel, id),
        format!("/space/{WS}/{segment}/{id}/ads"),
    )
    .await
    .expect("start");
    let state = url::Url::parse(&url)
        .expect("url")
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("state")
        .1
        .into_owned();
    let (_, nonce) = state.split_once('.').expect("signed state");
    let session = connect_session::find_by_nonce(db, nonce)
        .await
        .expect("lookup")
        .expect("session");
    (url, session)
}

fn admin() -> Caller {
    Caller {
        user_id: Some(USER),
        is_member: true,
        super_admin: true,
    }
}

async fn stored(
    db: &common::TestDb,
    table: &'static str,
    id: i64,
) -> (Option<String>, Option<String>) {
    db.read(move |conn| {
        Ok(conn.query_row(
            &format!(r#"SELECT "datasetId", "capiAccessToken" FROM "{table}" WHERE "id" = ?1"#),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })
    .await
    .expect("row")
}

fn token_of(raw: &str) -> String {
    let blob: EncryptedData = serde_json::from_str(raw).expect("blob");
    let value: Value = process_encryption()
        .decrypt_object(&blob, None)
        .expect("decrypt");
    value["accessToken"].as_str().expect("token").to_string()
}

fn messenger_graph(long_user: (u16, Value)) -> Vec<(u16, Value)> {
    vec![
        (200, json!({ "access_token": "short-user-token" })),
        long_user,
        (
            200,
            json!({ "data": [
                { "id": "other-page", "access_token": "other-page-token" },
                { "id": "page-1", "access_token": "page-token" },
            ] }),
        ),
        (200, json!({ "access_token": "long:page-token" })),
        (200, json!({ "id": "dataset-1" })),
    ]
}

/// "sends a Messenger admin to the CAPI-only dialog and returns to the Ads tab".
#[tokio::test]
async fn start_sends_a_messenger_admin_to_the_capi_only_dialog() {
    let db = seeded().await;
    let (url, session) = begin(&db, "messenger", MESSENGER).await;
    assert!(
        url.starts_with("https://www.facebook.com/v25.0/dialog/oauth?"),
        "{url}"
    );
    let q: std::collections::HashMap<_, _> = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(q["scope"], "page_events,pages_show_list");
    assert_eq!(q["auth_type"], "rerequest");
    assert_eq!(q["client_id"], "messenger-app");
    assert_eq!(
        q["redirect_uri"],
        "https://app.test/integrations/messenger/callback"
    );
    assert_eq!(session.provider, "messenger");
    assert_eq!(
        session.return_url.as_deref(),
        Some("/space/2001/messengers/7/ads")
    );
    assert_eq!(flow_of(&session), Some(flow("messenger", MESSENGER)));
}

/// "uses the Instagram Facebook-login app and callback for Instagram".
#[tokio::test]
async fn start_uses_the_instagram_facebook_login_app_for_instagram() {
    let db = seeded().await;
    let (url, session) = begin(&db, "instagram", INSTAGRAM).await;
    let q: std::collections::HashMap<_, _> = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(q["client_id"], "instagramFacebook-app");
    assert_eq!(
        q["scope"],
        "instagram_manage_events,instagram_basic,pages_show_list"
    );
    assert_eq!(
        q["redirect_uri"],
        "https://app.test/integrations/instagram-facebook/callback"
    );
    assert_eq!(session.provider, "instagramFacebook");
    assert_eq!(flow_of(&session), Some(flow("instagram", INSTAGRAM)));
}

/// "explains when the platform app is not configured".
#[tokio::test]
async fn start_explains_when_the_platform_app_is_not_configured() {
    let db = two_workspaces().await;
    let error = start(
        &db,
        &encryption(),
        APP,
        WS,
        USER,
        flow("messenger", MESSENGER),
        "/x".into(),
    )
    .await
    .expect_err("no app");
    assert_eq!(
        error.message(),
        "The Meta app for this channel is not configured yet."
    );
}

/// "stores the long-lived page token of the connected page, never the user token".
#[tokio::test]
async fn messenger_stores_the_long_lived_page_token_of_the_connected_page() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let graph = Script::with(messenger_graph((
        200,
        json!({ "access_token": "long:short-user-token" }),
    )));
    let outcome = complete_meta_capi(
        &db,
        &encryption(),
        APP,
        &graph,
        &session,
        admin(),
        "auth-code",
    )
    .await
    .expect("complete");
    assert_eq!(outcome, CapiOutcome::Success);
    let urls = graph.urls();
    assert!(urls[0].contains("code=auth-code"));
    assert!(
        urls[2].contains("/v25.0/me/accounts")
            && urls[2].contains("access_token=long%3Ashort-user-token")
    );
    assert!(urls[3].contains("fb_exchange_token=page-token"));
    assert!(urls[4].ends_with("/page-1/dataset"));
    let (dataset, token) = stored(&db, "IntegrationMessenger", MESSENGER).await;
    assert_eq!(dataset.as_deref(), Some("dataset-1"));
    assert_eq!(token_of(&token.expect("token")), "long:page-token");
    let after = connect_session::find_for_workspace(&db, session.id, WS)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, "completed");
}

/// "falls back to the short-lived user token when the long-lived exchange fails".
#[tokio::test]
async fn messenger_falls_back_to_the_short_lived_user_token() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let graph = Script::with(messenger_graph((
        500,
        json!({ "error": { "message": "graph down" } }),
    )));
    let outcome = complete_meta_capi(
        &db,
        &encryption(),
        APP,
        &graph,
        &session,
        admin(),
        "auth-code",
    )
    .await
    .expect("complete");
    assert_eq!(outcome, CapiOutcome::Success);
    assert!(graph.urls()[2].contains("access_token=short-user-token"));
}

/// "reports pageNotFound when the user did not grant the connected page".
#[tokio::test]
async fn messenger_reports_page_not_found_when_the_page_was_not_granted() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let graph = Script::with(vec![
        (200, json!({ "access_token": "short" })),
        (200, json!({ "access_token": "long" })),
        (
            200,
            json!({ "data": [{ "id": "other-page", "access_token": "t" }] }),
        ),
    ]);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("pageNotFound"));
    assert_eq!(
        stored(&db, "IntegrationMessenger", MESSENGER).await,
        (None, None)
    );
}

/// "reports datasetFailed, and stores nothing, when Meta refuses the dataset".
#[tokio::test]
async fn messenger_reports_dataset_failed_and_stores_nothing() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let mut replies = messenger_graph((200, json!({ "access_token": "long" })));
    replies[4] = (
        400,
        json!({ "error": { "message": "(#200) Permissions error", "code": 200 } }),
    );
    let graph = Script::with(replies);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("datasetFailed"));
    assert_eq!(
        stored(&db, "IntegrationMessenger", MESSENGER).await,
        (None, None)
    );
}

/// "reports a generic failure when the code exchange throws".
#[tokio::test]
async fn messenger_reports_a_generic_failure_when_the_code_exchange_fails() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let graph = Script::with(vec![(400, json!({ "error": { "message": "bad code" } }))]);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("failed"));
    assert_eq!(graph.urls().len(), 1);
}

/// The request binding: not a super admin / another user = `notFound`, a
/// used session = `sessionExpired`, an integration not in the workspace =
/// `notFound`; none of them calls Meta.
#[tokio::test]
async fn the_request_binding_refuses_before_any_provider_call() {
    let db = seeded().await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let graph = Script::with(Vec::new());
    for caller in [
        Caller {
            super_admin: false,
            ..admin()
        },
        Caller {
            user_id: Some(9999),
            ..admin()
        },
        Caller {
            is_member: false,
            ..admin()
        },
    ] {
        let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, caller, "c")
            .await
            .expect("complete");
        assert_eq!(outcome, CapiOutcome::NotFound);
    }
    let mut used = session.clone();
    used.status = "completed".into();
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &used, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("sessionExpired"));

    let (_, other) = begin(&db, "messenger", 4242).await;
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &other, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("notFound"));
    assert!(graph.urls().is_empty());
}

/// "stores the page token of the connected Instagram account" and the
/// account / Business Login refusals.
#[tokio::test]
async fn instagram_stores_the_page_token_of_the_connected_account() {
    let db = seeded().await;
    let (_, session) = begin(&db, "instagram", INSTAGRAM).await;
    let graph = Script::with(vec![
        (200, json!({ "access_token": "short" })),
        (200, json!({ "access_token": "long-user-token" })),
        (
            200,
            json!({ "access_token": "ig-page-token", "instagram_business_account": { "id": "ig-1" } }),
        ),
        (200, json!({ "id": "dataset-9" })),
    ]);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Success);
    let urls = graph.urls();
    assert!(urls[2].contains("/v25.0/page-9?") && urls[2].contains("access_token=long-user-token"));
    assert!(urls[3].ends_with("/ig-1/dataset"));
    let (dataset, token) = stored(&db, "IntegrationInstagram", INSTAGRAM).await;
    assert_eq!(dataset.as_deref(), Some("dataset-9"));
    assert_eq!(token_of(&token.expect("token")), "ig-page-token");

    // Another Instagram account on the page.
    let (_, session) = begin(&db, "instagram", INSTAGRAM).await;
    let graph = Script::with(vec![
        (200, json!({ "access_token": "short" })),
        (200, json!({ "access_token": "long" })),
        (
            200,
            json!({ "access_token": "t", "instagram_business_account": { "id": "ig-other" } }),
        ),
    ]);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("accountNotFound"));

    // "refuses an Instagram Business Login integration".
    let (_, session) = begin(&db, "instagram", 10).await;
    let graph = Script::with(Vec::new());
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("notFound"));
    assert!(graph.urls().is_empty());
}

async fn with_stored_dataset(db: &common::TestDb) {
    setup(db, |tx| {
        tx.execute(
            r#"UPDATE "IntegrationMessenger" SET "datasetId" = 'old-ds' WHERE "id" = ?1"#,
            [MESSENGER],
        )?;
        Ok(())
    })
    .await;
}

/// Q.P7-V1 (`findReadableStoredDataset`): a stored dataset the new token
/// cannot read (Graph access error) is replaced by the dataset linked at Meta.
#[tokio::test]
async fn messenger_replaces_a_stored_dataset_the_token_cannot_read() {
    let db = seeded().await;
    with_stored_dataset(&db).await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let mut replies = messenger_graph((200, json!({ "access_token": "long" })));
    replies.insert(
        4,
        (
            400,
            json!({ "error": { "message": "Unsupported get request", "code": 100 } }),
        ),
    );
    let graph = Script::with(replies);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Success);
    let urls = graph.urls();
    assert!(urls[4].contains("/old-ds?fields=id"));
    assert!(urls[5].ends_with("/page-1/dataset"));
    let (dataset, _) = stored(&db, "IntegrationMessenger", MESSENGER).await;
    assert_eq!(dataset.as_deref(), Some("dataset-1"));
}

/// Q.P7-V1: a transient error reading the stored dataset (rate limit) is
/// surfaced; the stored dataset and token are kept.
#[tokio::test]
async fn messenger_keeps_the_stored_dataset_on_a_transient_read_error() {
    let db = seeded().await;
    with_stored_dataset(&db).await;
    let (_, session) = begin(&db, "messenger", MESSENGER).await;
    let mut replies = messenger_graph((200, json!({ "access_token": "long" })));
    replies[4] = (
        400,
        json!({ "error": { "message": "Too many calls", "code": 4 } }),
    );
    let graph = Script::with(replies);
    let outcome = complete_meta_capi(&db, &encryption(), APP, &graph, &session, admin(), "c")
        .await
        .expect("complete");
    assert_eq!(outcome, CapiOutcome::Error("datasetFailed"));
    assert_eq!(graph.urls().len(), 5);
    assert_eq!(
        stored(&db, "IntegrationMessenger", MESSENGER).await,
        (Some("old-ds".into()), None)
    );
}
