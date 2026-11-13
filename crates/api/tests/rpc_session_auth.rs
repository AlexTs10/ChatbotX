//! `/rpc/*` session auth (`authMiddleware` + `workspaceAuthorizedMidddleware`
//! of `apps/builder/src/middlewares/auth.ts`). No TypeScript test covers these
//! middlewares directly (the builder tests mock them); these tests pin the
//! behaviour of the original against a migrated in-memory database.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::rpc_router;
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "rpc-test-secret";
const NOW_MS: i64 = 1_800_000_000_000;
const USER_ID: i64 = 101;
const FLAGGED_USER_ID: i64 = 102;
const WORKSPACE_ID: i64 = 201;
const DELETING_WORKSPACE_ID: i64 = 202;
const FOREIGN_WORKSPACE_ID: i64 = 203;

fn now_ms() -> i64 {
    NOW_MS
}

async fn state() -> Arc<AuthState> {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (id, email, flagged) in [
            (USER_ID, "member@rpc.test", false),
            (FLAGGED_USER_ID, "flagged@rpc.test", true),
            (103, "owner@rpc.test", false),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "emailVerified", "mustChangePassword")
                   VALUES (?1, ?2, 1, ?3)"#,
                params![id, email, flagged],
            )?;
        }
        for (id, token, user_id, expires_at) in [
            (301, "membertoken", USER_ID, NOW_MS + 60_000),
            (302, "expiredtoken", USER_ID, NOW_MS - 1),
            (303, "flaggedtoken", FLAGGED_USER_ID, NOW_MS + 60_000),
        ] {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, token, user_id, expires_at],
            )?;
        }
        for (id, scheduled) in [
            (WORKSPACE_ID, None),
            (DELETING_WORKSPACE_ID, Some(NOW_MS)),
            (FOREIGN_WORKSPACE_ID, None),
        ] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "scheduledDeletionAt")
                   VALUES (?1, 'ws', 103, ?2)"#,
                params![id, scheduled],
            )?;
        }
        for (id, workspace_id) in [(401, WORKSPACE_ID), (402, DELETING_WORKSPACE_ID)] {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role")
                   VALUES (?1, ?2, ?3, 'agent')"#,
                params![id, workspace_id, USER_ID],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed");
    Arc::new(AuthState {
        db: Arc::new(db),
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
    })
}

fn cookie(token: &str) -> String {
    format!(
        "better-auth.session_token={}",
        sign_cookie_value(token, SECRET)
    )
}

async fn call(
    state: Arc<AuthState>,
    method: &str,
    path: &str,
    token: Option<&str>,
    input: Value,
) -> (StatusCode, String) {
    let payload = json!({ "json": input, "meta": [] }).to_string();
    let mut builder = Request::builder().method(method);
    builder = if method == "GET" {
        let data: String = payload.bytes().map(|byte| format!("%{byte:02X}")).collect();
        builder.uri(format!("{path}?data={data}"))
    } else {
        builder
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
    };
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, cookie(token));
    }
    let body = if method == "GET" {
        Body::empty()
    } else {
        Body::from(payload)
    };
    let response = rpc_router(state)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn rpc_error(code: &str, status: u16, message: &str) -> Value {
    json!({
        "json": { "defined": false, "code": code, "status": status, "message": message },
        "meta": [[3, "data"]],
    })
}

fn parse(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| panic!("not JSON: {body}"))
}

#[tokio::test]
async fn an_unknown_router_key_answers_the_handler_not_found() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/noSuchAPI/list",
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, "Not found");
}

#[tokio::test]
async fn a_call_without_a_session_cookie_is_unauthorized() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/contactsAPIs/list",
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(parse(&body), rpc_error("UNAUTHORIZED", 401, "Unauthorized"));
}

#[tokio::test]
async fn an_expired_or_unknown_session_is_unauthorized() {
    for token in ["expiredtoken", "unknowntoken"] {
        let (status, _) = call(
            state().await,
            "POST",
            "/rpc/contactsAPIs/list",
            Some(token),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token}");
    }
}

#[tokio::test]
async fn a_forged_cookie_signature_is_unauthorized() {
    let response = rpc_router(state().await)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rpc/contactsAPIs/list")
                .header(
                    header::COOKIE,
                    "better-auth.session_token=membertoken.forged",
                )
                .body(Body::from(r#"{"json":{},"meta":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_user_who_must_change_the_password_is_forbidden() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/contactsAPIs/list",
        Some("flaggedtoken"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        parse(&body),
        rpc_error("FORBIDDEN", 403, "Password change required")
    );
}

#[tokio::test]
async fn a_valid_session_passes_auth_to_the_not_ported_procedure() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/workspacesAPI/list",
        Some("membertoken"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        parse(&body),
        rpc_error("NOT_FOUND", 404, "Operation not ported yet")
    );
}

#[tokio::test]
async fn a_member_passes_the_workspace_check() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/contactsAPIs/list",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn a_get_call_reads_the_input_from_the_data_query() {
    let (status, _) = call(
        state().await,
        "GET",
        "/rpc/contactsAPIs/list",
        Some("membertoken"),
        json!({ "workspaceId": FOREIGN_WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_non_member_or_unknown_workspace_is_unauthorized() {
    for workspace_id in [FOREIGN_WORKSPACE_ID.to_string(), "999999".to_string()] {
        let (status, body) = call(
            state().await,
            "POST",
            "/rpc/contactsAPIs/list",
            Some("membertoken"),
            json!({ "workspaceId": workspace_id }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(parse(&body), rpc_error("UNAUTHORIZED", 401, "Unauthorized"));
    }
}

#[tokio::test]
async fn a_workspace_scheduled_for_deletion_is_forbidden() {
    let (status, body) = call(
        state().await,
        "POST",
        "/rpc/contactsAPIs/list",
        Some("membertoken"),
        json!({ "workspaceId": DELETING_WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        parse(&body),
        rpc_error("FORBIDDEN", 403, "Workspace deletion scheduled")
    );
}
