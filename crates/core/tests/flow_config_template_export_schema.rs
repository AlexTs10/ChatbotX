//! Port of `packages/flow-config/__tests__/template-export-schema.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn build_envelope() -> Value {
    json!({
        "formatVersion": fc::TEMPLATE_EXPORT_FORMAT_VERSION,
        "exportedAt": "2026-11-12T00:00:00.000Z",
        "source": { "workspaceId": "1", "tenantId": "1" },
        "manifests": { "customFields": {}, "tags": {}, "productCategories": {}, "folders": {} },
        "resources": {
            "flows": [],
            "products": [],
            "aiFunctions": [],
            "aiAgents": [],
            "calendars": [],
            "webchats": [],
            "keywords": [],
            "entryPointLinks": [],
            "triggers": [],
            "fbCommentAutomations": [],
            "settings": { "savedReplies": [], "botFields": [] },
        },
    })
}

#[test]
fn parsetemplateexport_accepts_a_well_formed_empty_envelope() {
    assert!(fc::parse_template_export(&build_envelope()).is_ok());
}

#[test]
fn parsetemplateexport_a_v2_payload_carries_its_per_item_permissions_through_a_parse_round_trip() {
    let permissions = json!({ "flows": { "f1": { "allowEdit": false } } });

    let result = fc::parse_template_export(&spread(
        &build_envelope(),
        json!({ "formatVersion": fc::TEMPLATE_EXPORT_FORMAT_VERSION_V2, "permissions": permissions }),
    ));

    assert_eq!(result.unwrap()["permissions"], permissions);
}

#[test]
fn parsetemplateexport_a_v2_payload_carries_its_inboxes_manifest_through_a_parse_round_trip() {
    let inboxes = json!({ "11": { "channelType": "messenger", "name": "Page A" } });
    let envelope = build_envelope();
    let result = fc::parse_template_export(&spread(
        &envelope,
        json!({
            "formatVersion": fc::TEMPLATE_EXPORT_FORMAT_VERSION_V2,
            "manifests": spread(&envelope["manifests"], json!({ "inboxes": inboxes })),
        }),
    ));

    assert_eq!(result.unwrap()["manifests"]["inboxes"], inboxes);
}

#[test]
fn parsetemplateexport_a_payload_without_an_inboxes_manifest_parses_with_none() {
    let result = fc::parse_template_export(&build_envelope());

    assert_eq!(result.unwrap()["manifests"]["inboxes"], json!({}));
}

#[test]
fn parsetemplateexport_a_v1_payload_still_parses_and_has_no_permissions() {
    let result = fc::parse_template_export(&build_envelope());

    assert!(result.unwrap().get("permissions").is_none());
}

#[test]
fn parsetemplateexport_never_throws_on_malformed_input() {
    let result = fc::parse_template_export(&json!({ "not": "a template" }));
    assert!(result.is_err());
}

#[test]
fn parsetemplateexport_rejects_an_unknown_formatversion_before_full_validation() {
    let result =
        fc::parse_template_export(&spread(&build_envelope(), json!({ "formatVersion": 999 })));
    let reason = result.expect_err("not ok");
    assert!(
        reason.contains("Unsupported template export format version"),
        "{reason}"
    );
}

#[test]
fn parsetemplateexport_keys_manifest_entries_by_foldertype_not_name_alone() {
    let mut envelope = build_envelope();
    envelope["manifests"]["folders"] = json!({
        "src-1": { "name": "VIP", "folderType": "automatedResponse", "parentSourceId": null },
        "src-2": { "name": "VIP", "folderType": "outboundAutomatedResponse", "parentSourceId": null },
    });
    let data = fc::parse_template_export(&envelope).unwrap();
    assert_eq!(data["manifests"]["folders"].as_object().unwrap().len(), 2);
}

#[test]
fn parsetemplateexport_requires_each_resource_entry_to_carry_a_sourceid_join_key() {
    let mut envelope = build_envelope();
    envelope["resources"]["products"] = json!([{ "name": "no source id" }]);
    assert!(fc::parse_template_export(&envelope).is_err());
}

// A draft is saved unvalidated: a flow never published (or with a step still
// being written) must travel in a template as stored; the install keeps it a
// draft.
fn flow_entry(nodes: Value) -> Value {
    json!({
        "sourceId": "f1",
        "name": "Draft only",
        "active": false,
        "enableInInbox": true,
        "startNodeId": "n1",
        "published": false,
        "nodes": nodes,
        "edges": [],
    })
}

fn with_flow(nodes: Value) -> Value {
    let envelope = build_envelope();
    spread(
        &envelope,
        json!({ "resources": spread(&envelope["resources"], json!({ "flows": [flow_entry(nodes)] })) }),
    )
}

fn with_steps(node: &Value, steps: Value) -> Value {
    spread(
        node,
        json!({ "data": spread(&node["data"], json!({ "details": spread(&node["data"]["details"], json!({ "steps": steps })) })) }),
    )
}

fn incomplete_node() -> Value {
    let node = fc::send_message_node_default_fn(&json!({ "nodeProps": { "id": "n1" } }));
    with_steps(
        &node,
        json!([fc::whatsapp_flow_step_default_fn(&json!({}))]),
    )
}

#[test]
fn template_flow_nodes_a_draft_node_with_an_incomplete_step_is_kept_as_stored() {
    let node = incomplete_node();
    assert!(!fc::flow_version_schema().safe_parse(&node).success);

    let result = fc::parse_template_export(&with_flow(json!([node])));

    let data = result.expect("ok");
    assert_eq!(data["resources"]["flows"][0]["nodes"][0], node);
}

#[test]
fn template_flow_nodes_a_complete_node_is_parsed_exactly_as_the_flow_schema_parses_it() {
    let base = fc::send_message_node_default_fn(&json!({ "nodeProps": { "id": "1" } }));
    let node = with_steps(
        &base,
        json!([fc::send_text_step_default_fn(&json!({ "text": "Hello" }))]),
    );
    let expected = fc::flow_version_schema().parse(&node).unwrap();

    let result = fc::parse_template_export(&with_flow(json!([node])));

    assert_eq!(
        result.unwrap()["resources"]["flows"][0]["nodes"][0],
        expected
    );
}

#[test]
fn template_flow_nodes_a_value_that_is_not_a_node_is_refused_s() {
    let cases = [
        ("no id", json!({ "type": "sendMessage", "data": {} })),
        ("no type", json!({ "id": "n1", "data": {} })),
        (
            "no data object",
            json!({ "id": "n1", "type": "sendMessage", "data": "x" }),
        ),
        ("not an object", json!("n1")),
    ];
    for (label, node) in cases {
        assert!(
            fc::parse_template_export(&with_flow(json!([node]))).is_err(),
            "{label}"
        );
    }
}
