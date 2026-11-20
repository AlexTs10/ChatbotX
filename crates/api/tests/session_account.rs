//! better-auth `/update-user` and `/change-password`, and the
//! `forceChangePasswordAction` server action as `/rpc/authActions/forceChangePassword`
//! (Rust-native: no TypeScript test covers these endpoints; the builder calls
//! them from the profile dialog and the forced password change page).

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::rpc_router;
use chatbotx_api::session::{
    AuthSettings, AuthState, TrustedOriginSettings, auth_router, hash_password, sign_cookie_value,
    verify_password,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "account-secret";
const NOW_MS: i64 = 1_800_000_000_000;

fn now_ms() -> i64 {
    NOW_MS
}

/// User 7 (password `Old#password1`, sessions `tok7a` and `tok7b`, one live
/// mobile refresh token) and user 9 (`mustChangePassword`, session `tok9`).
async fn state() -> Arc<AuthState> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    let hash = hash_password("Old#password1");
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email", "emailVerified", "mustChangePassword")
               VALUES (7, 'Seven', 'seven@app.test', 1, 0), (9, 'Nine', 'nine@app.test', 1, 1)"#,
            [],
        )?;
        for (id, user) in [(70, 7), (90, 9)] {
            tx.execute(
                r#"INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId")
                   VALUES (?1, ?2, 'credential', ?3, ?2)"#,
                params![id, user, hash],
            )?;
        }
        for (id, token, user) in [(71, "tok7a", 7), (72, "tok7b", 7), (91, "tok9", 9)] {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "expiresAt", "token", "userId")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, NOW_MS + 60_000, token, user],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "MobileRefreshToken" ("id", "tokenHash", "userId", "deviceId", "familyId", "expiresAt")
               VALUES (75, 'h', 7, 'd', 'f', ?1)"#,
            params![NOW_MS + 60_000],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    Arc::new(AuthState {
        db: Arc::new(db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                builder_url: "http://app".to_string(),
                ..TrustedOriginSettings::default()
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

async fn post(router: Router, uri: &str, token: Option<&str>, body: Value) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::HOST, "app")
        .header("origin", "http://app")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::COOKIE, cookie(token));
    }
    router
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn user_row(state: &AuthState, id: i64) -> (Option<String>, Option<String>, bool) {
    state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "name", "image", "mustChangePassword" FROM "User" WHERE "id" = ?1"#,
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn password_hash(state: &AuthState, user: i64) -> String {
    state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "password" FROM "Account" WHERE "userId" = ?1"#,
                [user],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
}

async fn session_tokens(state: &AuthState, user: i64) -> Vec<String> {
    state
        .db
        .read(move |conn| {
            let mut statement =
                conn.prepare(r#"SELECT "token" FROM "Session" WHERE "userId" = ?1 ORDER BY "id""#)?;
            let rows = statement.query_map([user], |row| row.get(0))?;
            Ok(rows.collect::<Result<Vec<String>, _>>()?)
        })
        .await
        .unwrap()
}

async fn refresh_token_revoked(state: &AuthState) -> bool {
    state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "revoked" IS NOT NULL FROM "MobileRefreshToken" WHERE "id" = 75"#,
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
}

fn set_cookie_token(response: &Response) -> Option<String> {
    let raw = response.headers().get(header::SET_COOKIE)?.to_str().ok()?;
    let value = raw
        .split(';')
        .next()?
        .strip_prefix("better-auth.session_token=")?;
    let value = value
        .replace("%2B", "+")
        .replace("%2F", "/")
        .replace("%3D", "=");
    Some(value.split('.').next()?.to_string())
}

#[tokio::test]
async fn update_user_changes_name_and_image_of_the_session_user() {
    let state = state().await;
    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/update-user",
        Some("tok7a"),
        json!({ "name": "New name", "image": "https://img.test/a.png" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await, json!({ "status": true }));
    assert_eq!(
        user_row(&state, 7).await,
        (
            Some("New name".to_string()),
            Some("https://img.test/a.png".to_string()),
            false
        )
    );

    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/update-user",
        Some("tok7a"),
        json!({ "image": null }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(user_row(&state, 7).await.1, None);
}

#[tokio::test]
async fn update_user_rejects_no_session_email_and_empty_bodies() {
    let state = state().await;
    let cases = [
        (
            None,
            json!({ "name": "x" }),
            401,
            json!({ "code": "UNAUTHORIZED", "message": "Unauthorized" }),
        ),
        (
            Some("tok7a"),
            json!({ "email": "other@app.test" }),
            400,
            json!({ "code": "EMAIL_CAN_NOT_BE_UPDATED", "message": "Email can not be updated" }),
        ),
        (
            Some("tok7a"),
            json!({}),
            400,
            json!({ "message": "No fields to update" }),
        ),
        (
            Some("tok7a"),
            json!({ "mustChangePassword": false }),
            400,
            json!({ "message": "mustChangePassword is not allowed to be set" }),
        ),
    ];
    for (token, body, status, expected) in cases {
        let response = post(
            auth_router(Arc::clone(&state)),
            "/api/auth/update-user",
            token,
            body,
        )
        .await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(json_body(response).await, expected);
    }
    assert_eq!(user_row(&state, 7).await.0, Some("Seven".to_string()));
}

#[tokio::test]
async fn get_session_returns_the_additional_user_fields() {
    let state = state().await;
    let request = Request::builder()
        .uri("/api/auth/get-session")
        .header(header::COOKIE, cookie("tok9"))
        .body(Body::empty())
        .unwrap();
    let response = auth_router(Arc::clone(&state))
        .oneshot(request)
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(body["user"]["mustChangePassword"], json!(true));
    assert_eq!(body["user"]["isAnonymous"], json!(false));
}

#[tokio::test]
async fn change_password_checks_the_current_password() {
    let state = state().await;
    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/change-password",
        Some("tok7a"),
        json!({ "currentPassword": "wrong-password", "newPassword": "New#password2" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await,
        json!({ "code": "INVALID_PASSWORD", "message": "Invalid password" })
    );

    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/change-password",
        Some("tok7a"),
        json!({ "currentPassword": "Old#password1", "newPassword": "short" }),
    )
    .await;
    assert_eq!(
        json_body(response).await,
        json!({ "code": "PASSWORD_TOO_SHORT", "message": "Password too short" })
    );
    assert!(verify_password(
        &password_hash(&state, 7).await,
        "Old#password1"
    ));
    assert!(!refresh_token_revoked(&state).await);
}

#[tokio::test]
async fn change_password_keeps_sessions_unless_revoke_other_sessions() {
    let state = state().await;
    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/change-password",
        Some("tok7a"),
        json!({ "currentPassword": "Old#password1", "newPassword": "New#password2" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let body = json_body(response).await;
    assert_eq!(body["token"], Value::Null);
    assert_eq!(body["user"]["id"], json!("7"));
    assert!(verify_password(
        &password_hash(&state, 7).await,
        "New#password2"
    ));
    assert_eq!(session_tokens(&state, 7).await, ["tok7a", "tok7b"]);
    assert!(refresh_token_revoked(&state).await);

    let response = post(
        auth_router(Arc::clone(&state)),
        "/api/auth/change-password",
        Some("tok7a"),
        json!({
            "currentPassword": "New#password2",
            "newPassword": "Third#password3",
            "revokeOtherSessions": true
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let new_token = set_cookie_token(&response).expect("a rotated session cookie");
    // The bearer plugin exposes the signed cookie value.
    assert!(
        response.headers()["set-auth-token"]
            .to_str()
            .unwrap()
            .starts_with(&format!("{new_token}."))
    );
    let body = json_body(response).await;
    assert_eq!(body["token"], json!(new_token));
    assert_eq!(session_tokens(&state, 7).await, [new_token]);
}

async fn rpc_post(state: &Arc<AuthState>, path: &str, token: &str, input: Value) -> Response {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie(token))
        .body(Body::from(json!({ "json": input }).to_string()))
        .unwrap();
    rpc_router(Arc::clone(state))
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn force_change_password_clears_the_gate_and_rotates_the_session() {
    let state = state().await;
    // Every other procedure is gated while the flag is set.
    let gated = rpc_post(&state, "/rpc/workspacesAPI/list", "tok9", json!({})).await;
    assert_eq!(gated.status(), StatusCode::FORBIDDEN);

    let response = rpc_post(
        &state,
        "/rpc/authActions/forceChangePassword",
        "tok9",
        json!({
            "currentPassword": "Old#password1",
            "newPassword": "New#password2",
            "passwordConfirmation": "New#password2"
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let new_token = set_cookie_token(&response).expect("a rotated session cookie");
    assert_eq!(json_body(response).await, json!({ "json": { "ok": true } }));
    assert!(!user_row(&state, 9).await.2);
    assert_eq!(session_tokens(&state, 9).await, [new_token]);
}

#[tokio::test]
async fn force_change_password_answers_on_the_spa_stub_path() {
    let state = state().await;
    let response = rpc_post(
        &state,
        "/rpc/actions/forceChangePasswordAction",
        "tok9",
        json!({ "args": [{
            "currentPassword": "Old#password1",
            "newPassword": "New#password2",
            "passwordConfirmation": "New#password2"
        }] }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(set_cookie_token(&response).is_some());
    assert_eq!(json_body(response).await, json!({ "json": { "ok": true } }));
    assert!(!user_row(&state, 9).await.2);
}

#[tokio::test]
async fn force_change_password_errors() {
    let state = state().await;
    let rpc_error = |code: &str, status: u16, message: &str| {
        json!({
            "json": { "defined": false, "code": code, "status": status, "message": message },
            "meta": [[3, "data"]],
        })
    };
    let cases = [
        (
            "tok9",
            json!({ "currentPassword": "Old#password1", "newPassword": "New#password2", "passwordConfirmation": "Other#password" }),
            400,
            rpc_error("BAD_REQUEST", 400, "Input validation failed"),
        ),
        (
            "tok9",
            json!({ "currentPassword": "Old#password1", "newPassword": "Old#password1", "passwordConfirmation": "Old#password1" }),
            400,
            rpc_error("BAD_REQUEST", 400, "Input validation failed"),
        ),
        (
            "tok9",
            json!({ "currentPassword": "Wrong#password", "newPassword": "New#password2", "passwordConfirmation": "New#password2" }),
            400,
            rpc_error("changePasswordFailed", 400, "Invalid password"),
        ),
        (
            "nosuchtoken",
            json!({ "currentPassword": "Old#password1", "newPassword": "New#password2", "passwordConfirmation": "New#password2" }),
            401,
            rpc_error("unauthorized", 401, "Not authenticated"),
        ),
    ];
    for (token, input, status, expected) in cases {
        let response = rpc_post(&state, "/rpc/authActions/forceChangePassword", token, input).await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(json_body(response).await, expected);
    }
    assert!(user_row(&state, 9).await.2);
}
