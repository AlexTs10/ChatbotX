//! Social / generic OAuth sign-in (`/sign-in/social`, `/sign-in/oauth2`,
//! `/callback/{id}`) against a local fake identity provider (Rust-native:
//! the TypeScript suite tests only the provider configuration).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{
    AuthSettings, AuthState, OAuthKind, OAuthProvider, OAuthState, TrustedOriginSettings,
    code_challenge, oauth_router,
};
use chatbotx_db::Db;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const NOW_MS: i64 = 1_800_000_000_000;

fn now_ms() -> i64 {
    NOW_MS
}

/// Profiles by authorization code; the token request records its form.
#[derive(Default)]
struct Idp {
    profiles: Mutex<HashMap<String, Value>>,
    token_forms: Mutex<Vec<String>>,
}

fn json_response(value: Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn unsigned_jwt(claims: &Value) -> String {
    format!(
        "{}.{}.sig",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

async fn idp_token(State(idp): State<Arc<Idp>>, body: Bytes) -> Response {
    let form = String::from_utf8(body.to_vec()).unwrap();
    idp.token_forms.lock().unwrap().push(form.clone());
    let code = url::form_urlencoded::parse(form.as_bytes())
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let Some(profile) = idp.profiles.lock().unwrap().get(&code).cloned() else {
        return (StatusCode::BAD_REQUEST, "bad code").into_response();
    };
    json_response(json!({
        "access_token": format!("at-{code}"),
        "refresh_token": "rt",
        "id_token": unsigned_jwt(&profile),
        "expires_in": 3600,
        "scope": "openid email",
    }))
    .into_response()
}

async fn idp_userinfo(State(idp): State<Arc<Idp>>, headers: HeaderMap) -> Response {
    let token = headers[header::AUTHORIZATION].to_str().unwrap();
    let code = token.trim_start_matches("Bearer at-");
    match idp.profiles.lock().unwrap().get(code) {
        Some(profile) => json_response(profile.clone()).into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn start_idp() -> (Arc<Idp>, String) {
    let idp = Arc::new(Idp::default());
    let app = Router::new()
        .route("/token", post(idp_token))
        .route("/userinfo", get(idp_userinfo))
        .with_state(Arc::clone(&idp));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (idp, base)
}

fn provider(id: &str, kind: OAuthKind, idp: &str) -> OAuthProvider {
    OAuthProvider {
        id: id.to_string(),
        kind,
        client_id: format!("{id}-client"),
        client_secret: "shh".to_string(),
        authorization_url: format!("{idp}/authorize"),
        token_url: format!("{idp}/token"),
        user_info_url: (kind != OAuthKind::Google).then(|| format!("{idp}/userinfo")),
        scopes: vec!["openid".into(), "email".into()],
        pkce: kind != OAuthKind::Facebook,
        redirect_uri: format!("http://app/api/auth/callback/{id}"),
        extra_params: Vec::new(),
    }
}

async fn state(idp: &str, sign_up_disabled: bool) -> Arc<OAuthState> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email", "emailVerified")
               VALUES (7, 'Seven', 'seven@app.test', 1), (8, 'Eight', 'eight@app.test', 0)"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let auth = Arc::new(AuthState {
        db: Arc::new(db),
        settings: AuthSettings {
            secret: "oauth-secret".to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled,
            trusted_origins: TrustedOriginSettings {
                builder_url: "http://app".to_string(),
                ..TrustedOriginSettings::default()
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    Arc::new(OAuthState {
        auth,
        http: reqwest::Client::new(),
        providers: vec![
            provider("google", OAuthKind::Google, idp),
            provider("facebook", OAuthKind::Facebook, idp),
            provider("oidc", OAuthKind::Generic, idp),
        ],
        platform: None,
        trusted_providers: vec!["google".into(), "facebook".into()],
        trust_oauth_email: false,
    })
}

async fn send(state: &Arc<OAuthState>, request: Request<Body>) -> Response {
    oauth_router(Arc::clone(state))
        .oneshot(request)
        .await
        .unwrap()
}

async fn sign_in(state: &Arc<OAuthState>, path: &str, body: Value) -> Response {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::HOST, "app")
        .header("origin", "http://app")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(state, request).await
}

async fn json_body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

/// Starts a flow and returns the `state` of the authorization URL.
async fn start(state: &Arc<OAuthState>, provider: &str) -> String {
    let response = sign_in(
        state,
        "/api/auth/sign-in/social",
        json!({
            "provider": provider,
            "callbackURL": "/dashboard",
            "newUserCallbackURL": "/welcome",
            "errorCallbackURL": "/sign-in"
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    query(body["url"].as_str().unwrap())["state"].clone()
}

async fn callback(state: &Arc<OAuthState>, provider: &str, query: &str) -> Response {
    let request = Request::builder()
        .uri(format!("/api/auth/callback/{provider}?{query}"))
        .header(header::HOST, "app")
        .body(Body::empty())
        .unwrap();
    send(state, request).await
}

async fn accounts_of(state: &OAuthState, email: &str) -> Vec<(String, String, bool)> {
    let email = email.to_string();
    state
        .auth
        .db
        .read(move |conn| {
            let mut statement = conn.prepare(
                r#"SELECT a."providerId", a."accountId", u."emailVerified"
                   FROM "Account" a JOIN "User" u ON u."id" = a."userId"
                   WHERE u."email" = ?1 ORDER BY a."providerId""#,
            )?;
            let rows =
                statement.query_map([email], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn sign_in_social_builds_the_authorization_url_and_stores_the_state() {
    let (_idp, base) = start_idp().await;
    let state = state(&base, false).await;
    let response = sign_in(
        &state,
        "/api/auth/sign-in/social",
        json!({ "provider": "google", "callbackURL": "/dashboard" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let header_location = location(&response);
    let body = json_body(response).await;
    assert_eq!(body["redirect"], json!(true));
    let url = body["url"].as_str().unwrap();
    assert_eq!(header_location, url);
    assert!(url.starts_with(&format!("{base}/authorize?")));
    let params = query(url);
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], "google-client");
    assert_eq!(
        params["redirect_uri"],
        "http://app/api/auth/callback/google"
    );
    assert_eq!(params["scope"], "openid email");
    assert_eq!(params["code_challenge_method"], "S256");
    let oauth_state = params["state"].clone();
    let identifier = format!("auth-state:{oauth_state}");
    let stored: String = state
        .auth
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "value" FROM "Verification" WHERE "identifier" = ?1"#,
                [identifier],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["callbackURL"], json!("/dashboard"));
    assert_eq!(
        params["code_challenge"],
        code_challenge(stored["codeVerifier"].as_str().unwrap())
    );
}

#[tokio::test]
async fn sign_in_rejects_an_unknown_provider_and_an_untrusted_callback() {
    let (_idp, base) = start_idp().await;
    let state = state(&base, false).await;
    let response = sign_in(
        &state,
        "/api/auth/sign-in/social",
        json!({ "provider": "github" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(response).await,
        json!({ "code": "PROVIDER_NOT_FOUND", "message": "Provider not found" })
    );
    let response = sign_in(
        &state,
        "/api/auth/sign-in/social",
        json!({ "provider": "google", "callbackURL": "https://evil.test/x" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_new_google_user_signs_up_and_lands_on_the_new_user_url() {
    let (idp, base) = start_idp().await;
    let state = state(&base, false).await;
    idp.profiles.lock().unwrap().insert(
        "c1".into(),
        json!({ "sub": "g-1", "email": "New@App.Test", "email_verified": true, "name": "New", "picture": "https://img/p.png" }),
    );
    let oauth_state = start(&state, "google").await;
    let response = callback(&state, "google", &format!("code=c1&state={oauth_state}")).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(location(&response), "/welcome");
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("better-auth.session_token=")
    );
    assert_eq!(
        accounts_of(&state, "new@app.test").await,
        [("google".to_string(), "g-1".to_string(), true)]
    );
    let form = idp.token_forms.lock().unwrap().last().unwrap().clone();
    assert!(form.contains("code_verifier="));
    assert!(form.contains("client_secret=shh"));

    // The state is single use.
    let reused = callback(&state, "google", &format!("code=c1&state={oauth_state}")).await;
    assert_eq!(
        location(&reused),
        "http://app/api/auth/error?error=state_mismatch"
    );
}

#[tokio::test]
async fn a_trusted_provider_links_to_an_existing_user_and_a_second_sign_in_reuses_it() {
    let (idp, base) = start_idp().await;
    let state = state(&base, false).await;
    idp.profiles.lock().unwrap().insert(
        "c2".into(),
        json!({ "sub": "g-7", "email": "seven@app.test", "email_verified": true }),
    );
    for _ in 0..2 {
        let oauth_state = start(&state, "google").await;
        let response = callback(&state, "google", &format!("code=c2&state={oauth_state}")).await;
        assert_eq!(location(&response), "/dashboard");
    }
    assert_eq!(
        accounts_of(&state, "seven@app.test").await,
        [("google".to_string(), "g-7".to_string(), true)]
    );
}

#[tokio::test]
async fn an_untrusted_provider_without_a_verified_email_is_not_linked() {
    let (idp, base) = start_idp().await;
    let state = state(&base, false).await;
    idp.profiles.lock().unwrap().insert(
        "c3".into(),
        json!({ "sub": "o-7", "email": "seven@app.test", "email_verified": false }),
    );
    let oauth_state = start(&state, "oidc").await;
    let response = callback(&state, "oidc", &format!("code=c3&state={oauth_state}")).await;
    assert_eq!(location(&response), "/sign-in?error=account_not_linked");
    assert!(accounts_of(&state, "seven@app.test").await.is_empty());
}

#[tokio::test]
async fn facebook_without_an_email_gets_a_placeholder_user() {
    let (idp, base) = start_idp().await;
    let state = state(&base, false).await;
    idp.profiles.lock().unwrap().insert(
        "c4".into(),
        json!({ "id": "1234", "name": "Fb", "picture": { "data": { "url": "https://img/f.png" } } }),
    );
    let oauth_state = start(&state, "facebook").await;
    let response = callback(&state, "facebook", &format!("code=c4&state={oauth_state}")).await;
    assert_eq!(location(&response), "/welcome");
    let placeholder = chatbotx_core::utils::placeholder_email::facebook_placeholder_email("1234");
    assert_eq!(
        accounts_of(&state, &placeholder).await,
        [("facebook".to_string(), "1234".to_string(), false)]
    );
}

#[tokio::test]
async fn callback_errors_redirect_to_the_error_url() {
    let (_idp, base) = start_idp().await;
    let state = state(&base, false).await;
    let response = callback(&state, "google", "code=x").await;
    assert_eq!(
        location(&response),
        "http://app/api/auth/error?error=state_not_found"
    );
    let oauth_state = start(&state, "google").await;
    let response = callback(
        &state,
        "google",
        &format!("error=access_denied&error_description=no&state={oauth_state}"),
    )
    .await;
    assert_eq!(
        location(&response),
        "/sign-in?error=access_denied&error_description=no"
    );
    let oauth_state = start(&state, "google").await;
    let response = callback(
        &state,
        "google",
        &format!("code=unknown&state={oauth_state}"),
    )
    .await;
    assert_eq!(location(&response), "/sign-in?error=invalid_code");
}

#[tokio::test]
async fn a_closed_sign_up_refuses_a_new_oauth_user() {
    let (idp, base) = start_idp().await;
    let state = state(&base, true).await;
    idp.profiles.lock().unwrap().insert(
        "c5".into(),
        json!({ "sub": "g-9", "email": "nine@app.test", "email_verified": true }),
    );
    let oauth_state = start(&state, "google").await;
    let response = callback(&state, "google", &format!("code=c5&state={oauth_state}")).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json_body(response).await,
        json!({ "message": "signup_disabled" })
    );
    assert!(accounts_of(&state, "nine@app.test").await.is_empty());
}
