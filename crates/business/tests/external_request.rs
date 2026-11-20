//! Port of `packages/business/__tests__/external-request-service.test.ts` and
//! `external-request-redirect.integration.test.ts` (Q.P6-U4). Adapted: the
//! stubbed `fetch` is a scripted [`Hop`], the mocked `checkSsrfSafety` a DoH
//! stub that resolves every name to a public IP (an IP literal in a blocked
//! range is still refused by the real guard); the mocked contact / custom
//! field services are real rows. "cancels a redirect response body" is
//! not-applicable (the transport never reads a redirect body). The redirect
//! integration test runs on the scripted hop, not a loopback server.

mod common;

use std::collections::VecDeque;
use std::sync::Mutex;

use chatbotx_business::external_request::{
    Hop, HopReply, HopRequest, execute_and_map_with, execute_with,
};
use chatbotx_business::net::ssrf_guard::{DohFetch, DohReply};
use chatbotx_db::rusqlite::params;
use common::{
    TestDb, WS, custom_field, custom_field_value, custom_field_values, setup, two_workspaces,
};
use futures::future::BoxFuture;
use serde_json::{Value, json};

struct PublicDoh;

impl DohFetch for PublicDoh {
    async fn get(&self, _url: &str) -> Result<DohReply, String> {
        Ok(DohReply {
            status: 200,
            body: json!({ "Answer": [{ "type": 1, "data": "93.184.216.34" }] }).to_string(),
        })
    }
}

/// Answers in order; with `redirect_forever` every hop redirects to
/// `<url>/next`.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<HopReply>>,
    seen: Mutex<Vec<HopRequest>>,
    redirect_forever: bool,
}

impl Script {
    fn new(replies: Vec<HopReply>) -> Script {
        Script {
            replies: Mutex::new(replies.into()),
            ..Script::default()
        }
    }

    fn seen(&self) -> Vec<HopRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl Hop for Script {
    fn send(&self, request: &HopRequest) -> BoxFuture<'_, Result<HopReply, String>> {
        self.seen.lock().unwrap().push(request.clone());
        let reply = if self.redirect_forever {
            Some(redirect(&format!("{}/next", request.url), 302))
        } else {
            self.replies.lock().unwrap().pop_front()
        };
        Box::pin(async move { reply.ok_or_else(|| "no scripted reply".to_owned()) })
    }
}

fn ok(body: &str) -> HopReply {
    status(200, body)
}

fn status(code: u16, body: &str) -> HopReply {
    HopReply {
        status: code,
        headers: vec![],
        body: body.into(),
    }
}

fn redirect(to: &str, code: u16) -> HopReply {
    HopReply {
        status: code,
        headers: vec![("location".into(), to.into())],
        body: String::new(),
    }
}

fn get(url: &str) -> Value {
    json!({ "method": "GET", "url": url, "headers": [] })
}

const DATA: &str = "https://api.example.com/data";
const FINAL: &str = "https://api.example.com/final";

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (10, 'Ada', ?1)"#,
            params![WS],
        )?;
        custom_field(tx, 31, WS, "user_id", "shortText")?;
        custom_field_value(tx, 41, 10, 31, "x")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn throws_when_the_url_is_ssrf_unsafe_without_calling_fetch() {
    let db = db().await;
    let hop = Script::new(vec![]);
    let err = execute_with(
        &db,
        &hop,
        &PublicDoh,
        &get("http://169.254.169.254/"),
        WS,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), Some("ssrfBlocked"));
    assert!(hop.seen().is_empty());
}

#[tokio::test]
async fn follows_a_redirect_to_a_safe_target_and_returns_its_response() {
    let db = db().await;
    let hop = Script::new(vec![redirect(FINAL, 302), ok("{}")]);
    let result = execute_with(&db, &hop, &PublicDoh, &get(DATA), WS, None)
        .await
        .unwrap();
    assert_eq!(result.status_code, 200);
    let seen = hop.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].url, FINAL);
}

