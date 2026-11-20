//! Ports of `apps/builder/__tests__/refresh-all-channel-tokens-action.test.ts`
//! and `apps/builder/__tests__/refresh-contact-profile-action.test.ts` at the
//! `/rpc` boundary (session, membership, contact scope, result shape) over a
//! migrated database. The provider branches need a scripted channel API and
//! are covered in `crates/business/tests/channel_token_refresh_action.rs` and
//! `contact_channel_profile.rs`; here every case stays offline.
//! ponytail: `logMessengerWelcomeProfile` is not ported (DECISIONS.md).

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::{RpcState, rpc_router_with};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "rpc-u3-secret";
const NOW_MS: i64 = 1_800_000_000_000;
const ADMIN: i64 = 101;
const AGENT: i64 = 102;
const WS: i64 = 201;
const OTHER_WS: i64 = 202;
const CONTACT: i64 = 401;

fn now_ms() -> i64 {
    NOW_MS
}

async fn router() -> (axum::Router, Arc<Db>) {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (id, email, token) in [(ADMIN, "admin@u3.test", "admintoken"), (AGENT, "agent@u3.test", "agenttoken")] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (?1, ?2, 'Ann', 1)"#,
                params![id, email],
            )?;
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?1, ?3)"#,
                params![id, token, NOW_MS + 60_000],
            )?;
        }
        for ws in [WS, OTHER_WS] {
            tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'ws', ?2)"#, params![ws, ADMIN])?;
        }
        for (id, user, role, permissions) in [
            (501, ADMIN, "owner", r#"{"superAdmin":true}"#),
            (502, AGENT, "agent", r#"{"onlyAssignedContacts":true}"#),
        ] {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![id, WS, user, role, permissions],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES
                 (11, ?1, 'Page', 'messenger', 'page-1'), (12, ?1, 'WA', 'whatsapp', 'wa-1')"#,
            [WS],
        )?;
        tx.execute(r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (?1, ?2, 'Ada'), (402, ?2, 'Bob')"#, params![CONTACT, WS])?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES
                 (41, ?1, ?1, 11, 'messenger', 'direct', 'psid-1'),
                 (42, ?1, ?1, 12, 'whatsapp', 'direct', 'wa-user'),
                 (43, 402, 402, 11, 'messenger', 'direct', 'psid-2')"#,
            [CONTACT],
        )?;
        // A TikTok channel without a refresh token: listed, skipped offline.
        tx.execute(r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (13, ?1, 'TT', 'tiktok', 'tt-1')"#, [WS])?;
        tx.execute(
            r#"INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId")
               VALUES (1, '{"authType":"oauth2","tokens":{"accessToken":"a"}}', 'open-1', 'TT', ?1, 13)"#,
            [WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let db = Arc::new(db);
    let auth = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let api = Arc::new(ApiState::new(Arc::clone(&db), "http://app"));
    let router = rpc_router_with(RpcState {
        auth,
        api: Some(api),
        realtime_secret: None,
    });
    (router, db)
}

async fn post(
    router: &axum::Router,
    path: &str,
    token: Option<&str>,
    input: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/rpc/{path}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(token, SECRET)
            ),
        );
    }
    let request = request
        .body(Body::from(json!({ "json": input }).to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body["json"].clone())
}

fn refresh_profile(contact: i64, contact_inbox: i64) -> Value {
    json!({
        "workspaceId": WS.to_string(),
        "args": [WS.to_string(), contact.to_string(), { "contactInboxId": contact_inbox.to_string() }],
    })
}

// ---- refreshAllChannelTokensAction ---------------------------------------------

#[tokio::test]
async fn refresh_all_channel_tokens_returns_the_summary_of_the_member_workspaces() {
    let (router, db) = router().await;
    let (status, json) = post(
        &router,
        "actions/refreshAllChannelTokensAction",
        Some("admintoken"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json, json!({ "refreshed": 0, "failed": 0 }));
    // The skipped row is untouched (no refresh token, no provider call).
    let error: Option<String> = db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "tokenRefreshError" FROM "IntegrationTiktok" WHERE "id" = 1"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(error, None);
}

#[tokio::test]
async fn refresh_all_channel_tokens_requires_a_session() {
    let (router, _) = router().await;
    let (status, _) = post(
        &router,
        "actions/refreshAllChannelTokensAction",
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---- refreshContactProfileAction ------------------------------------------------

#[tokio::test]
async fn a_contact_outside_the_access_scope_rejects_before_any_channel_work() {
    let (router, _) = router().await;
    let (status, json) = post(
        &router,
        "actions/refreshContactProfileAction",
        Some("agenttoken"),
        refresh_profile(CONTACT, 41),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
    assert_eq!(json["message"], "Contact not found");
}

#[tokio::test]
async fn a_contact_inbox_not_belonging_to_the_contact_throws_not_found() {
    let (router, _) = router().await;
    let (status, json) = post(
        &router,
        "actions/refreshContactProfileAction",
        Some("admintoken"),
        refresh_profile(CONTACT, 43),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
    assert_eq!(json["message"], "Contact inbox not found");
}

#[tokio::test]
async fn a_channel_without_on_demand_refresh_forwards_the_channel_not_capable_skip() {
    let (router, _) = router().await;
    let (status, json) = post(
        &router,
        "actions/refreshContactProfileAction",
        Some("admintoken"),
        refresh_profile(CONTACT, 42),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        json,
        json!({ "status": "skipped", "reason": "channelNotCapable" })
    );
}

#[tokio::test]
async fn a_disconnected_integration_returns_failed_never_thrown() {
    let (router, _) = router().await;
    let (status, json) = post(
        &router,
        "actions/refreshContactProfileAction",
        Some("admintoken"),
        refresh_profile(CONTACT, 41),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json, json!({ "status": "failed" }));
}
