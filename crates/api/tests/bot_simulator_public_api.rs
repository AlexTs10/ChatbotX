//! Port of `apps/builder/__tests__/bot-simulator-public-api.test.ts`
//! (`createBotSimulatorLink`). Adapted: the mocked webchat service becomes a
//! real webchat made through `webchats.create`; the call is
//! `GET /v1/bot-simulator/link` through `handlers::dispatch`; the tenant app
//! URL is the fixture app URL (`http://app`).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn webchat(f: &Fixture, ws: i64, domains: &[&str], enable: bool) -> String {
    let body = json!({
        "name": "Site chat",
        "persistentMenus": [],
        "authorizedDomains": domains,
        "conversationStarters": [],
        "brandColor": "#112233",
        "hideHeader": false,
        "showLogo": true,
        "hideMessageInput": false,
        "enable": enable,
    });
    let (status, created) = call(f, "webchats.create", ws, &[], None, Some(body)).await;
    assert_eq!(status, 201, "{created}");
    created["id"].as_str().expect("id").to_owned()
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn link(f: &Fixture, webchat_id: &str, website: &str) -> (u16, Value) {
    let query = format!("webchatId={webchat_id}&websiteUrl={}", encode(website));
    call(f, "botSimulator.getLink", WS, &[], Some(&query), None).await
}

fn rejected_on(out: &(u16, Value), field: &str) {
    assert!(out.0 >= 400, "{}", out.1);
    assert!(out.1.to_string().contains(field), "{}", out.1);
}

#[tokio::test]
async fn registers_the_link_route_under_the_channels_scope() {
    let route = route_by_id("botSimulator.getLink").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["channels"]));
}

#[tokio::test]
async fn builds_the_bs_link_on_the_app_url() {
    let f = fixture().await;
    let id = webchat(&f, WS, &["example.com"], true).await;
    let (status, out) = link(&f, &id, "https://shop.example.com/pricing?a=1").await;
    assert_eq!(status, 200, "{out}");
    let url = url::Url::parse(out["url"].as_str().expect("url")).expect("parse");
    assert_eq!(url.origin().ascii_serialization(), "http://app");
    assert_eq!(url.path(), format!("/bs/{WS}/{id}"));
    let website = url
        .query_pairs()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned());
    assert_eq!(
        website.as_deref(),
        Some("https://shop.example.com/pricing?a=1")
    );
}

#[tokio::test]
async fn rejects_a_non_http_website_before_any_lookup() {
    let f = fixture().await;
    for website in ["javascript:alert(1)", "not a url", ""] {
        // An unknown webchat id: the website check comes first.
        rejected_on(&link(&f, "999", website).await, "websiteUrl");
    }
}

#[tokio::test]
async fn rejects_a_disabled_webchat() {
    let f = fixture().await;
    let id = webchat(&f, WS, &[], false).await;
    rejected_on(&link(&f, &id, "https://example.com").await, "webchatId");
}

#[tokio::test]
async fn rejects_a_website_outside_the_allowed_domains() {
    let f = fixture().await;
    let id = webchat(&f, WS, &["other.test"], true).await;
    rejected_on(&link(&f, &id, "https://example.com").await, "websiteUrl");
}

#[tokio::test]
async fn not_found_for_a_webchat_of_another_workspace() {
    let f = fixture().await;
    let id = webchat(&f, OTHER_WS, &[], true).await;
    let (status, out) = link(&f, &id, "https://example.com").await;
    assert_eq!(status, 404, "{out}");
}
