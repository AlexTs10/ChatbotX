//! Port of `apps/builder/__tests__/flows-public-api.test.ts`, the
//! `POST /v1/flows/{id}/publish`, `POST /v1/flows/validate`,
//! `PUT /v1/flows/{id}/draft`, `GET /v1/flows/{id}/versions` and
//! "mixed body rejection" blocks (P3.6-D2). Adapted: the mocked
//! `flowVersionService` / `getFlowAuthoringContext` become a real migrated
//! database; the asserted service calls become the responses and stored
//! versions; the request-schema cases call the handler's request parsers.
//! Not applicable: "compiles a pipeline step through publishFlowSchema using
//! the authoring context" (the pipeline authoring catalog is not loaded; the
//! pipelines domain is P3.6-D1p).

mod handlers_common;

use chatbotx_api::handlers::flows::{GraphBody, publish_flow_request, update_draft_flow_request};
use chatbotx_core::flow_config::nodes::send_message_node_default_fn;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Map, Value, json};

fn spec(name: &str, extra: Value, steps: Value) -> Value {
    let mut out = json!({ "formatVersion": 1, "name": name, "steps": steps });
    if let (Some(map), Some(extra)) = (out.as_object_mut(), extra.as_object()) {
        map.extend(extra.clone());
    }
    out
}

fn hello_spec() -> Value {
    spec(
        "Spec flow",
        json!({}),
        json!([{ "type": "send", "text": "Hello!" }]),
    )
}

async fn draft_flow(f: &Fixture) -> String {
    let (status, out) = call(
        f,
        "flows.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Flow", "folderId": null })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    out["id"].as_str().expect("id").to_string()
}

async fn version_nodes(f: &Fixture, flow: &str, draft: bool) -> Vec<Value> {
    let sql = if draft {
        r#"SELECT "nodes" FROM "FlowVersion" WHERE "flowId" = ?1 AND "isDraft" = 1"#
    } else {
        r#"SELECT "nodes" FROM "FlowVersion" WHERE "flowId" = ?1 AND "isDraft" = 0 AND "isLatest" = 1"#
    };
    let text = scalar(f, sql, flow.parse().expect("id"))
        .await
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("version");
    serde_json::from_str(&text).expect("nodes JSON")
}

async fn published_count(f: &Fixture, flow: &str) -> i64 {
    scalar(
        f,
        r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "flowId" = ?1 AND "isDraft" = 0"#,
        flow.parse().expect("id"),
    )
    .await
    .and_then(|v| v.as_i64())
    .unwrap_or_default()
}

async fn validate(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "flows.validate", WS, &[], None, Some(body)).await
}

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

// describe("POST /v1/flows/{id}/publish")

#[tokio::test]
async fn post_v1_flows_id_publish_delegates_to_flow_version_service_publish() {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let node = send_message_node_default_fn(&json!({}));
    let (status, out) = call(
        &f,
        "flows.publish",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "nodes": [node.clone()], "edges": [] })),
    )
    .await;
    assert_eq!((status, out), (204, Value::Null));
    assert_eq!(version_nodes(&f, &id, false).await, vec![node]);
}

#[tokio::test]
async fn compiles_a_spec_input_into_nodes_edges_before_publishing() {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let (status, out) = call(
        &f,
        "flows.publish",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "spec": hello_spec() })),
    )
    .await;
    assert_eq!(status, 204, "{out}");
    let nodes = version_nodes(&f, &id, false).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["type"], json!("sendMessage"));
    let edges = scalar(
        &f,
        r#"SELECT "edges" FROM "FlowVersion" WHERE "flowId" = ?1 AND "isDraft" = 0"#,
        id.parse().expect("id"),
    )
    .await;
    assert_eq!(edges, Some(json!("[]")));
}

