//! Ports of `apps/builder/__tests__/reflink-widget-route.test.ts` and the
//! "chat widget" describe of `reflinks-public-api.test.ts`. Adapted: the
//! mocked `findForWidget`, link builder and guest rate limit become real
//! `Reflink` / `Inbox` / `MediaLibraryFile` rows and the kv fixed-window
//! counters; the mocked tenant is the fixture app URL (`http://app`, storage
//! `http://app/storage/`).

mod handlers_common;

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::reflink_widget;
use chatbotx_db::rusqlite::params;
use chatbotx_kv::{Clock, Kv, KvConfig, SystemClock};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const REFLINK: i64 = 10;

/// Flow 1, inboxes 1 (messenger "Page") and 2 (whatsapp "Phone", display
/// number 84), logo file
/// 50, ref link 10 `summer` with `widget` columns.
async fn seed(f: &Fixture, widget: Value) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (1, ?1, 'Flow 1')"#,
                params![WS],
            )?;
            for (id, channel, source, name) in [
                (1_i64, "messenger", "1", "Page"),
                (2, "whatsapp", "84", "Phone"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status") VALUES (?1, ?2, ?3, ?4, ?5, 'connected')"#,
                    params![id, WS, name, channel, source],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp"
                     ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "displayPhoneNumber")
                   VALUES (1, '{}', 'pn-1', 'waba-1', 'biz-1', 'Phone', ?1, 2, '84')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "MediaLibraryFile" ("id", "workspaceId", "name", "path", "mimeType", "size")
                   VALUES (50, ?1, 'logo.png', 'public/space/ws-1/media-library/logo123', 'image/png', 10)"#,
                params![WS],
            )?;
            let text = |key: &str| widget.get(key).and_then(Value::as_str).map(str::to_string);
            let list = |key: &str| widget.get(key).map(Value::to_string);
            tx.execute(
                r#"INSERT INTO "Reflink"
                     ("id", "workspaceId", "type", "name", "flowId", "widgetAuthorizedDomains", "widgetHiddenInboxIds",
                      "widgetLogoFileId", "widgetBrandName", "widgetBrandUrl", "widgetLogoBackgroundColor")
                   VALUES (?1, ?2, 'refLink', 'summer', 1, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    REFLINK,
                    WS,
                    list("authorizedDomains").unwrap_or_else(|| "[]".into()),
                    list("hiddenInboxIds").unwrap_or_else(|| "[]".into()),
                    widget.get("logoFileId").and_then(Value::as_i64),
                    text("brandName"),
                    text("brandUrl"),
                    text("logoBackgroundColor"),
                ],
            )?;
            Ok(())
        })
        .await
        .expect("seed widget");
}

fn headers(origin: Option<&str>, extra: &[(&'static str, &str)]) -> HeaderMap {
    let mut out = HeaderMap::new();
    if let Some(origin) = origin {
        out.insert("origin", HeaderValue::from_str(origin).expect("origin"));
    }
    for (key, value) in extra {
        out.insert(*key, HeaderValue::from_str(value).expect("header"));
    }
    if !out.contains_key("x-forwarded-for") && !extra.iter().any(|(k, _)| *k == "x-no-ip") {
        out.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
    }
    out.remove("x-no-ip");
    out
}

struct Reply {
    status: u16,
    headers: HeaderMap,
    body: Value,
}

async fn get_widget(state: &ApiState, reflink_id: &str, headers: &HeaderMap) -> Reply {
    let response = reflink_widget::get(state, reflink_id, headers).await;
    let status = response.status().as_u16();
    let out_headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    Reply {
        status,
        headers: out_headers,
        body,
    }
}

async fn widget_at(f: &Fixture, origin: &str) -> Reply {
    get_widget(&f.state, "10", &headers(Some(origin), &[])).await
}

fn with_kv(f: &Fixture) -> ApiState {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let kv = Kv::start(Arc::clone(&f.state.db), clock, KvConfig::default());
    f.state.clone().with_kv(kv)
}

// --- GET /api/reflink-widget/[reflinkId] ---------------------------------------------

#[tokio::test]
async fn allows_any_origin_with_the_default_icon_and_no_powered_by_by_default() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let reply = widget_at(&f, "https://shop.example.com").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(
        reply
            .headers
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap_or_default()),
        Some("https://shop.example.com")
    );
    assert_eq!(
        reply
            .headers
            .get("cache-control")
            .map(|v| v.to_str().unwrap_or_default()),
        Some("no-store")
    );
    assert_eq!(
        reply.body,
        json!({
            "channels": [
                { "channel": "messenger", "name": "Page", "url": "https://m.me/1?ref=summer" },
                { "channel": "whatsapp", "name": "Phone", "url": "https://wa.me/84?text=%2Fref-summer" },
            ],
            "brand": {
                "name": null,
                "url": null,
                "logoUrl": null,
                "logoBackgroundColor": "#111827",
                "logoForegroundColor": "#ffffff",
                "poweredByLabel": "by",
                "toggleLabel": "Chat with us",
            },
        })
    );
}