#[tokio::test]
async fn switches_a_post_302_redirect_to_get_without_body_headers() {
    let db = db().await;
    let hop = Script::new(vec![redirect(FINAL, 302), ok("OK")]);
    let input = json!({
        "method": "POST", "url": DATA,
        "headers": [
            { "key": "Authorization", "value": "Bearer secret" },
            { "key": "Content-Language", "value": "en" },
            { "key": "X-Request-Id", "value": "request-1" },
        ],
        "body": { "bodyType": "json", "jsonBody": "{\"name\":\"Ada\"}" },
    });
    let result = execute_with(&db, &hop, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    assert_eq!(
        (result.status_code, result.response_body.as_str()),
        (200, "OK")
    );
    let seen = hop.seen();
    let (initial, redirected) = (&seen[0], &seen[1]);
    assert_eq!(redirected.method, "GET");
    assert_eq!(redirected.body, None);
    assert_eq!(redirected.header("authorization"), Some("Bearer secret"));
    for header in [
        "content-type",
        "content-language",
        "content-length",
        "content-encoding",
        "content-location",
    ] {
        assert_eq!(redirected.header(header), None, "{header}");
    }
    assert_eq!(redirected.header("x-request-id"), Some("request-1"));
    assert_eq!(initial.body.as_deref(), Some("{\"name\":\"Ada\"}"));
    assert_eq!(initial.header("content-type"), Some("application/json"));
    assert_eq!(initial.header("content-language"), Some("en"));
}

#[tokio::test]
async fn uses_301_and_303_redirect_semantics() {
    for (code, method, expected) in [
        (301, "POST", "GET"),
        (303, "PUT", "GET"),
        (303, "GET", "GET"),
    ] {
        let db = db().await;
        let hop = Script::new(vec![redirect(FINAL, code), ok("{}")]);
        let mut input = json!({ "method": method, "url": DATA, "headers": [] });
        if method != "GET" {
            input["body"] = json!({ "bodyType": "json", "jsonBody": "{\"name\":\"Ada\"}" });
        }
        execute_with(&db, &hop, &PublicDoh, &input, WS, None)
            .await
            .unwrap();
        let redirected = &hop.seen()[1];
        assert_eq!(redirected.method, expected, "{code} {method}");
        assert_eq!(redirected.body, None, "{code} {method}");
    }
}

#[tokio::test]
async fn preserves_post_method_and_body_across_307_and_308() {
    for code in [307, 308] {
        let db = db().await;
        let hop = Script::new(vec![redirect(FINAL, code), ok("{}")]);
        let input = json!({
            "method": "POST", "url": DATA, "headers": [],
            "body": { "bodyType": "json", "jsonBody": "{\"name\":\"Ada\"}" },
        });
        execute_with(&db, &hop, &PublicDoh, &input, WS, None)
            .await
            .unwrap();
        let redirected = &hop.seen()[1];
        assert_eq!(redirected.method, "POST");
        assert_eq!(redirected.body.as_deref(), Some("{\"name\":\"Ada\"}"));
    }
}

#[tokio::test]
async fn drops_authorization_only_on_cross_origin_redirects() {
    let db = db().await;
    let input = json!({ "method": "GET", "url": DATA,
        "headers": [{ "key": "Authorization", "value": "Bearer secret" }] });
    let cross = Script::new(vec![
        redirect("https://redirect.example.net/final", 302),
        ok("{}"),
    ]);
    execute_with(&db, &cross, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    let same = Script::new(vec![redirect(FINAL, 302), ok("{}")]);
    execute_with(&db, &same, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    assert_eq!(cross.seen()[1].header("authorization"), None);
    assert_eq!(
        same.seen()[1].header("authorization"),
        Some("Bearer secret")
    );
}

#[tokio::test]
async fn blocks_a_redirect_whose_target_is_ssrf_unsafe() {
    let db = db().await;
    let hop = Script::new(vec![redirect("http://169.254.169.254/", 302)]);
    let err = execute_with(&db, &hop, &PublicDoh, &get(DATA), WS, None)
        .await
        .unwrap_err();
    assert_eq!(
        err.message(),
        "This URL is not allowed for external requests"
    );
    assert_eq!(hop.seen().len(), 1);
}

#[tokio::test]
async fn stops_following_redirects_once_the_limit_is_exceeded() {
    let db = db().await;
    let hop = Script {
        redirect_forever: true,
        ..Script::default()
    };
    let err = execute_with(&db, &hop, &PublicDoh, &get(DATA), WS, None)
        .await
        .unwrap_err();
    assert!(err.message().contains("Too many redirects"));
    assert_eq!(hop.seen().len(), 6);
}

#[tokio::test]
async fn get_builds_a_request_with_no_body() {
    let db = db().await;
    let hop = Script::new(vec![ok("{}")]);
    execute_with(&db, &hop, &PublicDoh, &get(DATA), WS, None)
        .await
        .unwrap();
    let request = &hop.seen()[0];
    assert_eq!(request.url, DATA);
    assert_eq!(request.method, "GET");
    assert_eq!(request.body, None);
}

#[tokio::test]
async fn post_with_json_body_sets_content_type_application_json() {
    let db = db().await;
    let hop = Script::new(vec![ok("{}")]);
    let input = json!({ "method": "POST", "url": DATA, "headers": [],
        "body": { "bodyType": "json", "jsonBody": "{\"a\":1}" } });
    execute_with(&db, &hop, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    let request = &hop.seen()[0];
    assert_eq!(request.body.as_deref(), Some("{\"a\":1}"));
    assert_eq!(request.header("Content-Type"), Some("application/json"));
}

#[tokio::test]
async fn post_with_form_encoded_body_url_encodes_fields() {
    let db = db().await;
    let hop = Script::new(vec![ok("{}")]);
    let input = json!({ "method": "POST", "url": DATA, "headers": [],
        "body": { "bodyType": "formEncoded", "formFields": [{ "key": "name", "value": "Ada Lovelace" }] } });
    execute_with(&db, &hop, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    let request = &hop.seen()[0];
    assert_eq!(request.body.as_deref(), Some("name=Ada+Lovelace"));
    assert_eq!(
        request.header("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
}

#[tokio::test]
async fn all_contact_data_body_requires_a_contact_id() {
    let db = db().await;
    let hop = Script::new(vec![ok("{}")]);
    let input = json!({ "method": "POST", "url": DATA, "headers": [],
        "body": { "bodyType": "allContactData" } });
    let err = execute_with(&db, &hop, &PublicDoh, &input, WS, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some("contactRequired"));
    assert!(hop.seen().is_empty());
}

#[tokio::test]
async fn all_contact_data_body_includes_contact_and_custom_fields() {
    let db = db().await;
    let hop = Script::new(vec![ok("{}")]);
    let input = json!({ "method": "POST", "url": DATA, "headers": [],
        "body": { "bodyType": "allContactData" } });
    execute_with(&db, &hop, &PublicDoh, &input, WS, Some(10))
        .await
        .unwrap();
    let body: Value = serde_json::from_str(hop.seen()[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(body["contact"]["id"], "10");
    assert_eq!(body["contact"]["firstName"], "Ada");
    assert_eq!(
        body["customFields"],
        json!([{ "customFieldId": "31", "value": "x" }])
    );
}

#[tokio::test]
async fn execute_and_map_returns_without_writing_fields_on_a_non_2xx_response() {
    let db = db().await;
    let hop = Script::new(vec![status(500, "")]);
    let mapping = json!([{ "jsonPath": "id", "outputFieldId": "31" }]);
    let result = execute_and_map_with(&db, &hop, &PublicDoh, WS, 10, &get(DATA), &mapping)
        .await
        .unwrap();
    assert_eq!(result.status_code, 500);
    assert_eq!(
        custom_field_values(&db, 10).await,
        vec![(31, "x".to_owned())]
    );
}

#[tokio::test]
async fn execute_and_map_writes_mapped_json_response_fields() {
    let db = db().await;
    let hop = Script::new(vec![ok(r#"{"user":{"id":"abc-123"}}"#)]);
    let mapping = json!([{ "jsonPath": "user.id", "outputFieldId": "31" }]);
    execute_and_map_with(&db, &hop, &PublicDoh, WS, 10, &get(DATA), &mapping)
        .await
        .unwrap();
    assert_eq!(
        custom_field_values(&db, 10).await,
        vec![(31, "abc-123".to_owned())]
    );
}

#[tokio::test]
async fn execute_and_map_returns_the_raw_result_when_the_response_is_not_json() {
    let db = db().await;
    let hop = Script::new(vec![ok("not json")]);
    let mapping = json!([{ "jsonPath": "id", "outputFieldId": "31" }]);
    let result = execute_and_map_with(&db, &hop, &PublicDoh, WS, 10, &get(DATA), &mapping)
        .await
        .unwrap();
    assert_eq!(result.response_body, "not json");
    assert_eq!(
        custom_field_values(&db, 10).await,
        vec![(31, "x".to_owned())]
    );
}

/// `external-request-redirect.integration.test.ts`: an Apps Script web app
/// answers a POST with a 302 to its content URL, read with a GET.
#[tokio::test]
async fn follows_an_apps_script_style_post_redirect_with_a_get() {
    let db = db().await;
    let content = "https://script.googleusercontent.com/macros/echo?user_content_key=k";
    let hop = Script::new(vec![redirect(content, 302), ok(r#"{"ok":true}"#)]);
    let input = json!({ "method": "POST", "url": "https://script.google.com/macros/s/x/exec",
        "headers": [], "body": { "bodyType": "json", "jsonBody": "{\"a\":1}" } });
    let result = execute_with(&db, &hop, &PublicDoh, &input, WS, None)
        .await
        .unwrap();
    assert_eq!(result.response_body, r#"{"ok":true}"#);
    let seen = hop.seen();
    assert_eq!(
        (seen[1].method.as_str(), seen[1].url.as_str()),
        ("GET", content)
    );
    assert_eq!(seen[1].body, None);
}
