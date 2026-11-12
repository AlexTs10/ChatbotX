//! Port of `packages/flow-config/__tests__/import-export.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::BTreeMap;

fn now_iso() -> String {
    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn envelope(flows: Value) -> Value {
    json!({
        "formatVersion": fc::FLOW_EXPORT_FORMAT_VERSION,
        "exportedAt": now_iso(),
        "source": { "workspaceId": "1", "flowId": "1" },
        "flows": flows,
    })
}

fn build_fixture_flow() -> Value {
    let send_message_node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": {
            "quickReplies": [spread(
                &fc::button_step_default_fn(&json!({ "label": "Yes" })),
                json!({
                    "id": "10",
                    "buttonType": "startExternalFlow",
                    "beforeStep": fc::start_external_flow_step_default_fn(&json!({ "flowId": "999" })),
                    "steps": [],
                }),
            )],
            "steps": [spread(&fc::subscribe_sequence_step_default_fn(), json!({ "sequenceId": "555" }))],
        },
    }));

    let condition_node = fc::condition_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "2" },
        "detailProps": {
            "steps": [{
                "id": "20",
                "stepType": "condition",
                "otherwiseId": "21",
                "cases": [{
                    "id": "22",
                    "operator": "and",
                    "conditions": [{ "field": "name", "operator": "equals", "value": "Ada" }],
                }],
            }],
        },
    }));

    let split_traffic_node = fc::split_traffic_node_default_fn(
        &json!({ "labelVersion": 1, "nodeProps": { "id": "3" } }),
    );

    json!({
        "name": "Fixture flow",
        "active": true,
        "enableInInbox": true,
        "startNodeId": send_message_node["id"],
        "edges": [
            {
                "id": "e1",
                "source": send_message_node["id"],
                "sourceHandle": "10",
                "target": condition_node["id"],
                "targetHandle": "target",
            },
            {
                "id": "e2",
                "source": condition_node["id"],
                "sourceHandle": condition_node["data"]["details"]["steps"][0]["otherwiseId"],
                "target": split_traffic_node["id"],
                "targetHandle": "target",
            },
        ],
        "nodes": [send_message_node, condition_node, split_traffic_node],
    })
}

fn has_warning(warnings: &[fc::FlowReferenceWarning], kind: &str, value: Option<&str>) -> bool {
    warnings
        .iter()
        .any(|w| w.entity_kind == kind && value.is_none_or(|v| w.value == v))
}

fn simple_flow(name: &str, node: Value) -> Value {
    json!({
        "name": name,
        "active": true,
        "enableInInbox": true,
        "startNodeId": node["id"],
        "nodes": [node],
        "edges": [],
    })
}

fn steps_flow(steps: Value) -> Value {
    json!({ "nodes": [{ "id": "1", "data": { "details": { "steps": steps } } }], "edges": [] })
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn id_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
}

// --- flow export/import round trip ---

#[test]
fn flow_export_import_round_trip_nodes_edges_and_startnodeid_survive_export_import_byte_for_byte() {
    let flow = build_fixture_flow();
    let serialized: Value =
        serde_json::from_str(&envelope(json!([flow.clone()])).to_string()).unwrap();

    let result = fc::parse_flow_export(&serialized);

    let data = result.expect("parses");
    let imported = &data["flows"][0];
    assert_eq!(imported["nodes"], flow["nodes"]);
    assert_eq!(imported["edges"], flow["edges"]);
    assert_eq!(imported["startNodeId"], flow["startNodeId"]);
}

#[test]
fn flow_export_import_round_trip_a_template_step_with_a_payment_request_survives_export_import_unchanged()
 {
    let payment_button = json!({
        "sub_type": "payment_request",
        "index": 1,
        "payment_request": { "method": "pix", "code": "{{pix_code}}", "amount": "{{total}}" },
    });
    let node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": {
            "steps": [fc::send_wa_template_message_step_default_fn(&json!({
                "template": {
                    "id": "tpl-1",
                    "name": "pay",
                    "language": "pt_BR",
                    "params": { "button": [payment_button] },
                },
            }))],
        },
    }));
    let env = envelope(json!([simple_flow("Pay flow", node.clone())]));

    let result = fc::parse_flow_export(&serde_json::from_str(&env.to_string()).unwrap());

    let data = result.expect("parses");
    assert_eq!(data["flows"][0]["nodes"], json!([node]));
}

