//! Port of `apps/builder/__tests__/connections-public-api.test.ts` (the
//! self-serve `api` / `webchat` part, P3.6-D2). Adapted: the mocked
//! `connectionStateService` / `connectionService` become a real migrated
//! database and real `api` / `webchat` connections; the procedures run
//! through `handlers::dispatch_as` with the token scopes.

mod handlers_common;

use handlers_common::{Fixture, OTHER_WS, WS, call_as, fixture};
use serde_json::{Value, json};

async fn run(
    f: &Fixture,
    route_id: &str,
    ws: i64,
    params: &[(&'static str, &str)],
    query: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    call_as(f, route_id, ws, None, params, query, body)
        .await
        .unwrap_or_else(|| panic!("{route_id} not handled"))
}

/// `POST /v1/connections` `{ provider: "api", config: { name } }`.
async fn create_api(f: &Fixture, ws: i64, name: &str) -> Value {
    let (status, body) = run(
        f,
        "connections.create",
        ws,
        &[],
        None,
        Some(json!({ "provider": "api", "config": { "name": name } })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body
}

fn connection_id(envelope: &Value) -> String {
    envelope["connection"]["id"]
        .as_str()
        .expect("id")
        .to_string()
}

// ts:210 describe("GET /v1/connections")

#[tokio::test]
async fn get_v1_connections_passes_filters_through_and_computes_page_count_from_the_total_count() {
    let f = fixture().await;
    for name in ["One", "Two", "Three"] {
        create_api(&f, WS, name).await;
    }
    let (status, page) = run(
        &f,
        "connections.list",
        WS,
        &[],
        Some("kind=channel&provider=api&channel=api&status=connected&page=1&perPage=2"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["pageCount"], 2);
    assert_eq!(page["data"].as_array().map(Vec::len), Some(2));
    let (_, none) = run(
        &f,
        "connections.list",
        WS,
        &[],
        Some("status=disconnected"),
        None,
    )
    .await;
    assert_eq!(none["data"], json!([]));
}

#[tokio::test]
async fn get_v1_connections_never_returns_fewer_than_1_page_even_when_count_is_0() {
    let f = fixture().await;
    let (status, page) = run(&f, "connections.list", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(page, json!({ "data": [], "pageCount": 1 }));
}

// ts:259 describe("GET /v1/connections/{id}")

#[tokio::test]
async fn get_v1_connections_id_throws_not_found_exception_when_the_connection_does_not_exist_in_this_workspace()
 {
    let f = fixture().await;
    let other = create_api(&f, OTHER_WS, "Other").await;
    let id = connection_id(&other);
    let (status, body) = run(&f, "connections.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "notFound");
    assert_eq!(body["message"], "Connection not found");
}

#[tokio::test]
async fn get_v1_connections_id_returns_the_resolved_resource_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let created = create_api(&f, WS, "Orders").await;
    let id = connection_id(&created);
    let (status, body) = run(&f, "connections.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body, created["connection"]);
    assert_eq!(body["strategy"], "self_serve");
    assert_eq!(
        body["capabilities"],
        json!({ "refreshable": false, "verifiable": true, "multiAccount": false })
    );
}

// ts:332 / ts:355 the lifecycle routes (disconnect, refresh, verify)

#[tokio::test]
async fn lifecycle_loads_the_connection_then_delegates_to_connection_service_disconnect() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    let (status, body) = run(&f, "connections.disconnect", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "disconnected");
    assert_eq!(body["statusReason"], "manual");
    assert!(body["disconnectedAt"].is_string());
}

#[tokio::test]
async fn lifecycle_loads_the_connection_then_delegates_to_connection_service_refresh() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    let (status, body) = run(
        &f,
        "connections.refresh",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], "connectionNotRefreshable");
    assert_eq!(body["defined"], true);
}

#[tokio::test]
async fn lifecycle_loads_the_connection_then_delegates_to_connection_service_verify() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    let (status, body) = run(
        &f,
        "connections.verify",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected");
    assert_eq!(body["statusReason"], Value::Null);
}

#[tokio::test]
async fn lifecycle_throws_not_found_before_calling_connection_service_when_the_connection_does_not_exist()
 {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, OTHER_WS, "Other").await);
    for route_id in [
        "connections.disconnect",
        "connections.refresh",
        "connections.verify",
    ] {
        let body = (route_id != "connections.disconnect").then(|| json!({}));
        let (status, out) = run(&f, route_id, WS, &[("id", &id)], None, body).await;
        assert_eq!(status, 404, "{route_id}");
        assert_eq!(out["code"], "notFound", "{route_id}");
    }
    // Still connected in its own workspace.
    let (_, other) = run(&f, "connections.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(other["status"], "connected");
}

// ts:365 refresh — provider error propagation

#[tokio::test]
async fn refresh_propagates_a_connection_inactive_failure_without_swallowing_it() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    run(&f, "connections.disconnect", WS, &[("id", &id)], None, None).await;
    let (status, body) = run(
        &f,
        "connections.refresh",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["code"], "connectionInactive");
    assert_eq!(body["defined"], true);
}

// ts:386 describe("POST /v1/connections")

#[tokio::test]
async fn post_v1_connections_creates_a_webchat_immediately_through_the_self_serve_connector() {
    let f = fixture().await;
    let (status, body) = run(
        &f,
        "connections.create",
        WS,
        &[],
        None,
        Some(json!({ "provider": "webchat", "config": { "name": "Site chat" } })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["session"], Value::Null);
    assert_eq!(body["secret"], Value::Null);
    let connection = &body["connection"];
    assert_eq!(connection["provider"], "webchat");
    assert_eq!(connection["kind"], "channel");
    assert_eq!(connection["channel"], "webchat");
    assert_eq!(connection["status"], "connected");
    assert_eq!(connection["displayName"], "Site chat");
    // The webchat row behind it is readable with the connection's source id.
    let source_id = connection["sourceId"].as_str().expect("sourceId");
    let (status, webchat) = run(&f, "webchats.get", WS, &[("id", source_id)], None, None).await;
    assert_eq!(status, 200, "{webchat}");
    assert_eq!(webchat["inboxId"], connection["inboxId"]);
}

#[tokio::test]
async fn post_v1_connections_returns_the_api_channel_token_exactly_once_in_the_connect_envelope() {
    let f = fixture().await;
    let body = create_api(&f, WS, "Orders").await;
    assert_eq!(body["session"], Value::Null);
    assert_eq!(body["secret"]["kind"], "api_channel_token");
    let token = body["secret"]["token"].as_str().expect("token");
    assert!(token.starts_with("cbx_api_"));
    assert_eq!(body["connection"]["provider"], "api");
    assert_eq!(
        body["connection"]["inboxId"],
        body["connection"]["sourceId"]
    );
    // A read of the connection carries no secret.
    let id = connection_id(&body);
    let (_, read) = run(&f, "connections.get", WS, &[("id", &id)], None, None).await;
    assert!(read.get("secret").is_none());
}

// ts:534 describe("POST /v1/connections/{id}/reconnect")

#[tokio::test]
async fn post_v1_connections_id_reconnect_throws_not_found_when_the_connection_does_not_exist() {
    let f = fixture().await;
    let (status, body) = run(
        &f,
        "connections.reconnect",
        WS,
        &[("id", "987654321")],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "notFound");
}

// ts:578 describe("PUT /v1/connections/{id}")

#[tokio::test]
async fn put_v1_connections_id_throws_not_found_when_the_connection_does_not_exist_in_this_workspace()
 {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, OTHER_WS, "Other").await);
    let (status, body) = run(
        &f,
        "connections.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "displayName": "Renamed" })),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "notFound");
}

#[tokio::test]
async fn put_v1_connections_id_renames_the_connection_scoped_to_the_token_s_workspace() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    let (status, body) = run(
        &f,
        "connections.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "displayName": "  Renamed  " })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["displayName"], "Renamed");
    let (_, read) = run(&f, "connections.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(read["displayName"], "Renamed");
}

// ts:712 the dedicated-only googleAds provider

#[tokio::test]
async fn public_connections_api_post_v1_connections_rejects_google_ads_before_any_connect_work() {
    let f = fixture().await;
    let (status, body) = run(
        &f,
        "connections.create",
        WS,
        &[],
        None,
        Some(json!({ "provider": "googleAds" })),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["code"], "connectionProviderDedicatedOnly");
    assert_eq!(body["defined"], true);
    let (_, list) = run(&f, "connections.list", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));
}

// The token-scope rules of `connectionScope` (`assertTokenScopeForProvider`,
// `resolveListKind`), exercised by every route above.

#[tokio::test]
async fn token_scope_a_single_kind_token_is_restricted_to_its_kind() {
    let f = fixture().await;
    let id = connection_id(&create_api(&f, WS, "Orders").await);
    let integrations: Option<&[&str]> = Some(&["integrations"]);
    let (status, list) = call_as(&f, "connections.list", WS, integrations, &[], None, None)
        .await
        .expect("handled");
    assert_eq!(status, 200);
    assert_eq!(list["data"], json!([]));
    let (status, body) = call_as(
        &f,
        "connections.get",
        WS,
        integrations,
        &[("id", &id)],
        None,
        None,
    )
    .await
    .expect("handled");
    assert_eq!(status, 403);
    assert_eq!(body["code"], "FORBIDDEN");
    assert_eq!(
        body["message"],
        "Token is not authorized for the 'channels' scope"
    );
    let (status, _) = call_as(
        &f,
        "connections.create",
        WS,
        integrations,
        &[],
        None,
        Some(json!({ "provider": "api", "config": { "name": "A" } })),
    )
    .await
    .expect("handled");
    assert_eq!(status, 403);
}