#[tokio::test]
async fn brands_by_the_ref_link_own_logo_name_and_url_once_saved() {
    let f = fixture().await;
    seed(
        &f,
        json!({ "logoFileId": 50, "brandName": "Shop", "brandUrl": "https://shop.test/landing?sig=abc" }),
    )
    .await;
    let reply = widget_at(&f, "https://shop.example.com").await;
    assert_eq!(
        reply.body["brand"],
        json!({
            "name": "Shop",
            "url": "https://shop.test/landing?sig=abc",
            "logoUrl": "http://app/storage/public/space/ws-1/media-library/logo123",
            "logoBackgroundColor": "#111827",
            "logoForegroundColor": "#ffffff",
            "poweredByLabel": "by",
            "toggleLabel": "Chat with us",
        })
    );
}

#[tokio::test]
async fn draws_the_default_icon_on_the_saved_background_in_a_readable_color() {
    let f = fixture().await;
    seed(&f, json!({ "logoBackgroundColor": "#fde047" })).await;
    let brand = widget_at(&f, "https://shop.example.com").await.body["brand"].clone();
    assert_eq!(brand["logoUrl"], json!(null));
    assert_eq!(brand["logoBackgroundColor"], json!("#fde047"));
    assert_eq!(brand["logoForegroundColor"], json!("#0a0a0a"));
}

#[tokio::test]
async fn hides_powered_by_with_only_a_brand_name_or_only_a_redirect_url() {
    for branding in [
        json!({ "brandName": "Shop" }),
        json!({ "brandUrl": "https://shop.test" }),
    ] {
        let f = fixture().await;
        seed(&f, branding.clone()).await;
        let brand = widget_at(&f, "https://shop.example.com").await.body["brand"].clone();
        assert_eq!(brand["name"], json!(null), "{branding}");
        assert_eq!(brand["url"], json!(null), "{branding}");
    }
}

#[tokio::test]
async fn allows_an_authorized_domain_and_its_subdomains() {
    let f = fixture().await;
    seed(&f, json!({ "authorizedDomains": ["example.com"] })).await;
    assert_eq!(widget_at(&f, "https://example.com").await.status, 200);
    assert_eq!(widget_at(&f, "https://shop.example.com").await.status, 200);
}

#[tokio::test]
async fn rejects_an_origin_outside_the_authorized_domains() {
    let f = fixture().await;
    seed(&f, json!({ "authorizedDomains": ["example.com"] })).await;
    let reply = widget_at(&f, "https://evil.test").await;
    assert_eq!(reply.status, 403);
    assert!(reply.headers.get("access-control-allow-origin").is_none());
    assert_eq!(reply.body, Value::Null);
}

#[tokio::test]
async fn leaves_hidden_inboxes_out() {
    let f = fixture().await;
    seed(&f, json!({ "hiddenInboxIds": ["1"] })).await;
    let body = widget_at(&f, "https://example.com").await.body;
    let channels: Vec<&str> = body["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|c| c["channel"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(channels, vec!["whatsapp"]);
}

#[tokio::test]
async fn returns_404_for_a_missing_ref_link_or_a_non_numeric_id() {
    let f = fixture().await;
    let h = headers(Some("https://example.com"), &[]);
    assert_eq!(get_widget(&f.state, "10", &h).await.status, 404);
    seed(&f, json!({})).await;
    assert_eq!(get_widget(&f.state, "abc", &h).await.status, 404);
}

#[tokio::test]
async fn returns_429_when_rate_limited() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let state = with_kv(&f);
    let h = headers(Some("https://example.com"), &[]);
    // 60 requests per IP fit in one 10 s window; the next is limited. A
    // window edge can reset the counter once, so allow one extra round.
    let mut limited = None;
    for _ in 0..130 {
        let reply = get_widget(&state, "10", &h).await;
        if reply.status == 429 {
            limited = Some(reply);
            break;
        }
        assert_eq!(reply.status, 200);
    }
    let reply = limited.expect("a 429 after the IP limit");
    let retry: i64 = reply
        .headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .expect("Retry-After");
    assert!((1..=10).contains(&retry), "{retry}");
    assert_eq!(reply.body, Value::Null);
}