#[test]
fn flow_export_import_round_trip_an_export_produced_before_the_botfields_manifest_existed_still_parses_defaulting_botfields_to()
 {
    let mut env = envelope(json!([build_fixture_flow()]));
    // Deliberately no `botFields` key — an export written before this manifest was added.
    env["customFields"] = json!({});

    let result = fc::parse_flow_export(&env);

    let data = result.expect("parses");
    assert_eq!(data["botFields"], json!({}));
}

#[test]
fn flow_export_import_round_trip_parses_a_botfields_manifest_entry_alongside_customfields() {
    let mut env = envelope(json!([build_fixture_flow()]));
    env["customFields"] = json!({ "42": { "name": "Birthday", "type": "date" } });
    env["botFields"] = json!({ "7": { "name": "Loyalty Points", "type": "number" } });

    let result = fc::parse_flow_export(&env);

    let data = result.expect("parses");
    assert_eq!(
        data["botFields"],
        json!({ "7": { "name": "Loyalty Points", "type": "number" } })
    );
}

#[test]
fn flow_export_import_round_trip_rejects_a_botfields_manifest_entry_whose_name_starts_with_the_reserved_bot_field_prefix()
 {
    let mut env = envelope(json!([build_fixture_flow()]));
    env["customFields"] = json!({});
    env["botFields"] = json!({ "7": { "name": "bot_field:123", "type": "number" } });

    assert!(fc::parse_flow_export(&env).is_err());
}

// customFields intentionally does NOT reject the reserved prefix (legacy data).
#[test]
fn flow_export_import_round_trip_still_accepts_a_customfields_manifest_entry_whose_name_starts_with_the_reserved_bot_field_prefix_legacy_data()
 {
    let mut env = envelope(json!([build_fixture_flow()]));
    env["customFields"] = json!({ "42": { "name": "bot_field:123", "type": "shortText" } });
    env["botFields"] = json!({});

    assert!(fc::parse_flow_export(&env).is_ok());
}

#[test]
fn flow_export_import_round_trip_rejects_an_unknown_formatversion() {
    let result = fc::parse_flow_export(&json!({
        "formatVersion": 999,
        "exportedAt": now_iso(),
        "source": { "workspaceId": "1", "flowId": "1" },
        "flows": [],
    }));

    assert!(result.is_err());
}

#[test]
fn flow_export_import_round_trip_rejects_malformed_json_payloads() {
    assert!(fc::parse_flow_export(&json!({ "not": "an export" })).is_err());
}

#[test]
fn flow_export_import_round_trip_rejects_a_node_failing_flowversionschema() {
    let flow = build_fixture_flow();
    let broken_flow = spread(
        &flow,
        json!({ "nodes": [spread(&flow["nodes"][0], json!({ "type": "notARealNodeType" }))] }),
    );

    assert!(fc::parse_flow_export(&envelope(json!([broken_flow]))).is_err());
}

/// Import must share publish's per-channel step rules, not just the bare node union.
#[test]
fn flow_export_import_round_trip_rejects_a_whatsapp_carousel_card_that_mixes_a_link_button_with_a_reply()
 {
    let website_button = spread(
        &fc::button_step_default_fn(&json!({ "label": "Open" })),
        json!({
            "buttonType": "openWebsite",
            "beforeStep": spread(&fc::open_website_step_default_fn(), json!({ "url": "https://example.com" })),
        }),
    );
    let reply_button = fc::button_step_default_fn(&json!({ "label": "Yes" }));

    let node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": { "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": "whatsapp" })) },
    }));
    let carousel_node = spread(
        &node,
        json!({
            "data": spread(&node["data"], json!({
                "details": spread(&node["data"]["details"], json!({
                    "steps": [spread(&fc::send_carousel_step_default_fn(), json!({
                        "cards": [spread(&fc::send_card_step_default_fn(), json!({
                            "title": "Card",
                            "buttons": [website_button, reply_button],
                        }))],
                    }))],
                })),
            })),
        }),
    );

    let result = fc::parse_flow_export(&envelope(json!([simple_flow(
        "Broken carousel flow",
        carousel_node
    )])));

    assert!(result.is_err());
}

