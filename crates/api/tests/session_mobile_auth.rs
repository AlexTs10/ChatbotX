//! Port of `packages/auth/__tests__/mobile-auth.test.ts` (`mobileAuth()` +
//! the better-auth `jwt()` plugin). The TS test mocks the refresh-token
//! service with an in-memory map; here the real `"MobileRefreshToken"` rows
//! are checked instead (same observable results).

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{
    AuthSettings, AuthState, BearerVerifyError, TrustedOriginSettings, VerifyBearerOptions,
    auth_router, hash_password, mobile_change_password, verify_jwt,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt as _;

const ISSUER: &str = "https://app.example.com";
const AUDIENCE: &str = "https://app.example.com/api";
const PASSWORD: &str = "correct-horse-battery";
const SECRET: &str = "a-very-long-test-secret-for-better-auth-0123456789";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn verifier() -> String {
    "a".repeat(64)
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn state() -> Arc<AuthState> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    Arc::new(AuthState {
        db: Arc::new(db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: ISSUER.to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                builder_url: ISSUER.to_string(),
                ..TrustedOriginSettings::default()
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    })
}

/// `seedUser`: user 7 `agent@example.com` with a credential account.
async fn seed_user(state: &AuthState, email_verified: bool, must_change_password: bool) -> i64 {
    let hash = hash_password(PASSWORD);
    state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "User" ("id", "name", "email", "emailVerified", "mustChangePassword")
                   VALUES (7, 'Agent', 'agent@example.com', ?1, ?2)"#,
                params![email_verified, must_change_password],
            )?;
            tx.execute(
                r#"INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId")
                   VALUES (70, '7', 'credential', ?1, 7)"#,
                [hash],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    7
}

async fn send(state: &Arc<AuthState>, request: Request<Body>) -> Response {
    auth_router(Arc::clone(state))
        .oneshot(request)
        .await
        .unwrap()
}

async fn post(state: &Arc<AuthState>, path: &str, body: Value) -> Response {
    send(
        state,
        Request::builder()
            .method("POST")
            .uri(format!("/api/auth{path}"))
            .header(header::HOST, "app.example.com")
            .header("origin", ISSUER)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

/// `tokenGrant`: adds the default PKCE verifier to an `exchange_code` grant.
async fn token_grant(state: &Arc<AuthState>, mut body: Value) -> Response {
    if body["grantType"] == "exchange_code" && body.get("codeVerifier").is_none() {
        body["codeVerifier"] = json!(verifier());
    }
    post(state, "/mobile/token", body).await
}

async fn json_body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// `verifyAccessToken`: signature, issuer, audience and expiry against the stored JWKS.
async fn verify_access_token(
    state: &AuthState,
    token: &str,
    audience: &str,
) -> Result<Value, BearerVerifyError> {
    verify_jwt(
        &state.db,
        token,
        &VerifyBearerOptions {
            jwks_url: format!("{ISSUER}/api/auth/jwks"),
            issuer: ISSUER.to_string(),
            audience: audience.to_string(),
        },
        now_ms(),
    )
    .await
}

/// `sessionCookie`: signs in by email and returns the cookie.
async fn session_cookie(state: &Arc<AuthState>) -> String {
    let response = post(
        state,
        "/sign-in/email",
        json!({ "email": "agent@example.com", "password": PASSWORD }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|cookie| {
            cookie
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("; ")
}

async fn count(state: &AuthState, table: &'static str) -> i64 {
    state
        .db
        .read(move |conn| {
            Ok(
                conn.query_row(&format!(r#"SELECT COUNT(*) FROM "{table}""#), [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
        .unwrap()
}

async fn live_refresh_tokens(state: &AuthState) -> i64 {
    state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT COUNT(*) FROM "MobileRefreshToken" WHERE "revoked" IS NULL AND "rotatedAt" IS NULL"#,
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
}

async fn password_login(state: &Arc<AuthState>, device_id: Option<&str>) -> Value {
    let mut body =
        json!({ "grantType": "password", "email": "agent@example.com", "password": PASSWORD });
    if let Some(device_id) = device_id {
        body["deviceId"] = json!(device_id);
    }
    let response = token_grant(state, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

// ---------------------------------------------------------------- password grant

#[tokio::test]
async fn password_grant_returns_access_jwt_and_refresh_token_without_session() {
    let state = state().await;
    let user = seed_user(&state, true, false).await;

    let response = token_grant(
        &state,
        json!({ "grantType": "password", "email": "Agent@Example.com", "password": PASSWORD, "deviceId": "install-1" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_body(response).await;
    assert_eq!(body["tokenType"], "Bearer");
    assert_eq!(body["expiresIn"], 600);
    assert_eq!(body["deviceId"], "install-1");
    assert_eq!(body["mustChangePassword"], false);
    // Was `refreshToken: "rt-1"` (mocked service); the real token is `cbx_mrt_` + 32 random bytes.
    assert!(
        body["refreshToken"]
            .as_str()
            .unwrap()
            .starts_with("cbx_mrt_")
    );
    let payload = verify_access_token(&state, body["accessToken"].as_str().unwrap(), AUDIENCE)
        .await
        .unwrap();
    assert_eq!(payload["sub"], user.to_string());
    assert_eq!(payload["iss"], ISSUER);
    assert_eq!(payload["aud"], AUDIENCE);
    assert_eq!(payload["deviceId"], "install-1");
    assert_eq!(payload["mustChangePassword"], false);
    assert_eq!(
        payload["exp"].as_i64().unwrap() - payload["iat"].as_i64().unwrap(),
        600
    );
    assert_eq!(
        body["expiresAt"].as_i64().unwrap(),
        payload["exp"].as_i64().unwrap() * 1000
    );
    assert_eq!(count(&state, "Session").await, 0);
    // `issue` called with { userId, deviceId }.
    let (user_id, device): (i64, String) = state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "userId", "deviceId" FROM "MobileRefreshToken""#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!((user_id, device.as_str()), (user, "install-1"));
}

#[tokio::test]
async fn password_grant_carries_must_change_password() {
    let state = state().await;
    seed_user(&state, true, true).await;

    let body = password_login(&state, None).await;

    assert_eq!(body["mustChangePassword"], true);
    let payload = verify_access_token(&state, body["accessToken"].as_str().unwrap(), AUDIENCE)
        .await
        .unwrap();
    assert_eq!(payload["mustChangePassword"], true);
}

#[tokio::test]
async fn access_token_is_not_valid_for_the_mcp_audience() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let body = password_login(&state, None).await;

    let result = verify_access_token(
        &state,
        body["accessToken"].as_str().unwrap(),
        "https://app.example.com/mcp",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn wrong_password_or_unknown_email_is_401_and_issues_nothing() {
    for (email, password) in [
        ("agent@example.com", "wrong-password-1"),
        ("nobody@example.com", PASSWORD),
    ] {
        let state = state().await;
        seed_user(&state, true, false).await;

        let response = token_grant(
            &state,
            json!({ "grantType": "password", "email": email, "password": password }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            json_body(response).await["code"],
            "INVALID_EMAIL_OR_PASSWORD"
        );
        assert_eq!(count(&state, "MobileRefreshToken").await, 0);
        assert_eq!(count(&state, "Session").await, 0);
    }
}

#[tokio::test]
async fn unverified_email_is_403() {
    let state = state().await;
    seed_user(&state, false, false).await;

    let response = token_grant(
        &state,
        json!({ "grantType": "password", "email": "agent@example.com", "password": PASSWORD }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(response).await["code"], "EMAIL_NOT_VERIFIED");
    assert_eq!(count(&state, "MobileRefreshToken").await, 0);
}

#[tokio::test]
async fn rejects_unknown_grant_type_and_malformed_device_id() {
    let state = state().await;
    seed_user(&state, true, false).await;

    let unknown_grant = token_grant(&state, json!({ "grantType": "client_credentials" })).await;
    let bad_device = token_grant(
        &state,
        json!({ "grantType": "password", "email": "agent@example.com", "password": PASSWORD, "deviceId": "has spaces!" }),
    )
    .await;

    assert_eq!(unknown_grant.status(), StatusCode::BAD_REQUEST);
    assert_eq!(bad_device.status(), StatusCode::BAD_REQUEST);
}

// Not ported: "is throttled like sign-in: 3 attempts per 10 seconds per IP" checks the
// better-auth `rateLimit` plugin rule; the Rust auth router has no rate limiter yet
// (also not for sign-in), see DECISIONS P3.5f slice 4.

// ---------------------------------------------------------------- social flow

async fn callback(
    state: &Arc<AuthState>,
    query: &[(&str, &str)],
    cookie: Option<&str>,
) -> Response {
    let default_challenge = challenge(&verifier());
    let mut pairs: Vec<(&str, &str)> = query.to_vec();
    if !pairs.iter().any(|(key, _)| *key == "code_challenge") {
        pairs.push(("code_challenge", &default_challenge));
    }
    callback_raw(state, &pairs, cookie).await
}

async fn callback_raw(
    state: &Arc<AuthState>,
    query: &[(&str, &str)],
    cookie: Option<&str>,
) -> Response {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let mut request = Request::builder()
        .uri(format!("/api/auth/mobile/callback?{query}"))
        .header(header::HOST, "app.example.com");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    send(state, request.body(Body::empty()).unwrap()).await
}

fn location(response: &Response) -> Option<url::Url> {
    response
        .headers()
        .get(header::LOCATION)
        .map(|value| url::Url::parse(value.to_str().unwrap()).unwrap())
}

fn param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn code_from(response: &Response) -> String {
    param(&location(response).unwrap(), "code").unwrap()
}

async fn mint_code(state: &Arc<AuthState>, extra: &[(&str, &str)]) -> String {
    let cookie = session_cookie(state).await;
    let mut query = vec![("redirect_uri", "chatbotxmobileapp://auth-callback")];
    query.extend_from_slice(extra);
    code_from(&callback(state, &query, Some(&cookie)).await)
}

#[tokio::test]
async fn callback_mints_one_time_code_redirects_and_drops_the_session() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let cookie = session_cookie(&state).await;
    assert_eq!(count(&state, "Session").await, 1);

    let response = callback(
        &state,
        &[
            ("redirect_uri", "chatbotxmobileapp://auth-callback"),
            ("device_id", "install-9"),
            ("state", "xyz"),
        ],
        Some(&cookie),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FOUND);
    let location = location(&response).unwrap();
    assert_eq!(
        format!("{}://{}", location.scheme(), location.host_str().unwrap()),
        "chatbotxmobileapp://auth-callback"
    );
    assert_eq!(param(&location, "state").as_deref(), Some("xyz"));
    let code = param(&location, "code").unwrap();
    assert_eq!(count(&state, "Session").await, 0);
    // Only a hash of the code is stored, never the code itself.
    let stored: String = state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT group_concat("identifier" || "value") FROM "Verification""#,
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert!(!stored.contains(&code));
}

#[tokio::test]
async fn code_exchanges_once_bound_to_the_callback_device() {
    let state = state().await;
    let user = seed_user(&state, true, false).await;
    let code = mint_code(&state, &[("device_id", "install-9")]).await;

    let first = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": code }),
    )
    .await;
    let second = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": code }),
    )
    .await;

    assert_eq!(first.status(), StatusCode::OK);
    let body = json_body(first).await;
    assert_eq!(body["deviceId"], "install-9");
    let payload = verify_access_token(&state, body["accessToken"].as_str().unwrap(), AUDIENCE)
        .await
        .unwrap();
    assert_eq!(payload["sub"], user.to_string());
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(second).await["code"], "INVALID_EXCHANGE_CODE");
}

#[tokio::test]
async fn concurrently_exchanged_code_is_consumed_atomically() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let code = mint_code(&state, &[]).await;

    let (a, b) = tokio::join!(
        token_grant(
            &state,
            json!({ "grantType": "exchange_code", "code": code })
        ),
        token_grant(
            &state,
            json!({ "grantType": "exchange_code", "code": code })
        ),
    );

    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 401]);
}

#[tokio::test]
async fn requires_pkce_challenge_and_verifier() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let cookie = session_cookie(&state).await;
    let missing_challenge = callback_raw(
        &state,
        &[("redirect_uri", "chatbotxmobileapp://auth-callback")],
        Some(&cookie),
    )
    .await;
    assert_eq!(missing_challenge.status(), StatusCode::BAD_REQUEST);
    assert_eq!(count(&state, "Verification").await, 0);

    let code = mint_code(&state, &[]).await;
    let missing_verifier = post(
        &state,
        "/mobile/token",
        json!({ "grantType": "exchange_code", "code": code }),
    )
    .await;
    assert_eq!(missing_verifier.status(), StatusCode::BAD_REQUEST);
    let ok = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": code }),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn expired_code_is_rejected() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let code = mint_code(&state, &[]).await;
    state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Verification" SET "expiresAt" = ?1"#,
                [now_ms() - 1],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let response = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": code }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_verifier_burns_the_code() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let verifier = verifier();
    let challenge = challenge(&verifier);
    let with_challenge = [("code_challenge", challenge.as_str())];

    let missing_verifier_code = mint_code(&state, &with_challenge).await;
    let without_verifier = post(
        &state,
        "/mobile/token",
        json!({ "grantType": "exchange_code", "code": missing_verifier_code }),
    )
    .await;
    let retry_after_missing = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": missing_verifier_code, "codeVerifier": verifier }),
    )
    .await;
    let stolen = mint_code(&state, &with_challenge).await;
    let wrong_verifier = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": stolen, "codeVerifier": "b".repeat(64) }),
    )
    .await;
    let retry_after_wrong = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": stolen, "codeVerifier": verifier }),
    )
    .await;
    let fresh = mint_code(&state, &with_challenge).await;
    let ok = token_grant(
        &state,
        json!({ "grantType": "exchange_code", "code": fresh, "codeVerifier": verifier }),
    )
    .await;

    assert_eq!(without_verifier.status(), StatusCode::BAD_REQUEST);
    assert_eq!(retry_after_missing.status(), StatusCode::OK);
    assert_eq!(wrong_verifier.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(retry_after_wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn refuses_bad_redirect_uris_without_redirecting() {
    for redirect_uri in [
        "https://evil.example.com/steal",
        "otherapp://auth-callback",
        "javascript:alert(1)",
    ] {
        let state = state().await;
        seed_user(&state, true, false).await;
        let cookie = session_cookie(&state).await;

        let response = callback(&state, &[("redirect_uri", redirect_uri)], Some(&cookie)).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{redirect_uri}");
        assert!(response.headers().get(header::LOCATION).is_none());
        assert_eq!(count(&state, "Verification").await, 0);
    }
}

#[tokio::test]
async fn without_session_redirects_with_access_denied() {
    let state = state().await;

    let response = callback(
        &state,
        &[
            ("redirect_uri", "chatbotxmobileapp://auth-callback"),
            ("state", "s1"),
        ],
        None,
    )
    .await;

    let location = location(&response).unwrap();
    assert_eq!(param(&location, "error").as_deref(), Some("access_denied"));
    assert_eq!(param(&location, "state").as_deref(), Some("s1"));
    assert_eq!(param(&location, "code"), None);
    assert_eq!(count(&state, "Verification").await, 0);
}

// ---------------------------------------------------------------- refresh / revoke

#[tokio::test]
async fn refresh_rotates_for_the_same_device() {
    let state = state().await;
    let user = seed_user(&state, true, false).await;
    let login = password_login(&state, Some("install-1")).await;

    let response = token_grant(
        &state,
        json!({ "grantType": "refresh_token", "refreshToken": login["refreshToken"] }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_ne!(body["refreshToken"], login["refreshToken"]);
    assert_eq!(body["deviceId"], "install-1");
    let payload = verify_access_token(&state, body["accessToken"].as_str().unwrap(), AUDIENCE)
        .await
        .unwrap();
    assert_eq!(payload["sub"], user.to_string());
}

#[tokio::test]
async fn reusing_a_spent_refresh_token_is_reused() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let login = password_login(&state, None).await;
    let refresh = json!({ "grantType": "refresh_token", "refreshToken": login["refreshToken"] });
    token_grant(&state, refresh.clone()).await;
    // The real service answers a replay inside the 30 s rotation grace as INVALID (a
    // concurrent refresh), so the replay is made after the grace window.
    state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "MobileRefreshToken" SET "rotatedAt" = "rotatedAt" - 31000 WHERE "rotatedAt" IS NOT NULL"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let replay = token_grant(&state, refresh).await;

    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(replay).await["code"], "REFRESH_TOKEN_REUSED");
    // Reuse revokes the whole family (`business` service).
    assert_eq!(live_refresh_tokens(&state).await, 0);
}

#[tokio::test]
async fn unknown_refresh_token_is_invalid() {
    let state = state().await;

    let response = token_grant(
        &state,
        json!({ "grantType": "refresh_token", "refreshToken": "nope" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(response).await["code"], "INVALID_REFRESH_TOKEN");
}

#[tokio::test]
async fn revoke_revokes_the_family_and_always_answers_success() {
    let state = state().await;
    let response = post(
        &state,
        "/mobile/revoke",
        json!({ "refreshToken": "whatever" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await, json!({ "success": true }));

    // `revokeByToken` effect on a real token.
    seed_user(&state, true, false).await;
    let login = password_login(&state, None).await;
    assert_eq!(live_refresh_tokens(&state).await, 1);
    let response = post(
        &state,
        "/mobile/revoke",
        json!({ "refreshToken": login["refreshToken"] }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(live_refresh_tokens(&state).await, 0);
}

// ---------------------------------------------------------------- password change

#[tokio::test]
async fn mobile_change_password_is_not_reachable_over_http() {
    let state = state().await;
    let response = post(
        &state,
        "/mobile/change-password",
        json!({ "userId": "1", "currentPassword": PASSWORD, "newPassword": "another-long-password", "deviceId": "install-1" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mobile_change_password_ends_every_login_and_returns_fresh_tokens() {
    let state = state().await;
    let user = seed_user(&state, true, true).await;
    session_cookie(&state).await;
    assert_eq!(count(&state, "Session").await, 1);
    password_login(&state, Some("old-device")).await;

    let result =
        mobile_change_password(&state, user, PASSWORD, "another-long-password", "install-1")
            .await
            .unwrap();

    assert_eq!(result["tokenType"], "Bearer");
    assert_eq!(result["deviceId"], "install-1");
    assert_eq!(result["mustChangePassword"], false);
    // `revokeAllForUser`: only the freshly issued token is live.
    assert_eq!(live_refresh_tokens(&state).await, 1);
    // `clearMustChangePassword`.
    let flag: bool = state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "mustChangePassword" FROM "User" WHERE "id" = 7"#,
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert!(!flag);
    assert_eq!(count(&state, "Session").await, 0);
    // The old password no longer works; the new one does.
    let old_login = token_grant(
        &state,
        json!({ "grantType": "password", "email": "agent@example.com", "password": PASSWORD }),
    )
    .await;
    let new_login = token_grant(
        &state,
        json!({ "grantType": "password", "email": "agent@example.com", "password": "another-long-password" }),
    )
    .await;
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(new_login.status(), StatusCode::OK);
}

#[tokio::test]
async fn mobile_change_password_rejections_revoke_nothing() {
    for (current, next) in [
        ("not-the-password", "another-long-password"),
        (PASSWORD, PASSWORD),
        (PASSWORD, "short"),
    ] {
        let state = state().await;
        let user = seed_user(&state, true, false).await;
        password_login(&state, None).await;

        let error = mobile_change_password(&state, user, current, next, "install-1")
            .await
            .unwrap_err();

        assert_eq!(error.status, 400, "{current} -> {next}");
        assert_eq!(live_refresh_tokens(&state).await, 1);
    }
}

async fn web_change_password(state: &Arc<AuthState>, current: &str) -> Response {
    let cookie = session_cookie(state).await;
    send(
        state,
        Request::builder()
            .method("POST")
            .uri("/api/auth/change-password")
            .header(header::HOST, "app.example.com")
            .header("origin", ISSUER)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, cookie)
            .body(Body::from(
                json!({ "currentPassword": current, "newPassword": "web-side-new-password" })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn web_change_password_revokes_every_mobile_login() {
    let state = state().await;
    seed_user(&state, true, false).await;
    password_login(&state, None).await;

    let response = web_change_password(&state, PASSWORD).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(live_refresh_tokens(&state).await, 0);
}

#[tokio::test]
async fn failed_web_change_password_revokes_nothing() {
    let state = state().await;
    seed_user(&state, true, false).await;
    password_login(&state, None).await;

    let response = web_change_password(&state, "wrong-current-password").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(live_refresh_tokens(&state).await, 1);
}

#[tokio::test]
async fn jwks_lists_the_signing_key() {
    let state = state().await;
    seed_user(&state, true, false).await;
    let login = password_login(&state, None).await;

    let response = send(
        &state,
        Request::builder()
            .uri("/api/auth/jwks")
            .header(header::HOST, "app.example.com")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let key = &body["keys"][0];
    assert_eq!(key["alg"], "EdDSA");
    assert_eq!(key["crv"], "Ed25519");
    assert_eq!(key["kty"], "OKP");
    assert!(key.get("d").is_none());
    let header_segment = login["accessToken"]
        .as_str()
        .unwrap()
        .split('.')
        .next()
        .unwrap();
    let header: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header_segment).unwrap()).unwrap();
    assert_eq!(header, json!({ "alg": "EdDSA", "kid": key["kid"] }));
}
