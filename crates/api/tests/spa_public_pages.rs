//! Q.F2-admin goal 3: the public page loaders (crates/api/src/spa/public_pages.rs)
//! and the `/go/{workspaceId}` document. Ports of
//! `app/connect/[sessionId]/resolve-message.test.ts`,
//! `features/bot-simulator/lib/build-simulator-link.test.ts` (URL part) and
//! `app/go/[workspaceId]/page.test.tsx` (forged / plain destinations).

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings};
use chatbotx_api::spa::public_pages::{connect_session_kind, parse_simulator_website_url};
use chatbotx_api::spa::spa_router_with;
use chatbotx_db::Db;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const WS: i64 = 700;
const CONTACT: i64 = 701;

fn now_ms() -> i64 {
    1_800_000_000_000
}

async fn setup() -> (Arc<AuthState>, Arc<ApiState>) {
    let db = Arc::new(Db::open_in_memory().expect("open db"));
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email", "emailVerified") VALUES (1, 'Own', 'own@pub.test', 1)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (?1, 'Pub', 1, 1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId", "emailOptIn") VALUES (?1, 'Ann', ?2, 1)"#,
            [CONTACT, WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let api = Arc::new(ApiState::new(Arc::clone(&db), "http://app"));
    let auth = Arc::new(AuthState {
        db,
        settings: AuthSettings {
            secret: "public-pages-secret".to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: true,
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
    (auth, api)
}

async fn send(
    auth: &Arc<AuthState>,
    api: &Arc<ApiState>,
    request: Request<Body>,
) -> (StatusCode, Option<String>, String) {
    let app = spa_router_with(Arc::clone(auth), Some(Arc::clone(api)));
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().expect("location").to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

async fn page(
    auth: &Arc<AuthState>,
    api: &Arc<ApiState>,
    name: &str,
    input: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/rpc/pages/{name}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "json": input, "meta": [] }).to_string()))
        .expect("request");
    let (status, _, body) = send(auth, api, request).await;
    let value: Value = serde_json::from_str(&body).expect("json body");
    (status, value["json"].clone())
}

async fn go(
    auth: &Arc<AuthState>,
    api: &Arc<ApiState>,
    query: &str,
) -> (StatusCode, Option<String>, String) {
    let request = Request::builder()
        .uri(format!("/go/{WS}?{query}"))
        .body(Body::empty())
        .expect("request");
    send(auth, api, request).await
}

#[test]
fn connect_session_status_maps_to_a_coarse_kind() {
    assert_eq!(connect_session_kind(None), "invalid");
    for status in ["pending", "authorized", "awaiting_selection"] {
        assert_eq!(connect_session_kind(Some(status)), "processing");
    }
    assert_eq!(connect_session_kind(Some("completed")), "completed");
    assert_eq!(connect_session_kind(Some("cancelled")), "cancelled");
    assert_eq!(connect_session_kind(Some("expired")), "expired");
    assert_eq!(connect_session_kind(Some("failed")), "failed");
}

#[test]
fn simulator_website_must_be_http() {
    assert!(parse_simulator_website_url(Some(" https://shop.test/a ")).is_some());
    assert!(parse_simulator_website_url(Some("javascript:alert(1)")).is_none());
    assert!(parse_simulator_website_url(Some("not a url")).is_none());
    assert!(parse_simulator_website_url(None).is_none());
}

#[tokio::test]
async fn unsubscribe_token_clears_the_email_opt_in_once_verified() {
    let (auth, api) = setup().await;
    let (_, forged) = page(&auth, &api, "unsubscribe", json!({ "token": "bm9wZQ" })).await;
    assert_eq!(forged["state"], "invalid");

    let url = chatbotx_business::email_marketing::smtp::unsubscribe_url("http://app", CONTACT, WS)
        .expect("unsubscribe url");
    let token = url.split("token=").nth(1).expect("token");
    let (status, done) = page(&auth, &api, "unsubscribe", json!({ "token": token })).await;
    assert_eq!(
        (status, done["state"].as_str()),
        (StatusCode::OK, Some("done"))
    );
    let opt_in: i64 = auth
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "emailOptIn" FROM "Contact" WHERE "id" = ?1"#,
                [CONTACT],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("contact");
    assert_eq!(opt_in, 0);

    // A workspace scheduled for deletion is not servable.
    auth.db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("schedule");
    let (_, unavailable) = page(&auth, &api, "unsubscribe", json!({ "token": token })).await;
    assert_eq!(unavailable["state"], "unavailable");
}

