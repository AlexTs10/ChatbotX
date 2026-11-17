//! Q.A1: `GET /api/invitations/{code}/start` (port of
//! `apps/builder/__tests__/invitation-start-route.test.ts`; the tenant case
//! is the root tenant, see `invitation::lookup_active`).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{
    AuthSettings, AuthState, START_LIMIT_PER_IP, TrustedOriginSettings, invitation_start_router,
};
use chatbotx_db::Db;
use chatbotx_kv::{Kv, KvConfig, TestClock};
use tower::ServiceExt as _;

const ORIGIN: &str = "https://chat.acme.com";

async fn app() -> Router {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.migrate().await.unwrap();
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email") VALUES (7, 'U', 'u7@example.com')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2001, 'Acme', 7)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Invitation" ("id", "code", "permissions", "expiresAt", "workspaceId", "invitedBy")
               VALUES (903, 'invite-code', '{}', 4000000000000, 2001, 7)"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
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
        // 3 s into a 10 s window: `Retry-After` 7.
        now_ms: || 1_800_000_003_000,
    });
    let kv = Kv::start(
        db,
        Arc::new(TestClock::new(1_800_000_003_000)),
        KvConfig::default(),
    );
    invitation_start_router(state, kv)
}

async fn get(app: &Router, code: &str) -> Response {
    app.clone()
        .oneshot(
            Request::get(format!("/api/invitations/{code}/start"))
                .header(header::HOST, "chat.acme.com")
                .header("x-forwarded-for", "203.0.113.1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn header_of<'a>(response: &'a Response, name: header::HeaderName) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn valid_code_redirects_to_sign_up_with_a_short_lived_http_only_cookie() {
    let app = app().await;
    let response = get(&app, "invite-code").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header_of(&response, header::LOCATION),
        Some("https://chat.acme.com/auth/sign-up?callbackURL=%2Finvitations%2Finvite-code")
    );
    let cookie = header_of(&response, header::SET_COOKIE).unwrap();
    for part in [
        "chatbotx_invitation=invite-code",
        "HttpOnly",
        "SameSite=lax",
        "Max-Age=600",
        "Path=/",
        "Secure",
    ] {
        assert!(cookie.contains(part), "{cookie} lacks {part}");
    }
}

#[tokio::test]
async fn unknown_code_redirects_to_sign_in_and_sets_no_cookie() {
    let app = app().await;
    let response = get(&app, "nope").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header_of(&response, header::LOCATION),
        Some("https://chat.acme.com/auth/sign-in?error=invalid_invitation")
    );
    assert!(header_of(&response, header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn a_flood_from_one_ip_gets_429_with_retry_after() {
    let app = app().await;
    for _ in 0..START_LIMIT_PER_IP {
        assert_eq!(get(&app, "nope").await.status(), StatusCode::FOUND);
    }
    let response = get(&app, "invite-code").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(header_of(&response, header::RETRY_AFTER), Some("7"));
    assert!(header_of(&response, header::SET_COOKIE).is_none());
}