// --- collectFlowReferenceWarnings ---

#[test]
fn collectflowreferencewarnings_finds_a_sequenceid_and_a_cross_flow_flowid_in_a_nested_button_beforestep()
 {
    let warnings = fc::collect_flow_reference_warnings(&build_fixture_flow());

    assert!(has_warning(&warnings, "sequence", None));
    assert!(has_warning(&warnings, "flow", Some("999")));
}

#[test]
fn collectflowreferencewarnings_flags_the_mini_app_an_open_mini_app_button_opens() {
    let node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": {
            "quickReplies": [spread(
                &fc::button_step_default_fn(&json!({ "label": "Form" })),
                json!({
                    "id": "11",
                    "buttonType": "openMiniApp",
                    "beforeStep": fc::open_mini_app_step_default_fn(&json!({ "id": "12", "miniAppId": "77" })),
                }),
            )],
        },
    }));

    let warnings = fc::collect_flow_reference_warnings(&simple_flow("Mini App flow", node));

    assert!(has_warning(&warnings, "miniApp", Some("77")));
}

#[test]
fn collectflowreferencewarnings_does_not_flag_addcontacttag_tags_as_a_reference() {
    let node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": {
            "steps": [{ "id": "20", "stepType": "addContactTag", "tags": ["vip", "newsletter"] }],
        },
    }));

    let warnings = fc::collect_flow_reference_warnings(&simple_flow("Tag flow", node));

    assert!(warnings.is_empty());
}

#[test]
fn collectflowreferencewarnings_does_not_flag_startanothernode_nodeid_same_flow_reference() {
    let node = fc::send_message_node_default_fn(&json!({
        "labelVersion": 1,
        "nodeProps": { "id": "1" },
        "detailProps": {
            "quickReplies": [spread(
                &fc::button_step_default_fn(&json!({ "label": "Continue" })),
                json!({
                    "id": "11",
                    "buttonType": "startAnotherNode",
                    "beforeStep": fc::start_another_node_step_default_fn(&json!({ "nodeId": "2" })),
                    "steps": [],
                }),
            )],
        },
    }));

    let warnings = fc::collect_flow_reference_warnings(&simple_flow("Node jump flow", node));

    assert!(warnings.is_empty());
}

// --- collectCustomFieldReferences ---

#[test]
fn collectcustomfieldreferences_system_slugs_and_merge_tag_text_never_appear_in_the_collected_ids()
{
    let ids = fc::collect_custom_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "first_name" },
        { "id": "s2", "stepType": "setCustomField", "inputFieldId": "user_tags" },
        { "id": "s3", "stepType": "setCustomField", "inputFieldId": "42" },
    ])));

    assert_eq!(ids, vec!["42"]);
}

#[test]
fn collectcustomfieldreferences_dedupes_the_same_id_referenced_from_multiple_slots() {
    let ids = fc::collect_custom_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "42" },
        {
            "id": "s2",
            "stepType": "condition",
            "cases": [{ "id": "c1", "conditions": [{ "customFieldId": "42" }, { "customFieldId": "43" }] }],
        },
    ])));

    assert_eq!(sorted(ids), vec!["42", "43"]);
}

