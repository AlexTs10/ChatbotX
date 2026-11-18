//! Q.A1: the MCP OAuth authorization server end to end (register →
//! authorize → consent → token → refresh) and its security rules: PKCE S256,
//! single-use codes, exact redirect URIs, hashed secrets / refresh tokens,
//! consent bound to the session user and an admin workspace. The TS side
//! has no route test of the better-auth plugin; these cover the behaviour
//! `packages/auth/src/mcp-config.ts` configures.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{
    AuthSettings, AuthState, DbJwtVerifier, McpOAuth, TrustedOriginSettings, mcp_oauth_router,
    mcp_resource_url, oauth_consent_page, sign_cookie_value, submit_oauth_consent,
    verify_mcp_access_token,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_kv::{Kv, KvConfig, SystemClock};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt as _;

const ORIGIN: &str = "https://app.example.com";
const SECRET: &str = "a-very-long-test-secret-for-better-auth-0123456789";
const REDIRECT: &str = "http://127.0.0.1:5555/cb";
const WS: i64 = 2001;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn verifier() -> String {
    "v".repeat(64)
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

struct Fixture {
    state: Arc<AuthState>,
    router: Router,
}

/// User 7 owns workspace 2001; user 8 is a plain agent of it. Both have a session.
async fn fixture() -> Fixture {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    let db = Arc::new(db);
    let state = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: ORIGIN.to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                builder_url: ORIGIN.to_string(),
                ..TrustedOriginSettings::default()
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let expires = now_ms() + 3_600_000;
    db.write(move |tx| {
        for (id, role) in [(7, "owner"), (8, "agent")] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "name", "email") VALUES (?1, 'U', ?2)"#,
                params![id, format!("u{id}@example.com")],
            )?;
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?1, ?3)"#,
                params![id, format!("session-{id}"), expires],
            )?;
            if id == 7 {
                tx.execute(
                    r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'Acme', 7)"#,
                    [WS],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (?1, ?2, ?3)"#,
                params![WS, id, role],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let kv = Kv::start(Arc::clone(&db), Arc::new(SystemClock), KvConfig::default());
    let router = mcp_oauth_router(Arc::new(McpOAuth {
        auth: Arc::clone(&state),
        kv,
    }));
    Fixture { state, router }
}

fn cookie(user: i64) -> String {
    format!(
        "better-auth.session_token={}",
        sign_cookie_value(&format!("session-{user}"), SECRET)
    )
}

fn cookie_headers(user: i64) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&cookie(user)).unwrap(),
    );
    headers
}

async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn location(response: &Response) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

impl Fixture {
    async fn send(&self, request: Request<Body>) -> Response {
        self.router.clone().oneshot(request).await.unwrap()
    }