#[tokio::test]
async fn skips_the_rate_limit_when_the_visitor_ip_is_unknown() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let state = with_kv(&f);
    let h = headers(Some("https://example.com"), &[("x-no-ip", "1")]);
    assert_eq!(reflink_widget::guest_client_ip(&h), "unknown");
    for _ in 0..70 {
        assert_eq!(get_widget(&state, "10", &h).await.status, 200);
    }
}

#[tokio::test]
async fn translates_the_powered_by_label_to_the_visitor_language() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let h = headers(
        Some("https://example.com"),
        &[("accept-language", "vi-VN,vi;q=0.9,en;q=0.8")],
    );
    let reply = get_widget(&f.state, "10", &h).await;
    assert_eq!(reply.body["brand"]["poweredByLabel"], json!("bởi"));
}

// --- reflinks public API: chat widget -----------------------------------------------

#[tokio::test]
async fn chat_widget_get_returns_settings_embed_code_on_the_tenant_domain_and_visible_channels() {
    let f = fixture().await;
    seed(
        &f,
        json!({
            "authorizedDomains": ["example.com"],
            "hiddenInboxIds": ["2"],
            "logoFileId": 50,
            "logoBackgroundColor": "#111827",
            "brandName": "Acme",
            "brandUrl": "https://acme.test",
        }),
    )
    .await;
    let (status, body) = call(
        &f,
        "reflinks.getChatWidget",
        WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reflinkId"], json!("10"));
    assert_eq!(body["authorizedDomains"], json!(["example.com"]));
    assert_eq!(body["hiddenInboxIds"], json!(["2"]));
    assert_eq!(body["logoFileId"], json!("50"));
    assert_eq!(body["logoBackgroundColor"], json!("#111827"));
    assert_eq!(body["brandName"], json!("Acme"));
    assert_eq!(body["brandUrl"], json!("https://acme.test"));
    assert_eq!(
        body["scriptUrl"],
        json!("http://app/chat-widget/ref-widget.js")
    );
    assert_eq!(
        body["embedCode"],
        json!(
            r#"<script async src="http://app/chat-widget/ref-widget.js" data-reflink-id="10"></script>"#
        )
    );
    assert!(
        body["logoUrl"]
            .as_str()
            .is_some_and(|url| url.contains("media-library/logo123")),
        "{body}"
    );
    // The hidden WhatsApp inbox is left out, as on the live widget.
    let inboxes: Vec<&str> = body["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .map(|link| link["inboxId"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(inboxes, vec!["1"]);
}

#[tokio::test]
async fn chat_widget_put_saves_the_settings_then_returns_the_reloaded_widget() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let settings = json!({
        "authorizedDomains": ["example.com"],
        "hiddenInboxIds": ["2"],
        "logoFileId": "50",
        "logoBackgroundColor": "#111827",
        "brandName": "Acme",
        "brandUrl": "https://acme.test",
    });
    let (status, body) = call(
        &f,
        "reflinks.updateChatWidget",
        WS,
        &[("id", "10")],
        None,
        Some(settings),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["embedCode"]
            .as_str()
            .is_some_and(|code| code.contains(r#"data-reflink-id="10""#))
    );
    assert_eq!(body["brandName"], json!("Acme"));
    assert_eq!(body["hiddenInboxIds"], json!(["2"]));
    let (_, reloaded) = call(
        &f,
        "reflinks.getChatWidget",
        WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(reloaded, body);
}

#[tokio::test]
async fn chat_widget_a_ref_link_outside_the_workspace_surfaces_the_service_not_found() {
    let f = fixture().await;
    seed(&f, json!({})).await;
    let (status, body) = call(
        &f,
        "reflinks.getChatWidget",
        OTHER_WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], json!("notFound"));
}