#[tokio::test]
async fn throws_flow_authoring_exception_not_a_raw_500_when_the_compiled_spec_fails_channel_refinement()
 {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let body = json!({
        "spec": spec(
            "Spec flow",
            json!({ "channel": "instagram" }),
            json!([{ "type": "send", "imageUrl": "https://example.com/a.png", "buttons": [{ "text": "Yes" }] }]),
        ),
    });
    let (status, out) = call(&f, "flows.publish", WS, &[("id", &id)], None, Some(body)).await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["code"], json!("invalidRequestData"));
    let path = out["data"][0]["path"]
        .as_str()
        .expect("authoring error path");
    assert!(path.starts_with("steps["), "{path}");
    assert_eq!(published_count(&f, &id).await, 0);
}

// describe("POST /v1/flows/validate")

#[tokio::test]
async fn compiles_a_spec_and_returns_the_graph_without_persisting_anything() {
    let f = fixture().await;
    let (status, out) = validate(&f, json!({ "spec": hello_spec() })).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["nodes"].as_array().map(Vec::len), Some(1));
    assert_eq!(out["nodes"][0]["type"], json!("sendMessage"));
    assert_eq!(out["edges"], json!([]));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn compiles_a_call_phone_number_button_from_call() {
    let f = fixture().await;
    let body = json!({
        "spec": spec(
            "Call flow",
            json!({ "channel": "messenger" }),
            json!([{ "type": "send", "text": "Need help?", "buttons": [{ "text": "Call us", "call": "+14155552671" }] }]),
        ),
    });
    let (status, out) = validate(&f, body).await;
    assert_eq!(status, 200, "{out}");
    let button = &out["nodes"][0]["data"]["details"]["steps"][0]["buttons"][0];
    assert_eq!(button["buttonType"], json!("callPhoneNumber"));
    assert_eq!(button["beforeStep"]["phoneNumber"], json!("+14155552671"));
    assert_eq!(button["steps"], json!([]));
    assert_eq!(out["edges"], json!([]));
}

#[tokio::test]
async fn compiles_a_copy_text_button_from_copy_on_telegram() {
    let f = fixture().await;
    let body = json!({
        "spec": spec(
            "PIX flow",
            json!({ "channel": "telegram" }),
            json!([{ "type": "send", "text": "Pague com PIX", "buttons": [{ "text": "Copiar PIX", "copy": "{{pix_code}}" }] }]),
        ),
    });
    let (status, out) = validate(&f, body).await;
    assert_eq!(status, 200, "{out}");
    let button = &out["nodes"][0]["data"]["details"]["steps"][0]["buttons"][0];
    assert_eq!(button["buttonType"], json!("copyText"));
    assert_eq!(button["beforeStep"]["stepType"], json!("copyText"));
    assert_eq!(button["beforeStep"]["text"], json!("{{pix_code}}"));
    assert_eq!(button["steps"], json!([]));
    assert_eq!(out["edges"], json!([]));
}

#[tokio::test]
async fn rejects_a_call_number_that_is_not_a_real_international_number() {
    let f = fixture().await;
    let body = json!({
        "spec": spec(
            "Call flow",
            json!({}),
            json!([{ "type": "send", "text": "Need help?", "buttons": [{ "text": "Call us", "call": "+9991234567" }] }]),
        ),
    });
    let (status, out) = validate(&f, body).await;
    assert_eq!(status, 422, "{out}");
}

#[tokio::test]
async fn reports_an_unknown_pipeline_with_its_spec_path() {
    let f = fixture().await;
    let body = json!({
        "spec": spec(
            "Pipeline flow",
            json!({}),
            json!([{ "type": "pipeline", "action": "removeFromPipeline", "pipeline": "missing" }]),
        ),
    });
    let (status, out) = validate(&f, body).await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["data"][0]["path"], json!("steps[0].pipeline"));
    assert_eq!(out["data"][0]["code"], json!("unknownPipeline"));
}

#[tokio::test]
async fn throws_flow_authoring_exception_for_an_unknown_template_name() {
    let f = fixture().await;
    let body = json!({
        "spec": spec(
            "Spec flow",
            json!({}),
            json!([{ "type": "sendTemplate", "templateName": "does_not_exist" }]),
        ),
    });
    let (status, out) = validate(&f, body).await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["data"][0]["code"], json!("unknownTemplate"));
}

