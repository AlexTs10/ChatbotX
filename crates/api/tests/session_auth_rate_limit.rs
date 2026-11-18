//! Q.A1: the `/api/auth/*` rate limits (better-auth `rateLimit` special rule,
//! `mobileAuth` `/mobile/token` rule) and the builder's password-grant
//! account limit (port of `apps/builder/__tests__/mobile-password-grant-rate-limit.test.ts`,
//! main cases). The clock is fixed inside one 10 s window.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{
    AuthRateLimit, AuthSettings, AuthState, TrustedOriginSettings, auth_rate_limit, auth_router,
};
use chatbotx_db::Db;
use chatbotx_kv::{Kv, KvConfig, TestClock};
use serde_json::json;
use tower::ServiceExt as _;

const ORIGIN: &str = "https://app.example.com";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

async fn app() -> Router {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.migrate().await.unwrap();
    let state = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: "a-very-long-test-secret-for-better-auth-0123456789".into(),
            base_url: ORIGIN.into(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                builder_url: ORIGIN.into(),
                ..TrustedOriginSettings::default()
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let kv = Kv::start(
        db,
        Arc::new(TestClock::new(1_800_000_000_000)),
        KvConfig::default(),
    );
    auth_router(state).layer(axum::middleware::from_fn_with_state(
        AuthRateLimit {
            kv,
            now_ms: || 1_800_000_000_000,
        },
        auth_rate_limit,
    ))
}

async fn post(app: &Router, path: &str, ip: &str, body: serde_json::Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::HOST, "app.example.com")
                .header("origin", ORIGIN)
                .header("x-forwarded-for", ip)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn sign_in_allows_three_requests_per_ip_and_window() {
    let app = app().await;
    let body = json!({ "email": "x@example.com", "password": "wrong-password" });
    for _ in 0..3 {
        assert_ne!(
            post(
                &app,
                "/api/auth/sign-in/email",
                "198.51.100.1",
                body.clone()
            )
            .await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    assert_eq!(
        post(
            &app,
            "/api/auth/sign-in/email",
            "198.51.100.1",
            body.clone()
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another client IP has its own counter.
    assert_ne!(
        post(&app, "/api/auth/sign-in/email", "198.51.100.2", body).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn password_grant_is_limited_per_account_across_ips() {
    let app = app().await;
    let body = json!({ "grantType": "password", "email": "Agent@Example.com", "password": "x", "codeChallenge": "c" });
    for n in 0..5 {
        assert_ne!(
            post(
                &app,
                "/api/auth/mobile/token",
                &format!("203.0.113.{n}"),
                body.clone()
            )
            .await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    let lower = json!({ "grantType": "password", "email": "agent@example.com", "password": "x" });
    assert_eq!(
        post(&app, "/api/auth/mobile/token", "203.0.113.99", lower).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Refresh grants carry no password and skip the account limit.
    assert_ne!(
        post(
            &app,
            "/api/auth/mobile/token",
            "203.0.113.100",
            json!({ "grantType": "refresh_token", "refreshToken": "r" })
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // An unknown grant is a 400 before better-auth.
    assert_eq!(
        post(
            &app,
            "/api/auth/mobile/token",
            "203.0.113.101",
            json!({ "grantType": "magic" })
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}
