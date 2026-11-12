//! Port of `packages/flow-config/__tests__/flow-action-target.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn make_node(id: &str, detail_props: Value) -> Value {
    fc::send_message_node_default_fn(
        &json!({ "nodeProps": { "id": id }, "detailProps": detail_props }),
    )
}

fn target_json(target: Option<fc::FlowActionTarget>) -> Value {
    match target {
        Some(t) => {
            json!({ "details": t.details, "nodeId": t.node_id, "targetType": t.target_type })
        }
        None => Value::Null,
    }
}

#[test]
fn resolveflowactiontarget_resolves_a_button_owned_by_a_step() {
    let button = fc::button_step_default_fn(&json!({ "label": "Buy now" }));
    let node = make_node(
        "node-1",
        json!({ "steps": [fc::send_text_step_default_fn(&json!({ "text": "Hi", "buttons": [button] }))] }),
    );

    assert_eq!(
        target_json(fc::resolve_flow_action_target(&[node], s(&button, "id"))),
        json!({ "details": button, "nodeId": "node-1", "targetType": "button" })
    );
}

#[test]
fn resolveflowactiontarget_resolves_a_button_nested_in_a_carousel_card() {
    let button = fc::button_step_default_fn(&json!({ "label": "Details" }));
    let card = spread(
        &fc::send_card_step_default_fn(),
        json!({ "title": "Card", "buttons": [button] }),
    );
    let node = make_node(
        "node-1",
        json!({ "steps": [spread(&fc::send_carousel_step_default_fn(), json!({ "cards": [card] }))] }),
    );

    assert_eq!(
        target_json(fc::resolve_flow_action_target(&[node], s(&button, "id"))),
        json!({ "details": button, "nodeId": "node-1", "targetType": "button" })
    );
}

#[test]
fn resolveflowactiontarget_resolves_a_whatsapp_option_list_item_as_a_button_target() {
    let step = fc::whatsapp_option_list_step_default_fn(&json!({ "text": "Pick one" }));
    let option = step["options"][0].clone();
    let node = make_node("node-1", json!({ "steps": [step] }));

    assert_match_object(
        &target_json(fc::resolve_flow_action_target(&[node], s(&option, "id"))),
        &json!({
            "details": { "id": option["id"], "label": option["title"] },
            "nodeId": "node-1",
            "targetType": "button",
        }),
    );
}

#[test]
fn resolveflowactiontarget_resolves_a_node_level_quick_reply_as_a_quickreply_target() {
    let quick_reply = fc::button_step_default_fn(&json!({ "label": "Yes" }));
    let node = make_node(
        "node-1",
        json!({
            "steps": [fc::send_text_step_default_fn(&json!({ "text": "Ready?" }))],
            "quickReplies": [quick_reply],
        }),
    );

    assert_eq!(
        target_json(fc::resolve_flow_action_target(
            &[node],
            s(&quick_reply, "id")
        )),
        json!({ "details": quick_reply, "nodeId": "node-1", "targetType": "quickReply" })
    );
}

#[test]
fn resolveflowactiontarget_finds_a_quick_reply_on_a_later_node() {
    let quick_reply = fc::button_step_default_fn(&json!({ "label": "Yes" }));
    let nodes = [
        make_node(
            "node-1",
            json!({ "steps": [fc::send_text_step_default_fn(&json!({ "text": "Hi" }))] }),
        ),
        make_node(
            "node-2",
            json!({ "steps": [], "quickReplies": [quick_reply] }),
        ),
    ];

    assert_match_object(
        &target_json(fc::resolve_flow_action_target(
            &nodes,
            s(&quick_reply, "id"),
        )),
        &json!({ "nodeId": "node-2", "targetType": "quickReply" }),
    );
}

#[test]
fn resolveflowactiontarget_prefers_a_step_button_when_a_quick_reply_reuses_the_same_id() {
    let button = fc::button_step_default_fn(&json!({ "label": "Buy now" }));
    let nodes = [
        make_node(
            "node-1",
            json!({ "steps": [fc::send_text_step_default_fn(&json!({ "text": "Hi", "buttons": [button] }))] }),
        ),
        make_node(
            "node-2",
            json!({ "steps": [], "quickReplies": [spread(&button, json!({ "label": "Yes" }))] }),
        ),
    ];

    assert_match_object(
        &target_json(fc::resolve_flow_action_target(&nodes, s(&button, "id"))),
        &json!({ "nodeId": "node-1", "targetType": "button" }),
    );
}

#[test]
fn resolveflowactiontarget_returns_null_when_no_node_owns_the_action_id() {
    let node = make_node(
        "node-1",
        json!({
            "steps": [fc::send_text_step_default_fn(&json!({ "text": "Hi" }))],
            "quickReplies": [fc::button_step_default_fn(&json!({ "label": "Yes" }))],
        }),
    );

    assert!(fc::resolve_flow_action_target(&[node], "9999999999999").is_none());
}
