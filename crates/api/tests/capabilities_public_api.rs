//! Ported from `apps/builder/__tests__/capabilities-public-api.test.ts`
//! (Q.D11 slice 5). The TS test captures the oRPC procedures and mocks
//! `getCapabilities`; here the routes run on a migrated SQLite file
//! (`handlers_common`) and the scope test reads the route table.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use handlers_common::{WS, call, fixture};
use serde_json::json;

#[test]
fn registers_the_capabilities_public_router_under_the_contacts_scope() {
    for id in ["capabilities.get", "schemas.flowSpec"] {
        let route = route_by_id(id).expect("route");
        assert!(
            matches!(route.auth, RouteAuth::WorkspaceToken(&["contacts"])),
            "{id}"
        );
    }
}

#[tokio::test]
async fn delegates_with_the_workspace_and_the_default_include_set() {
    let fx = fixture().await;
    let (status, body) = call(&fx, "capabilities.get", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "botFields",
            "customFields",
            "flowSpec",
            "flows",
            "inboxes",
            "sequences",
            "tags",
            "templates"
        ]
    );
    assert!(body["flowSpec"].get("selectedChannelPolicy").is_none());
}

#[tokio::test]
async fn forwards_a_parsed_include_list() {
    let fx = fixture().await;
    let (status, body) = call(
        &fx,
        "capabilities.get",
        WS,
        &[],
        Some("include=%20tags%20,flows,"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "tags": [], "flows": [] }));
}

#[tokio::test]
async fn forwards_a_requested_channel_policy() {
    let fx = fixture().await;
    let (status, body) = call(
        &fx,
        "capabilities.get",
        WS,
        &[],
        Some("include=flowSpec&channel=tiktok"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["flowSpec"]["selectedChannelPolicy"]["unsupportedSteps"]
            .as_array()
            .unwrap()
            .contains(&json!("sendCard"))
    );
}

#[tokio::test]
async fn rejects_an_unknown_include_or_channel() {
    // Own test (parity `p1-04/07-capabilities`: 422 at `include.1` / `channel`).
    let fx = fixture().await;
    let (status, body) = call(
        &fx,
        "capabilities.get",
        WS,
        &[],
        Some("include=inboxes,secrets"),
        None,
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["data"]["issues"][0]["path"], json!(["include", 1]));
    let (status, body) = call(&fx, "capabilities.get", WS, &[], Some("channel=fax"), None).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["data"]["issues"][0]["path"], json!(["channel"]));
}

#[tokio::test]
async fn returns_a_json_schema_object_describing_the_flow_spec_dsl() {
    let fx = fixture().await;
    let (status, body) = call(&fx, "schemas.flowSpec", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert!(body.get("type").is_some());
    let text = body.to_string();
    assert!(text.contains("formatVersion"));
    assert!(text.contains("steps"));
}
