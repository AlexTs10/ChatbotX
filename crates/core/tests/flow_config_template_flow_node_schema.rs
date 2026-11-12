//! Port of `packages/flow-config/__tests__/template-flow-node-schema.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn node(details: Value) -> Value {
    json!({ "id": "1", "type": "performAction", "data": { "details": details } })
}

#[test]
fn templateflownodeschema_a_draft_with_half_written_steps_travels() {
    assert!(
        fc::template_flow_node_schema()
            .safe_parse(&node(json!({ "steps": [{ "stepType": "x" }] })))
            .success
    );
}

#[test]
fn templateflownodeschema_steps_that_are_not_a_list_are_refused_the_viewer_walks_them() {
    assert!(
        !fc::template_flow_node_schema()
            .safe_parse(&node(json!({ "steps": "unfinished" })))
            .success
    );
}

#[test]
fn templateflownodeschema_a_node_without_details_is_refused() {
    assert!(
        !fc::template_flow_node_schema()
            .safe_parse(&json!({ "id": "1", "type": "x", "data": {} }))
            .success
    );
}