    async fn register(&self, body: Value) -> Response {
        self.send(
            Request::post("/api/auth/oauth2/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "203.0.113.9")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn public_client(&self) -> String {
        let response = self
            .register(json!({
                "client_name": "Claude",
                "redirect_uris": ["http://127.0.0.1/cb"],
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            }))
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body_json(response).await["client_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn authorize(&self, user: Option<i64>, query: &[(&str, &str)]) -> Response {
        let mut uri = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in query {
            uri.append_pair(k, v);
        }
        let mut request = Request::get(format!("/api/auth/oauth2/authorize?{}", uri.finish()));
        if let Some(user) = user {
            request = request.header(header::COOKIE, cookie(user));
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }

    async fn token(&self, form: &[(&str, &str)]) -> Response {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in form {
            body.append_pair(k, v);
        }
        self.send(
            Request::post("/api/auth/oauth2/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.finish()))
                .unwrap(),
        )
        .await
    }

    /// Authorize + accept consent for workspace 2001 as user 7: the code.
    async fn code(&self, client_id: &str) -> String {
        let challenge = challenge(&verifier());
        let query = [
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT),
            ("scope", "openid offline_access"),
            ("state", "xyz"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        let response = self.authorize(Some(7), &query).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        let target = location(&response);
        if let Some(code) = query_param(&target, "code") {
            return code;
        }
        let consent_query = target.strip_prefix("/oauth/consent?").expect(&target);
        let result = submit_oauth_consent(
            &self.state,
            &cookie_headers(7),
            &json!({ "accept": true, "oauthQuery": consent_query, "workspaceId": WS.to_string() }),
        )
        .await
        .unwrap();
        let url = result["url"].as_str().unwrap();
        assert!(url.starts_with(REDIRECT), "{url}");
        assert_eq!(query_param(url, "state").as_deref(), Some("xyz"));
        assert_eq!(query_param(url, "iss").as_deref(), Some(ORIGIN));
        query_param(url, "code").unwrap()
    }

    async fn exchange(&self, client_id: &str, code: &str, verifier: &str) -> Response {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .await
    }

    async fn count(&self, sql: &'static str) -> i64 {
        self.state
            .db
            .read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn well_known_metadata_points_at_the_auth_endpoints() {
    let f = fixture().await;
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        let response = f
            .send(Request::get(path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["issuer"], ORIGIN);
        assert_eq!(
            body["token_endpoint"],
            format!("{ORIGIN}/api/auth/oauth2/token")
        );
        assert_eq!(body["code_challenge_methods_supported"], json!(["S256"]));
    }
    let resource = f
        .send(
            Request::get("/.well-known/oauth-protected-resource/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resource.status(), StatusCode::OK);
    let body = body_json(resource).await;
    assert_eq!(body["resource"], format!("{ORIGIN}/mcp"));
    assert_eq!(body["authorization_servers"], json!([ORIGIN]));
    let other = f
        .send(
            Request::get("/.well-known/oauth-protected-resource/other")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(other.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn registration_validates_redirects_and_hashes_the_secret() {
    let f = fixture().await;
    let insecure = f
        .register(json!({ "redirect_uris": ["http://evil.example/cb"], "application_type": "web" }))
        .await;
    assert_eq!(insecure.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(insecure).await["error"], "invalid_redirect_uri");
    let no_pkce = f
        .register(json!({ "redirect_uris": ["https://a.example/cb"], "require_pkce": false }))
        .await;
    assert_eq!(no_pkce.status(), StatusCode::BAD_REQUEST);

    let confidential = f
        .register(json!({ "redirect_uris": ["https://a.example/cb"] }))
        .await;
    assert_eq!(confidential.status(), StatusCode::CREATED);
    let body = body_json(confidential).await;
    let secret = body["client_secret"].as_str().unwrap().to_string();
    let client_id = body["client_id"].as_str().unwrap().to_string();
    let stored: String = f
        .state
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT "clientSecret" FROM "OAuthClient" WHERE "clientId" = ?1"#,
                [client_id],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_ne!(stored, secret);
    assert_eq!(
        stored,
        URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
    );
}

#[tokio::test]
async fn full_flow_issues_a_verifiable_token_and_codes_are_single_use() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let code = f.code(&client_id).await;

    let response = f.exchange(&client_id, &code, &verifier()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let tokens = body_json(response).await;
    let access = tokens["access_token"].as_str().unwrap();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_string();
    assert!(tokens["id_token"].is_string());

    let verifier_db = DbJwtVerifier {
        db: &f.state.db,
        now_ms: now_ms(),
    };
    let subject = verify_mcp_access_token(
        &verifier_db,
        &format!("{ORIGIN}/api/auth"),
        &mcp_resource_url(&f.state),
        access,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(subject.user_id, "7");
    assert_eq!(subject.client_id, client_id);

    // The refresh token is stored hashed.
    assert_eq!(
        f.count(r#"SELECT count(*) FROM "OAuthRefreshToken""#).await,
        1
    );
    let plain = refresh.clone();
    let stored_plain: i64 = f
        .state
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT count(*) FROM "OAuthRefreshToken" WHERE "token" = ?1"#,
                [plain],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(stored_plain, 0);

    // Consent is bound to user 7 and the selected workspace.
    assert_eq!(
        f.count(
            r#"SELECT count(*) FROM "OAuthConsent" WHERE "userId" = 7 AND "workspaceId" = 2001"#
        )
        .await,
        1
    );

    // A replayed code fails and drops the tokens it issued.
    let replay = f.exchange(&client_id, &code, &verifier()).await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(replay).await["error"], "invalid_grant");
    assert_eq!(
        f.count(r#"SELECT count(*) FROM "OAuthRefreshToken""#).await,
        0
    );
    let stale = f
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pkce_is_required_and_checked() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let without = f
        .authorize(
            Some(7),
            &[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("state", "s"),
            ],
        )
        .await;
    let target = location(&without);
    assert!(target.starts_with(REDIRECT), "{target}");
    assert_eq!(
        query_param(&target, "error").as_deref(),
        Some("invalid_request")
    );

    let plain = f
        .authorize(
            Some(7),
            &[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", "abc"),
                ("code_challenge_method", "plain"),
            ],
        )
        .await;
    assert_eq!(
        query_param(&location(&plain), "error").as_deref(),
        Some("invalid_request")
    );

    let code = f.code(&client_id).await;
    let wrong = f.exchange(&client_id, &code, &"w".repeat(64)).await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(wrong).await["error"], "invalid_grant");
    // The code was consumed by the failed attempt.
    let retry = f.exchange(&client_id, &code, &verifier()).await;
    assert_eq!(retry.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn redirect_uri_must_match_a_registered_one() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let challenge = challenge(&verifier());
    for redirect in [
        "http://127.0.0.1:5555/cb/extra",
        "http://127.0.0.1:5555/other",
        "https://evil.example/cb",
    ] {
        let response = f
            .authorize(
                Some(7),
                &[
                    ("response_type", "code"),
                    ("client_id", &client_id),
                    ("redirect_uri", redirect),
                    ("code_challenge", &challenge),
                    ("code_challenge_method", "S256"),
                ],
            )
            .await;
        let target = location(&response);
        assert!(
            target.starts_with(&format!("{ORIGIN}/api/auth/error")),
            "{target}"
        );
        assert_eq!(
            query_param(&target, "error").as_deref(),
            Some("invalid_redirect")
        );
    }
    // The token request must repeat the same redirect URI.
    let code = f.code(&client_id).await;
    let mismatch = f
        .token(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "http://127.0.0.1:5555/elsewhere"),
            ("client_id", &client_id),
            ("code_verifier", &verifier()),
        ])
        .await;
    assert_eq!(body_json(mismatch).await["error"], "invalid_grant");
}

#[tokio::test]
async fn unauthenticated_authorize_goes_to_sign_in_with_a_signed_query() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let challenge = challenge(&verifier());
    let response = f
        .authorize(
            None,
            &[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
            ],
        )
        .await;
    let target = location(&response);
    assert!(target.starts_with("/oauth/sign-in?"), "{target}");
    assert!(query_param(&target, "sig").is_some());
}

#[tokio::test]
async fn consent_is_bound_to_the_session_user_and_an_admin_workspace() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let challenge = challenge(&verifier());
    let response = f
        .authorize(
            Some(8),
            &[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
            ],
        )
        .await;
    let target = location(&response);
    let query = target.strip_prefix("/oauth/consent?").expect(&target);

    // User 8 is an agent, not an admin of 2001: refused, nothing stored.
    let error = submit_oauth_consent(
        &f.state,
        &cookie_headers(8),
        &json!({ "accept": true, "oauthQuery": query, "workspaceId": WS.to_string() }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(f.count(r#"SELECT count(*) FROM "OAuthConsent""#).await, 0);

    // The page lists no authorizable workspace for user 8.
    let page = oauth_consent_page(
        &f.state,
        &cookie_headers(8),
        &json!({ "oauthQuery": query }),
    )
    .await
    .unwrap();
    assert_eq!(page["workspaces"], json!([]));

    // A tampered query is refused.
    let tampered = query.replace("S256", "S257");
    let error = submit_oauth_consent(
        &f.state,
        &cookie_headers(7),
        &json!({ "accept": true, "oauthQuery": tampered, "workspaceId": WS.to_string() }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, 403);

    // No session: 401.
    let error = submit_oauth_consent(
        &f.state,
        &HeaderMap::new(),
        &json!({ "accept": true, "oauthQuery": query, "workspaceId": WS.to_string() }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, 401);

    // Deny sends the user back with access_denied.
    let denied = submit_oauth_consent(
        &f.state,
        &cookie_headers(8),
        &json!({ "accept": false, "oauthQuery": query }),
    )
    .await
    .unwrap();
    let url = denied["url"].as_str().unwrap();
    assert!(url.starts_with(REDIRECT));
    assert_eq!(query_param(url, "error").as_deref(), Some("access_denied"));
}

#[tokio::test]
async fn refresh_rotates_and_a_reused_token_revokes_the_family() {
    let f = fixture().await;
    let client_id = f.public_client().await;
    let code = f.code(&client_id).await;
    let tokens = body_json(f.exchange(&client_id, &code, &verifier()).await).await;
    let first = tokens["refresh_token"].as_str().unwrap().to_string();

    let rotated = f
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &first),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated = body_json(rotated).await;
    let second = rotated["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(first, second);

    // A widened scope on the old token is refused (inside the reuse window).
    let widened = f
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &first),
            ("client_id", &client_id),
            ("scope", "openid offline_access email"),
        ])
        .await;
    assert_eq!(widened.status(), StatusCode::BAD_REQUEST);

    // Another client cannot use the token.
    let other = f.public_client().await;
    let stolen = f
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &second),
            ("client_id", &other),
        ])
        .await;
    assert_eq!(stolen.status(), StatusCode::BAD_REQUEST);

    // Reuse after the window: the whole family is revoked.
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "OAuthRefreshToken" SET "rotationReplayExpiresAt" = 1 WHERE "revoked" IS NOT NULL"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let reuse = f
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &first),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(reuse.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        f.count(r#"SELECT count(*) FROM "OAuthRefreshToken""#).await,
        0
    );
}