#[test]
fn collectcustomfieldreferences_collects_across_the_awkward_per_step_shapes() {
    let ids = fc::collect_custom_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "condition", "cases": [{ "id": "c1", "conditions": [{ "customFieldId": "1" }] }] },
        { "id": "s2", "stepType": "aiExtractData", "extractFields": [{ "customFieldId": "2" }] },
        {
            "id": "s3",
            "stepType": "getDataFromJson",
            "inputFieldId": "3",
            "mapping": [{ "jsonPath": "$.a", "outputFieldId": "4" }],
        },
        {
            "id": "s4",
            "stepType": "whatsappFlow",
            "flow": { "fieldMappings": [{ "customFieldId": "5" }, { "customFieldId": null }] },
        },
        {
            "id": "s5",
            "stepType": "appointmentScheduling",
            "mode": "checkAvailabilityFromCustomField",
            "startDateFieldId": "6",
            "endDateFieldId": "7",
            "outputCustomFieldId": "8",
        },
        { "id": "s6", "stepType": "spreadsheetSendData", "map": [{ "customFieldId": "" }] },
    ])));

    assert_eq!(sorted(ids), vec!["1", "2", "3", "4", "5", "6", "7", "8"]);
}

// --- collectFieldReferences ---

#[test]
fn collectfieldreferences_splits_numeric_customfield_ids_and_bot_field_tokens_into_their_own_lists()
{
    let result = fc::collect_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "42" },
        { "id": "s2", "stepType": "setCustomField", "inputFieldId": "bot_field:7" },
        {
            "id": "s3",
            "stepType": "condition",
            "cases": [{ "id": "c1", "conditions": [{ "customFieldId": "bot_field:8" }] }],
        },
    ])));

    assert_eq!(result.custom_field_ids, vec!["42"]);
    assert_eq!(sorted(result.bot_field_ids), vec!["7", "8"]);
}

#[test]
fn collectfieldreferences_a_malformed_near_token_contributes_to_neither_list() {
    let result = fc::collect_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "bot_field:abc" },
    ])));

    assert!(result.custom_field_ids.is_empty());
    assert!(result.bot_field_ids.is_empty());
}

#[test]
fn collectfieldreferences_collects_a_dedicated_botfieldid_key_condition_step_s_botfield_condition_as_a_raw_id_not_a_token()
 {
    let result = fc::collect_field_references(&steps_flow(json!([{
        "id": "s1",
        "stepType": "condition",
        "cases": [{
            "id": "c1",
            "conditions": [{ "field": "botField", "botFieldId": "9", "operator": "eq" }],
        }],
    }])));

    assert_eq!(result.bot_field_ids, vec!["9"]);
    assert!(result.custom_field_ids.is_empty());
}

#[test]
fn collectfieldreferences_dedupes_bot_field_ids_referenced_from_multiple_slots() {
    let result = fc::collect_field_references(&steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "bot_field:7" },
        {
            "id": "s2",
            "stepType": "condition",
            "cases": [{ "id": "c1", "conditions": [{ "customFieldId": "bot_field:7" }] }],
        },
    ])));

    assert_eq!(result.bot_field_ids, vec!["7"]);
}

#[test]
fn collectfieldreferences_collectcustomfieldreferences_stays_behaviorally_identical_thin_wrapper() {
    let flow = steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "42" },
        { "id": "s2", "stepType": "setCustomField", "inputFieldId": "bot_field:7" },
    ]));

    assert_eq!(
        fc::collect_custom_field_references(&flow),
        fc::collect_field_references(&flow).custom_field_ids
    );
    assert_eq!(fc::collect_custom_field_references(&flow), vec!["42"]);
}

// --- remapCustomFieldReferences ---

#[test]
fn remapcustomfieldreferences_rewrites_a_mapped_id_and_leaves_an_unmapped_id_untouched() {
    let flow = steps_flow(json!([
        { "id": "s1", "stepType": "setCustomField", "inputFieldId": "42" },
        { "id": "s2", "stepType": "setCustomField", "inputFieldId": "999" },
    ]));

    let remapped = fc::remap_custom_field_references(&flow, &id_map(&[("42", "target-42")]));

    let steps = &remapped["nodes"][0]["data"]["details"]["steps"];
    assert_eq!(steps[0]["inputFieldId"], "target-42");
    assert_eq!(steps[1]["inputFieldId"], "999");
}