#[tokio::test]
async fn unknown_public_records_are_not_found() {
    let (auth, api) = setup().await;
    let (status, _) = page(
        &auth,
        &api,
        "landing",
        json!({ "workspaceId": WS.to_string(), "id": "9" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = page(
        &auth,
        &api,
        "botSimulator",
        json!({ "workspaceId": WS.to_string(), "webchatId": "9", "url": "https://shop.test" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, kind) = page(&auth, &api, "connectSession", json!({ "sessionId": "9" })).await;
    assert_eq!(kind["kind"], "invalid");
    let (_, picker) = page(&auth, &api, "datetimePicker", json!({ "token": "forged" })).await;
    assert_eq!(picker["state"], "invalid");
    let (status, _) = page(
        &auth,
        &api,
        "submitDateTime",
        json!({ "token": "forged", "selectedValue": "tomorrow" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn open_link_never_redirects_to_a_forged_destination() {
    use base64::Engine as _;
    let (auth, api) = setup().await;
    let destination = "https://evil.test/";
    let u = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(destination);
    let (status, location, body) = go(&auth, &api, &format!("u={u}&s=00")).await;
    assert_eq!((status, location), (StatusCode::OK, None));
    assert!(body.contains("Invalid link."), "{body}");

    let s = chatbotx_business::meta_catalog::process_encryption().sign_open_link_url(
        &chatbotx_core::encryption::OpenLinkParams {
            workspace_id: WS.to_string(),
            url: destination.to_string(),
        },
    );
    let (status, location, _) = go(&auth, &api, &format!("u={u}&s={s}")).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location.as_deref(), Some(destination));

    // A signature for another workspace does not verify here.
    let other = chatbotx_business::meta_catalog::process_encryption().sign_open_link_url(
        &chatbotx_core::encryption::OpenLinkParams {
            workspace_id: "1".to_string(),
            url: destination.to_string(),
        },
    );
    let (_, location, body) = go(&auth, &api, &format!("u={u}&s={other}")).await;
    assert_eq!(location, None);
    assert!(body.contains("Invalid link."));
}

// ---- `webchat-page-guards.test.tsx` -------------------------------------------

/// A webchat of `WS` with these authorized domains.
async fn seed_webchat(auth: &Arc<AuthState>, domains: &[&str]) {
    let domains = json!(domains).to_string();
    auth.db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (710, ?1, 'Chat', 'webchat', 'wc-1')"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWebchat" ("id", "auth", "name", "authorizedDomains", "conversationStarters",
                     "persistentMenus", "brandColor", "workspaceId", "inboxId")
                   VALUES (711, '{}', 'Chat', ?1, '[]', '[]', '#000000', ?2, 710)"#,
                chatbotx_db::rusqlite::params![domains, WS],
            )?;
            Ok(())
        })
        .await
        .expect("webchat");
}

/// The `webchat` loader as the SPA calls it on the app host `app`.
async fn webchat(
    auth: &Arc<AuthState>,
    api: &Arc<ApiState>,
    referrer: &str,
    in_iframe: bool,
    parent_origin: Option<&str>,
) -> Value {
    let input = json!({
        "workspaceId": WS.to_string(),
        "webchatId": "711",
        "referrer": referrer,
        "inIframe": in_iframe,
        "parentOrigin": parent_origin,
        "domain": null,
    });
    let request = Request::builder()
        .method("POST")
        .uri("/rpc/pages/webchat")
        .header(header::HOST, "app")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "json": input, "meta": [] }).to_string()))
        .expect("request");
    let (status, _, body) = send(auth, api, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).expect("json")["json"].clone()
}

/// The `originHost` the access token is bound to.
fn token_host(page: &Value) -> Value {
    use base64::Engine as _;
    let token = page["accessToken"].as_str().expect("token");
    let encoded = token.split('.').next().expect("payload");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("base64");
    serde_json::from_slice::<Value>(&bytes).expect("payload")["originHost"].clone()
}

#[tokio::test]
async fn webchat_without_referer_is_refused_only_in_a_restricted_iframe() {
    let (auth, api) = setup().await;
    seed_webchat(&auth, &["shop.test"]).await;
    let direct = webchat(&auth, &api, "", false, None).await;
    assert_eq!(direct["state"], "ok", "{direct}");
    // A direct open has no first-party referer: the URL parent origin is not trusted.
    let spoofed = webchat(&auth, &api, "", false, Some("https://evil.test")).await;
    assert_eq!(token_host(&spoofed), Value::Null);
    let framed = webchat(&auth, &api, "", true, None).await;
    assert_eq!(framed["state"], "unauthorized");
}

#[tokio::test]
async fn webchat_referer_less_iframe_is_allowed_without_domains() {
    let (auth, api) = setup().await;
    seed_webchat(&auth, &[]).await;
    assert_eq!(webchat(&auth, &api, "", true, None).await["state"], "ok");
}

#[tokio::test]
async fn webchat_first_party_referer_trusts_the_url_parent_origin() {
    let (auth, api) = setup().await;
    seed_webchat(&auth, &["shop.test"]).await;
    let page = webchat(
        &auth,
        &api,
        "http://app/space/1",
        true,
        Some("https://shop.test"),
    )
    .await;
    assert_eq!(page["state"], "ok", "{page}");
    assert_eq!(token_host(&page), "shop.test");
    let plain = webchat(&auth, &api, "http://app/space/1", true, None).await;
    assert_eq!(token_host(&plain), "app");
}

#[tokio::test]
async fn webchat_third_party_referer_must_be_authorized() {
    let (auth, api) = setup().await;
    seed_webchat(&auth, &["shop.test"]).await;
    let allowed = webchat(
        &auth,
        &api,
        "https://shop.test/p",
        true,
        Some("https://other.test"),
    )
    .await;
    assert_eq!(allowed["state"], "ok", "{allowed}");
    // The token stays bound to the referer, not the URL parent origin.
    assert_eq!(token_host(&allowed), "shop.test");
    let attacker = webchat(
        &auth,
        &api,
        "https://evil.test/p",
        true,
        Some("https://shop.test"),
    )
    .await;
    assert_eq!(attacker["state"], "unauthorized");
}

#[tokio::test]
async fn webchat_third_party_referer_fails_closed_without_domains() {
    let (auth, api) = setup().await;
    seed_webchat(&auth, &[]).await;
    let page = webchat(&auth, &api, "https://shop.test/p", true, None).await;
    assert_eq!(page["state"], "unauthorized");
}
