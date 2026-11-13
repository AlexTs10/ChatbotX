//! Magic link and password reset endpoints against a migrated in-memory
//! database (Rust-native: the TypeScript suite tests better-auth's endpoints
//! only through the original stack). Also pins NFKC password normalization.

use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use chatbotx_api::mail::{Mailer, SendFuture};
use chatbotx_api::session::{
    AuthSettings, AuthState, TrustedOriginSettings, auth_router, hash_password, verify_password,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

#[derive(Default)]
struct Outbox(Mutex<Vec<(String, String)>>);

impl Mailer for Outbox {
    fn send<'a>(&'a self, to: &'a str, _subject: &'a str, text: &'a str) -> SendFuture<'a> {
        self.0
            .lock()
            .unwrap()
            .push((to.to_string(), text.to_string()));
        Box::pin(async { Ok(()) })
    }
}

fn now_ms() -> i64 {
    1_800_000_000_000
}

async fn state(outbox: Arc<Outbox>) -> Arc<AuthState> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    let hash = hash_password("Old#password1");
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email", "emailVerified") VALUES (7, 'known@app.test', 1)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId")
               VALUES (8, '7', 'credential', ?1, 7)"#,
            params![hash],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    Arc::new(AuthState {
        db: Arc::new(db),
        settings: AuthSettings {
            secret: "secret".to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                builder_url: "http://app".to_string(),
                ..TrustedOriginSettings::default()
            },
            extra_trusted_origins: None,
        },
        mailer: outbox,
        now_ms,
    })
}

async fn send(state: &Arc<AuthState>, method: &str, uri: &str, body: Option<Value>) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "app")
        .header("origin", "http://app");
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    auth_router(Arc::clone(state))
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn mailed_link(outbox: &Outbox, marker: &str) -> String {
    let mails = outbox.0.lock().unwrap();
    let (_, text) = mails.last().expect("a mail");
    let start = text.find(marker).expect("link in mail");
    text[start..].split_whitespace().next().unwrap().to_string()
}

fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_magic_link_signs_in_a_known_user_once() {
    let outbox = Arc::new(Outbox::default());
    let state = state(Arc::clone(&outbox)).await;

    let response = send(
        &state,
        "POST",
        "/api/auth/sign-in/magic-link",
        Some(json!({ "email": "known@app.test", "callbackURL": "/dashboard" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await, json!({ "status": true }));

    let link = mailed_link(&outbox, "http://app/api/auth/magic-link/verify?");
    let path = link.trim_start_matches("http://app");
    let response = send(&state, "GET", path, None).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(location(&response), "http://app/dashboard");
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("better-auth.session_token=")
    );

    let reused = send(&state, "GET", path, None).await;
    assert_eq!(
        location(&reused),
        "http://app/dashboard?error=INVALID_TOKEN"
    );
}

#[tokio::test]
async fn a_magic_link_for_an_unknown_email_is_refused() {
    let outbox = Arc::new(Outbox::default());
    let state = state(Arc::clone(&outbox)).await;

    let response = send(
        &state,
        "POST",
        "/api/auth/sign-in/magic-link",
        Some(json!({ "email": "nobody@app.test" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["message"],
        "Your email is not registered with ChatbotX"
    );
    assert!(outbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_untrusted_callback_url_is_forbidden() {
    let state = state(Arc::new(Outbox::default())).await;
    let response = send(
        &state,
        "POST",
        "/api/auth/sign-in/magic-link",
        Some(json!({ "email": "known@app.test", "callbackURL": "https://evil.test/" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(response).await["code"], "INVALID_CALLBACK_URL");
}

#[tokio::test]
async fn a_password_reset_replaces_the_password_and_spends_the_token() {
    let outbox = Arc::new(Outbox::default());
    let state = state(Arc::clone(&outbox)).await;

    let response = send(
        &state,
        "POST",
        "/api/auth/request-password-reset",
        Some(json!({ "email": "known@app.test", "redirectTo": "/auth/reset" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["status"], true);

    let link = mailed_link(&outbox, "http://app/api/auth/reset-password/");
    let response = send(&state, "GET", link.trim_start_matches("http://app"), None).await;
    let target = location(&response);
    let token = target.split("token=").nth(1).expect("token").to_string();
    assert!(
        target.starts_with("http://app/auth/reset?token="),
        "{target}"
    );

    let short = send(
        &state,
        "POST",
        "/api/auth/reset-password",
        Some(json!({ "token": token, "newPassword": "short" })),
    )
    .await;
    assert_eq!(json_body(short).await["code"], "PASSWORD_TOO_SHORT");

    let response = send(
        &state,
        "POST",
        "/api/auth/reset-password",
        Some(json!({ "token": token, "newPassword": "New#password1" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let again = send(
        &state,
        "POST",
        "/api/auth/reset-password",
        Some(json!({ "token": token, "newPassword": "New#password2" })),
    )
    .await;
    assert_eq!(again.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(again).await["code"], "INVALID_TOKEN");

    let sign_in = send(
        &state,
        "POST",
        "/api/auth/sign-in/email",
        Some(json!({ "email": "known@app.test", "password": "New#password1" })),
    )
    .await;
    assert_eq!(sign_in.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_email_gets_the_same_reset_answer_and_no_mail() {
    let outbox = Arc::new(Outbox::default());
    let state = state(Arc::clone(&outbox)).await;
    let response = send(
        &state,
        "POST",
        "/api/auth/request-password-reset",
        Some(json!({ "email": "nobody@app.test" })),
    )
    .await;
    assert_eq!(
        json_body(response).await,
        json!({
            "status": true,
            "message": "If this email exists in our system, check your email for the reset link",
        })
    );
    assert!(outbox.0.lock().unwrap().is_empty());
}

#[test]
fn passwords_are_nfkc_normalized_before_hashing() {
    // U+FB01 (LATIN SMALL LIGATURE FI) is NFKC-equivalent to "fi".
    let hash = hash_password("\u{FB01}nance#2026");
    assert!(verify_password(&hash, "finance#2026"));
}