#[test]
fn remapcustomfieldreferences_preserves_sibling_keys_customfieldtype_valuetype_on_a_condition_row()
{
    let flow = steps_flow(json!([{
        "id": "s1",
        "stepType": "condition",
        "cases": [{
            "id": "c1",
            "conditions": [{
                "customFieldId": "42",
                "customFieldType": "date",
                "valueType": "date",
                "operator": "eq",
            }],
        }],
    }]));

    let remapped = fc::remap_custom_field_references(&flow, &id_map(&[("42", "target-42")]));

    assert_eq!(
        remapped["nodes"][0]["data"]["details"]["steps"][0]["cases"][0]["conditions"][0],
        json!({ "customFieldId": "target-42", "customFieldType": "date", "valueType": "date", "operator": "eq" })
    );
}

#[test]
fn remapcustomfieldreferences_does_not_mutate_the_input() {
    let flow =
        steps_flow(json!([{ "id": "s1", "stepType": "setCustomField", "inputFieldId": "42" }]));
    let original = flow.clone();

    fc::remap_custom_field_references(&flow, &id_map(&[("42", "target-42")]));

    assert_eq!(flow, original);
}

// --- deeply nested graphs ---

fn deeply_nested(depth: usize) -> Value {
    let mut value = json!({ "inputFieldId": "42" });
    for _ in 0..depth {
        // Not `json!({ "child": value })`: that re-serializes `value` (quadratic).
        let mut wrapper = serde_json::Map::new();
        wrapper.insert("child".to_string(), value);
        value = Value::Object(wrapper);
    }
    value
}

fn descend(value: &Value, depth: usize) -> &Value {
    let mut current = value;
    for _ in 0..depth {
        current = &current["child"];
    }
    current
}

/// serde_json clones, compares and drops a `Value` recursively; a 50 000-level
/// tree needs a large stack whatever the walkers do.
fn on_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join");
}

// adapted: runs on a 1 GiB-stack thread (serde_json Value clone/drop recurse); "not.toThrow" = no panic
#[test]
fn deeply_nested_graphs_collectflowreferencewarnings_does_not_overflow_the_stack() {
    on_big_stack(|| {
        let flow = json!({ "nodes": [deeply_nested(50_000)], "edges": [] });
        let _ = fc::collect_flow_reference_warnings(&flow);
    });
}

// adapted: runs on a 1 GiB-stack thread (serde_json Value clone/drop recurse); "not.toThrow" = no panic
#[test]
fn deeply_nested_graphs_collectcustomfieldreferences_does_not_overflow_the_stack() {
    on_big_stack(|| {
        let flow = json!({ "nodes": [deeply_nested(50_000)], "edges": [] });
        let _ = fc::collect_custom_field_references(&flow);
    });
}

// adapted: runs on a 1 GiB-stack thread (serde_json Value clone/drop recurse)
#[test]
fn deeply_nested_graphs_remapcustomfieldreferences_preserves_data_past_the_depth_ceiling() {
    on_big_stack(|| {
        let flow = json!({ "nodes": [deeply_nested(50_000)], "edges": [] });

        let remapped = fc::remap_custom_field_references(&flow, &id_map(&[("42", "target-42")]));

        // Truncation must never drop the subtree — the deep leaf still resolves.
        assert_eq!(
            descend(&remapped["nodes"][0], 50_000),
            &json!({ "inputFieldId": "42" })
        );
    });
}

#[test]
fn deeply_nested_graphs_still_remaps_references_at_realistic_depths() {
    let flow = json!({ "nodes": [deeply_nested(10)], "edges": [] });

    let remapped = fc::remap_custom_field_references(&flow, &id_map(&[("42", "target-42")]));

    assert_eq!(
        descend(&remapped["nodes"][0], 10),
        &json!({ "inputFieldId": "target-42" })
    );
}