// describe("PUT /v1/flows/{id}/draft")

#[tokio::test]
async fn put_v1_flows_id_draft_delegates_to_flow_version_service_update_draft_by_flow_id() {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let (status, out) = call(
        &f,
        "flows.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "nodes": [], "edges": [] })),
    )
    .await;
    assert_eq!((status, out), (204, Value::Null));
    assert!(version_nodes(&f, &id, true).await.is_empty());
}

#[tokio::test]
async fn compiles_a_spec_input_into_nodes_edges_before_updating_the_draft() {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let (status, out) = call(
        &f,
        "flows.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "spec": hello_spec() })),
    )
    .await;
    assert_eq!(status, 204, "{out}");
    let nodes = version_nodes(&f, &id, true).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["type"], json!("sendMessage"));
    assert_eq!(published_count(&f, &id).await, 0);
}

// describe("GET /v1/flows/{id}/versions")

#[tokio::test]
async fn get_v1_flows_id_versions_delegates_to_flow_version_service_list() {
    let f = fixture().await;
    let id = draft_flow(&f).await;
    let (status, _) = call(
        &f,
        "flows.publish",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "spec": hello_spec() })),
    )
    .await;
    assert_eq!(status, 204);
    let (status, out) = call(&f, "flows.versions", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{out}");
    let current = scalar(
        &f,
        r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
        id.parse().expect("id"),
    )
    .await
    .and_then(|v| v.as_i64())
    .expect("current version");
    assert_eq!(out["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(out["data"][0]["id"], json!(current.to_string()));
}

// describe("publishFlowRequest / updateDraftFlowRequest — mixed body rejection")

#[test]
fn publish_flow_request_rejects_a_body_carrying_both_nodes_edges_and_spec() {
    let body = obj(json!({ "nodes": [], "edges": [], "spec": hello_spec() }));
    assert!(publish_flow_request(&body).is_err());
}

#[test]
fn publish_flow_request_still_accepts_a_spec_only_body() {
    let body = obj(json!({ "spec": hello_spec() }));
    assert!(matches!(
        publish_flow_request(&body),
        Ok(GraphBody::Spec(_))
    ));
}

#[test]
fn publish_flow_request_still_accepts_a_graph_only_body() {
    let body = obj(json!({ "nodes": [], "edges": [] }));
    assert!(matches!(
        publish_flow_request(&body),
        Ok(GraphBody::Graph { .. })
    ));
}

#[test]
fn update_draft_flow_request_rejects_a_body_carrying_both_nodes_edges_and_spec() {
    let body = obj(json!({ "nodes": [], "edges": [], "spec": hello_spec() }));
    assert!(update_draft_flow_request(&body).is_err());
}

#[test]
fn update_draft_flow_request_still_accepts_a_spec_only_body() {
    let body = obj(json!({ "spec": hello_spec() }));
    assert!(matches!(
        update_draft_flow_request(&body),
        Ok(GraphBody::Spec(_))
    ));
}

#[test]
fn update_draft_flow_request_still_accepts_a_graph_only_body() {
    let body = obj(json!({ "nodes": [], "edges": [] }));
    assert!(matches!(
        update_draft_flow_request(&body),
        Ok(GraphBody::Graph { .. })
    ));
}

#[test]
fn publish_flow_request_composed_with_the_route_s_id_param_still_accepts_spec_only_and_graph_only_bodies()
 {
    let spec_only = obj(json!({ "id": "1", "spec": hello_spec() }));
    assert!(publish_flow_request(&spec_only).is_ok());
    let graph_only = obj(json!({ "id": "1", "nodes": [], "edges": [] }));
    assert!(publish_flow_request(&graph_only).is_ok());
    let mixed = obj(json!({ "id": "1", "nodes": [], "edges": [], "spec": hello_spec() }));
    assert!(publish_flow_request(&mixed).is_err());
}
