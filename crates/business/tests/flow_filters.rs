//! Port of `packages/business/__tests__/flow-filters.test.ts` (P3.6-D2).
//! The test flows are JSON values (`{ id, flowVersions: [{ nodes }] }`).

use chatbotx_business::flow::filters::{
    filter_flows_by_start_step_type, filter_flows_by_template_ids, has_start_node,
};
use serde_json::{Value, json};

const SEND_WA_TEMPLATE: &str = "sendWaTemplateMessage";
const SEND_MESSENGER_TEMPLATE: &str = "sendMessengerTemplateMessage";

fn flow_with_steps(id: &str, steps: Value, is_start_node: bool) -> Value {
    json!({
        "id": id,
        "flowVersions": [{
            "nodes": [{
                "id": "node-1",
                "data": { "isStartNode": is_start_node, "details": { "steps": steps } },
            }],
        }],
    })
}

fn ids(flows: &[Value]) -> Vec<&str> {
    flows
        .iter()
        .map(|f| f["id"].as_str().unwrap_or_default())
        .collect()
}

#[test]
fn detects_a_step_on_the_start_node_only() {
    let nodes = |is_start: bool| {
        json!([{
            "data": {
                "isStartNode": is_start,
                "details": { "steps": [{ "stepType": SEND_WA_TEMPLATE }] },
            },
        }])
    };
    assert!(has_start_node(&nodes(true), SEND_WA_TEMPLATE));
    assert!(!has_start_node(&nodes(false), SEND_WA_TEMPLATE));
    assert!(!has_start_node(&Value::Null, SEND_WA_TEMPLATE));
}

#[test]
fn filters_flows_by_whatsapp_messenger_and_legacy_whatsapp_step_types() {
    let whatsapp = flow_with_steps("wa", json!([{ "stepType": SEND_WA_TEMPLATE }]), true);
    let legacy = flow_with_steps("legacy-wa", json!([{ "stepType": "WA_TM01" }]), true);
    let messenger = flow_with_steps(
        "messenger",
        json!([{ "stepType": SEND_MESSENGER_TEMPLATE }]),
        true,
    );
    let non_start = flow_with_steps(
        "non-start",
        json!([{ "stepType": SEND_WA_TEMPLATE }]),
        false,
    );
    let out = filter_flows_by_start_step_type(
        vec![
            whatsapp.clone(),
            legacy.clone(),
            messenger.clone(),
            non_start,
        ],
        SEND_WA_TEMPLATE,
    );
    assert_eq!(ids(&out), vec!["wa", "legacy-wa"]);
    let out =
        filter_flows_by_start_step_type(vec![whatsapp, legacy, messenger], SEND_MESSENGER_TEMPLATE);
    assert_eq!(ids(&out), vec!["messenger"]);
}

#[test]
fn filters_whatsapp_template_ids_across_current_and_legacy_step_types() {
    let current = flow_with_steps(
        "current-template",
        json!([{ "stepType": SEND_WA_TEMPLATE, "template": { "id": "template-1" } }]),
        true,
    );
    let legacy = flow_with_steps(
        "legacy-template",
        json!([{ "stepType": "WA_TM01", "template": { "id": "template-1" } }]),
        true,
    );
    let wrong = flow_with_steps(
        "wrong-template",
        json!([{ "stepType": SEND_WA_TEMPLATE, "template": { "id": "template-2" } }]),
        true,
    );
    let wrong_start = json!({
        "id": "wrong-start-template",
        "flowVersions": [{
            "nodes": [
                {
                    "id": "start-node",
                    "data": {
                        "isStartNode": true,
                        "details": { "steps": [{ "stepType": SEND_WA_TEMPLATE, "template": { "id": "template-2" } }] },
                    },
                },
                {
                    "id": "non-start-node",
                    "data": {
                        "isStartNode": false,
                        "details": { "steps": [{ "stepType": SEND_WA_TEMPLATE, "template": { "id": "template-1" } }] },
                    },
                },
            ],
        }],
    });
    let template_1 = vec!["template-1".to_string()];
    let out = filter_flows_by_template_ids(
        vec![current.clone(), legacy, wrong, wrong_start],
        &template_1,
    );
    assert_eq!(ids(&out), vec!["current-template", "legacy-template"]);
    assert!(filter_flows_by_template_ids(vec![current], &[]).is_empty());
}
