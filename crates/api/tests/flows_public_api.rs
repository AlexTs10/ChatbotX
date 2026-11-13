//! Port of `apps/builder/__tests__/flows-public-api.test.ts` (the router
//! scope and `POST /v1/flows`, P3.6-D2). Adapted: the mocked
//! `flowService` / `flowVersionService` become a real migrated database;
//! the asserted service calls become the stored `Flow` / `FlowVersion`
//! rows and the response.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_core::flow_config::nodes::send_message_node_default_fn;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

async fn create(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "flows.create", WS, &[], None, Some(body)).await
}

fn flow_id(body: &Value) -> i64 {
    body["id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("numeric id")
}

async fn count(f: &Fixture, sql: &'static str, id: i64) -> i64 {
    scalar(f, sql, id)
        .await
        .and_then(|v| v.as_i64())
        .unwrap_or_default()
}

async fn versions(f: &Fixture, flow: i64) -> i64 {
    count(
        f,
        r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "flowId" = ?1"#,
        flow,
    )
    .await
}

async fn draft_nodes(f: &Fixture, flow: i64) -> Vec<Value> {
    let text = scalar(
        f,
        r#"SELECT "nodes" FROM "FlowVersion" WHERE "flowId" = ?1 AND "isDraft" = 1"#,
        flow,
    )
    .await
    .and_then(|v| v.as_str().map(str::to_string))
    .expect("draft version");
    serde_json::from_str(&text).expect("nodes JSON")
}

#[tokio::test]
async fn registers_the_flows_public_router_under_the_automation_scope() {
    let route = route_by_id("flows.create").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["automation"]));
}

#[tokio::test]
async fn post_v1_flows_route_metadata() {
    let route = route_by_id("flows.create").expect("route");
    assert_eq!((route.method, route.path), ("POST", "/v1/flows"));
    let f = fixture().await;
    let (status, body) = create(&f, json!({ "name": "New flow", "folderId": null })).await;
    assert_eq!(status, 201, "{body}");
}

#[tokio::test]
async fn post_v1_flows_delegates_to_flow_service_create_draft() {
    let f = fixture().await;
    let (status, body) = create(&f, json!({ "name": "  New flow  ", "folderId": null })).await;
    assert_eq!(status, 201, "{body}");
    assert!(body.get("nodeIds").is_none());
    let flow = flow_id(&body);
    assert_eq!(
        scalar(&f, r#"SELECT "name" FROM "Flow" WHERE "id" = ?1"#, flow).await,
        Some(json!("New flow"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
            flow
        )
        .await,
        Some(Value::Null)
    );
    assert_eq!(versions(&f, flow).await, 1);
    // The default draft graph: one start "Send Message #1" node.
    let nodes = draft_nodes(&f, flow).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["type"], "sendMessage");
    assert_eq!(nodes[0]["data"]["isStartNode"], true);
}

#[tokio::test]
async fn post_v1_flows_spec_content_create_draft_receives_a_positioned_graph_and_start_node_id_publish_is_not_called()
 {
    let f = fixture().await;
    let (status, body) = create(
        &f,
        json!({
            "name": "Spec flow",
            "folderId": null,
            "spec": { "formatVersion": 1, "name": "Spec flow", "steps": [{ "type": "send", "text": "Hi" }] },
        }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let flow = flow_id(&body);
    assert_eq!(versions(&f, flow).await, 1);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
            flow
        )
        .await,
        Some(Value::Null)
    );
    let nodes = draft_nodes(&f, flow).await;
    assert!(!nodes.is_empty());
    assert!(nodes[0]["position"].is_object());
    let start = count(
        &f,
        r#"SELECT "startNodeId" FROM "FlowVersion" WHERE "flowId" = ?1"#,
        flow,
    )
    .await;
    assert_eq!(
        start.to_string(),
        nodes[0]["id"].as_str().unwrap_or_default()
    );
}

#[tokio::test]
async fn post_v1_flows_raw_graph_content_create_draft_receives_normalized_positions_edges_and_start_node_id()
 {
    let f = fixture().await;
    let (status, body) = create(
        &f,
        json!({
            "name": "New flow",
            "folderId": null,
            "nodes": [
                { "id": "n1", "type": "sendMessage", "data": {} },
                { "id": "n2", "type": "sendMessage", "data": {} },
            ],
            "edges": [{ "source": "n1", "target": "n2" }],
        }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let flow = flow_id(&body);
    let nodes = draft_nodes(&f, flow).await;
    let (n1, n2) = (nodes[0]["id"].clone(), nodes[1]["id"].clone());
    assert_ne!(n1, "n1");
    assert_ne!(n2, "n2");
    assert_eq!(nodes[0]["position"], json!({ "x": 100, "y": 100 }));
    assert_eq!(body["nodeIds"], json!({ "n1": n1, "n2": n2 }));
    assert_eq!(versions(&f, flow).await, 1);
}

#[tokio::test]
async fn post_v1_flows_publish_true_validates_the_graph_and_creates_it_atomically_via_flow_service_create_published_not_create_draft_publish()
 {
    let f = fixture().await;
    let node = send_message_node_default_fn(&json!({
        "nodeProps": { "id": "n1" },
        "dataProps": {},
        "detailProps": {},
    }));
    let (status, body) = create(
        &f,
        json!({ "name": "New flow", "folderId": null, "nodes": [node], "edges": [], "publish": true }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let flow = flow_id(&body);
    assert_ne!(body["nodeIds"]["n1"], "n1");
    assert!(body["nodeIds"]["n1"].is_string());
    // One draft and one published (latest) version, both referenced.
    assert_eq!(versions(&f, flow).await, 2);
    assert!(
        scalar(
            &f,
            r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
            flow
        )
        .await
        .is_some_and(|v| v.is_i64())
    );
    assert!(
        scalar(
            &f,
            r#"SELECT "draftVersionId" FROM "Flow" WHERE "id" = ?1"#,
            flow
        )
        .await
        .is_some_and(|v| v.is_i64())
    );
    assert_eq!(
        count(
            &f,
            r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "flowId" = ?1 AND "isLatest" = 1 AND "isDraft" = 0"#,
            flow,
        )
        .await,
        1
    );
}

#[tokio::test]
async fn post_v1_flows_publish_true_rejects_an_invalid_node_before_create_draft_is_called() {
    let f = fixture().await;
    let (status, body) = create(
        &f,
        json!({
            "name": "New flow",
            "folderId": null,
            "nodes": [{ "id": "n1", "type": "sendMessage", "data": {} }],
            "edges": [],
            "publish": true,
        }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "invalidRequestData");
    assert_eq!(
        count(
            &f,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        0
    );
}
