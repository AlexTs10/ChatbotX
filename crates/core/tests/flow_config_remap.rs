//! Port of `packages/flow-config/__tests__/remap.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::BTreeSet;

fn maps(entries: &[(&str, &[(&str, &str)])]) -> fc::ReferenceIdMaps {
    entries
        .iter()
        .map(|(kind, pairs)| {
            (
                (*kind).to_string(),
                pairs
                    .iter()
                    .map(|(from, to)| ((*from).to_string(), (*to).to_string()))
                    .collect(),
            )
        })
        .collect()
}

fn remap(value: &Value, id_maps: &fc::ReferenceIdMaps) -> Value {
    fc::remap_references(value, id_maps, fc::RemapOptions::default())
}

fn remap_kinds(value: &Value, id_maps: &fc::ReferenceIdMaps, kinds: &[&str]) -> Value {
    fc::remap_references(
        value,
        id_maps,
        fc::RemapOptions {
            kinds: Some(kinds.iter().map(|k| (*k).to_string()).collect()),
            ..Default::default()
        },
    )
}

/// `remapReferences(value, idMaps, { kinds, onUnresolved: (ref) => unresolved.push(ref) })`.
fn remap_collect(
    value: &Value,
    id_maps: &fc::ReferenceIdMaps,
    kinds: Option<&[&str]>,
) -> (Value, Vec<fc::UnresolvedReference>) {
    let mut unresolved = Vec::new();
    let mut push = |r: &fc::UnresolvedReference| unresolved.push(r.clone());
    let remapped = fc::remap_references(
        value,
        id_maps,
        fc::RemapOptions {
            kinds: kinds.map(|ks| ks.iter().map(|k| (*k).to_string()).collect()),
            on_unresolved: Some(&mut push),
        },
    );
    (remapped, unresolved)
}

fn graph_collect(
    value: &Value,
    id_maps: &fc::ReferenceIdMaps,
) -> (Value, Vec<fc::UnresolvedReference>) {
    let mut unresolved = Vec::new();
    let mut push = |r: &fc::UnresolvedReference| unresolved.push(r.clone());
    let remapped = fc::remap_flow_graph_references(
        value,
        id_maps,
        fc::RemapOptions {
            kinds: None,
            on_unresolved: Some(&mut push),
        },
    );
    (remapped, unresolved)
}

fn unresolved(entity_kind: &str, path: &str, value: &str) -> fc::UnresolvedReference {
    fc::UnresolvedReference {
        entity_kind: entity_kind.to_string(),
        path: path.to_string(),
        value: value.to_string(),
    }
}

fn kinds_of(refs: &[fc::UnresolvedReference]) -> Vec<String> {
    refs.iter().map(|r| r.entity_kind.clone()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

// --- remapReferences — multi-kind ---

#[test]
fn remapreferences_multi_kind_remaps_distinct_kinds_independently_in_one_pass() {
    let value = json!({ "customFieldId": "cf-1", "aiAgentId": "agent-1", "calendarId": "cal-1" });

    let remapped = remap(
        &value,
        &maps(&[
            ("customField", &[("cf-1", "cf-target")]),
            ("aiAgent", &[("agent-1", "agent-target")]),
            ("calendar", &[("cal-1", "cal-target")]),
        ]),
    );

    assert_eq!(
        remapped,
        json!({ "customFieldId": "cf-target", "aiAgentId": "agent-target", "calendarId": "cal-target" })
    );
}

#[test]
fn remapreferences_multi_kind_leaves_an_unmapped_id_untouched_and_reports_it_via_onunresolved() {
    let (remapped, refs) =
        remap_collect(&json!({ "customFieldId": "cf-missing" }), &maps(&[]), None);

    assert_eq!(remapped, json!({ "customFieldId": "cf-missing" }));
    assert_eq!(
        refs,
        vec![unresolved("customField", "customFieldId", "cf-missing")]
    );
}

#[test]
fn remapreferences_multi_kind_kinds_option_scopes_remapping_so_unlisted_kinds_pass_through_inert() {
    let remapped = remap_kinds(
        &json!({ "customFieldId": "cf-1", "aiAgentId": "agent-1" }),
        &maps(&[
            ("customField", &[("cf-1", "cf-target")]),
            ("aiAgent", &[("agent-1", "agent-target")]),
        ]),
        &["customField"],
    );

    assert_eq!(
        remapped,
        json!({ "customFieldId": "cf-target", "aiAgentId": "agent-1" })
    );
}

// --- remapReferences — botField scalar key (Condition step's botFieldId) ---

#[test]
fn remapreferences_botfield_scalar_key_condition_step_s_botfieldid_remaps_a_dedicated_botfieldid_key_directly_against_idmaps_botfield_no_bot_field_token()
 {
    let remapped = remap(
        &json!({ "field": "botField", "botFieldId": "bf-1", "operator": "eq" }),
        &maps(&[("botField", &[("bf-1", "bf-target")])]),
    );

    assert_eq!(
        remapped,
        json!({ "field": "botField", "botFieldId": "bf-target", "operator": "eq" })
    );
}

#[test]
fn remapreferences_botfield_scalar_key_condition_step_s_botfieldid_leaves_an_unmapped_botfieldid_untouched_and_reports_it_via_onunresolved()
 {
    let (remapped, refs) = remap_collect(&json!({ "botFieldId": "bf-missing" }), &maps(&[]), None);

    assert_eq!(remapped, json!({ "botFieldId": "bf-missing" }));
    assert_eq!(
        refs,
        vec![unresolved("botField", "botFieldId", "bf-missing")]
    );
}

#[test]
fn remapreferences_botfield_scalar_key_condition_step_s_botfieldid_a_sibling_customfieldid_on_the_same_object_is_unaffected_by_the_botfield_map()
 {
    let remapped = remap(
        &json!({ "botFieldId": "bf-1", "customFieldId": "cf-1" }),
        &maps(&[
            ("botField", &[("bf-1", "bf-target")]),
            ("customField", &[("cf-1", "cf-target")]),
        ]),
    );

    assert_eq!(
        remapped,
        json!({ "botFieldId": "bf-target", "customFieldId": "cf-target" })
    );
}

// --- PREFIXED_REFERENCE_ENTITY_KIND — bot_field entry ---

#[test]
fn prefixed_reference_entity_kind_bot_field_entry_registers_the_same_prefix_and_kind_field_reference_ts_defines()
 {
    assert_eq!(
        fc::prefixed_reference_entity_kind()[fc::BOT_FIELD_REFERENCE_PREFIX],
        fc::field_reference_kind()["botField"]
    );
}

// --- remapReferences — bot_field scalar slot tokens ---

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_remaps_a_well_formed_bot_field_token_in_a_customfield_kind_slot_against_idmaps_botfield()
 {
    let remapped = remap(
        &json!({ "inputFieldId": "bot_field:1" }),
        &maps(&[("botField", &[("1", "42")])]),
    );

    assert_eq!(remapped, json!({ "inputFieldId": "bot_field:42" }));
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_leaves_an_unmapped_bot_field_token_untouched_and_reports_it_as_a_botfield_miss()
 {
    let (remapped, refs) = remap_collect(
        &json!({ "customFieldId": "bot_field:99" }),
        &maps(&[]),
        None,
    );

    assert_eq!(remapped, json!({ "customFieldId": "bot_field:99" }));
    assert_eq!(refs, vec![unresolved("botField", "customFieldId", "99")]);
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_never_misroutes_a_bot_field_token_against_idmaps_customfield()
 {
    // A customField map entry keyed "bot_field:1" would only ever be hit if the
    // token were (incorrectly) treated as a literal customField id/name.
    let remapped = remap(
        &json!({ "inputFieldId": "bot_field:1" }),
        &maps(&[("customField", &[("bot_field:1", "should-not-apply")])]),
    );

    assert_eq!(remapped, json!({ "inputFieldId": "bot_field:1" }));
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_a_malformed_near_token_is_treated_as_a_legacy_customfield_key_not_a_botfield_reference()
 {
    let (remapped, refs) = remap_collect(
        &json!({ "inputFieldId": "bot_field:abc" }),
        &maps(&[("customField", &[("bot_field:abc", "target")])]),
        None,
    );

    assert_eq!(remapped, json!({ "inputFieldId": "target" }));
    assert!(refs.is_empty());
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_a_malformed_near_token_with_no_customfield_match_is_left_untouched_and_warns_as_customfield_never_botfield()
 {
    let (remapped, refs) =
        remap_collect(&json!({ "inputFieldId": "bot_field:" }), &maps(&[]), None);

    assert_eq!(remapped, json!({ "inputFieldId": "bot_field:" }));
    assert_eq!(
        refs,
        vec![unresolved("customField", "inputFieldId", "bot_field:")]
    );
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_kinds_gating_kinds_botfield_remaps_the_token_and_leaves_a_sibling_customfieldid_inert()
 {
    let remapped = remap_kinds(
        &json!({ "inputFieldId": "bot_field:1", "customFieldId": "42" }),
        &maps(&[
            ("botField", &[("1", "target-bot")]),
            ("customField", &[("42", "target-custom")]),
        ]),
        &["botField"],
    );

    assert_eq!(
        remapped,
        json!({ "inputFieldId": "bot_field:target-bot", "customFieldId": "42" })
    );
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_kinds_gating_kinds_customfield_leaves_a_bot_field_token_untouched_without_warning()
 {
    let (remapped, refs) = remap_collect(
        &json!({ "inputFieldId": "bot_field:1", "customFieldId": "42" }),
        &maps(&[
            ("botField", &[("1", "target-bot")]),
            ("customField", &[("42", "target-custom")]),
        ]),
        Some(&["customField"]),
    );

    assert_eq!(
        remapped,
        json!({ "inputFieldId": "bot_field:1", "customFieldId": "target-custom" })
    );
    assert!(refs.is_empty());
}

#[test]
fn remapreferences_bot_field_scalar_slot_tokens_does_not_mutate_the_input() {
    let value = json!({ "inputFieldId": "bot_field:1" });
    let original = value.clone();

    remap(&value, &maps(&[("botField", &[("1", "target")])]));

    assert_eq!(value, original);
}

// --- remapReferences — array-valued keys ---

#[test]
fn remapreferences_array_valued_keys_remaps_each_element_of_tagids_independently() {
    let remapped = remap(
        &json!({ "tagIds": ["tag-1", "tag-2", "tag-missing"] }),
        &maps(&[("tag", &[("tag-1", "target-1"), ("tag-2", "target-2")])]),
    );

    assert_eq!(
        remapped,
        json!({ "tagIds": ["target-1", "target-2", "tag-missing"] })
    );
}

#[test]
fn remapreferences_array_valued_keys_remaps_each_element_of_sequenceids_via_the_sequence_map() {
    let remapped = remap(
        &json!({ "type": "addSequence", "sequenceIds": ["seq-1", "seq-missing"] }),
        &maps(&[("sequence", &[("seq-1", "target-1")])]),
    );

    assert_eq!(
        remapped,
        json!({ "type": "addSequence", "sequenceIds": ["target-1", "seq-missing"] })
    );
}

#[test]
fn remapreferences_array_valued_keys_remaps_addonproductids_via_the_product_map() {
    let remapped = remap(
        &json!({ "addonProductIds": ["p-1"] }),
        &maps(&[("product", &[("p-1", "p-target")])]),
    );

    assert_eq!(remapped, json!({ "addonProductIds": ["p-target"] }));
}

// --- remapReferences — sendProducts product references ---

fn send_products_step() -> Value {
    json!({ "sections": [{ "productIds": ["p-1", "p-missing"] }, { "productIds": ["p-2"] }] })
}

#[test]
fn remapreferences_sendproducts_product_references_remaps_every_section_product_via_the_product_map()
 {
    let remapped = remap(
        &send_products_step(),
        &maps(&[("product", &[("p-1", "t-1"), ("p-2", "t-2")])]),
    );

    assert_eq!(
        remapped,
        json!({ "sections": [{ "productIds": ["t-1", "p-missing"] }, { "productIds": ["t-2"] }] })
    );
}

#[test]
fn remapreferences_sendproducts_product_references_reports_each_unmapped_product_id_with_its_path()
{
    let (_, refs) = remap_collect(&send_products_step(), &maps(&[]), None);

    assert_eq!(
        refs,
        vec![
            unresolved("product", "sections[0].productIds[0]", "p-1"),
            unresolved("product", "sections[0].productIds[1]", "p-missing"),
            unresolved("product", "sections[1].productIds[0]", "p-2"),
        ]
    );
}

// --- remapReferences — discriminated union (sourceId) ---

#[test]
fn remapreferences_discriminated_union_sourceid_resolves_sourceid_as_a_trigger_id_when_sourcetype_is_trigger()
 {
    let remapped = remap(
        &json!({ "sourceType": "trigger", "sourceId": "trig-1" }),
        &maps(&[("trigger", &[("trig-1", "trig-target")])]),
    );

    assert_eq!(
        remapped,
        json!({ "sourceType": "trigger", "sourceId": "trig-target" })
    );
}

#[test]
fn remapreferences_discriminated_union_sourceid_resolves_sourceid_as_a_webhook_id_when_sourcetype_is_webhook()
 {
    let remapped = remap(
        &json!({ "sourceType": "webhook", "sourceId": "wh-1" }),
        &maps(&[("webhook", &[("wh-1", "wh-target")])]),
    );

    assert_eq!(
        remapped,
        json!({ "sourceType": "webhook", "sourceId": "wh-target" })
    );
}

#[test]
fn remapreferences_discriminated_union_sourceid_leaves_sourceid_untouched_for_an_unknown_discriminator_no_corruption()
 {
    let remapped = remap(
        &json!({ "sourceType": "somethingElse", "sourceId": "raw-1" }),
        &maps(&[("trigger", &[("raw-1", "should-not-apply")])]),
    );

    assert_eq!(
        remapped,
        json!({ "sourceType": "somethingElse", "sourceId": "raw-1" })
    );
}

// --- remapReferences — prefixed tokens (tools) ---

#[test]
fn remapreferences_prefixed_tokens_tools_rewrites_fn_file_mcp_prefixed_ids_inside_the_tools_array()
{
    let remapped = remap(
        &json!({ "tools": ["fn:1", "file:2", "mcp:3", "sys:noop"] }),
        &maps(&[
            ("aiFunction", &[("1", "1-target")]),
            ("aiFile", &[("2", "2-target")]),
            ("aiMcpServer", &[("3", "3-target")]),
        ]),
    );

    assert_eq!(
        remapped,
        json!({ "tools": ["fn:1-target", "file:2-target", "mcp:3-target", "sys:noop"] })
    );
}

#[test]
fn remapreferences_prefixed_tokens_tools_does_not_touch_adversarial_free_text_containing_fn_outside_the_tools_key()
 {
    let remapped = remap(
        &json!({ "notes": "see fn:1 for details" }),
        &maps(&[("aiFunction", &[("1", "1-target")])]),
    );

    assert_eq!(remapped, json!({ "notes": "see fn:1 for details" }));
}

#[test]
fn remapreferences_prefixed_tokens_tools_leaves_an_unresolved_prefixed_id_untouched() {
    let remapped = remap(&json!({ "tools": ["fn:missing"] }), &maps(&[]));
    assert_eq!(remapped, json!({ "tools": ["fn:missing"] }));
}

// --- remapReferences — depth ceiling and immutability ---

fn deeply_nested(depth: usize) -> Value {
    let mut value = json!({ "customFieldId": "cf-1" });
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
/// tree needs a large stack whatever `remap_references` does.
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
fn remapreferences_depth_ceiling_and_immutability_does_not_overflow_the_stack_on_a_deeply_nested_graph()
 {
    on_big_stack(|| {
        let value = deeply_nested(50_000);
        let remapped = remap(&value, &maps(&[("customField", &[("cf-1", "target")])]));
        drop(remapped);
        drop(value);
    });
}

// adapted: runs on a 1 GiB-stack thread (serde_json Value clone/drop recurse)
#[test]
fn remapreferences_depth_ceiling_and_immutability_preserves_data_past_the_depth_ceiling_without_remapping_it()
 {
    on_big_stack(|| {
        let remapped = remap(
            &deeply_nested(50_000),
            &maps(&[("customField", &[("cf-1", "target")])]),
        );
        assert_eq!(
            descend(&remapped, 50_000),
            &json!({ "customFieldId": "cf-1" })
        );
    });
}

#[test]
fn remapreferences_depth_ceiling_and_immutability_does_not_mutate_the_input() {
    let value = json!({ "customFieldId": "cf-1", "nested": { "calendarId": "cal-1" } });
    let original = value.clone();

    remap(
        &value,
        &maps(&[
            ("customField", &[("cf-1", "target")]),
            ("calendar", &[("cal-1", "target")]),
        ]),
    );

    assert_eq!(value, original);
}

// --- remapFlowGraphReferences ---

#[test]
fn remapflowgraphreferences_walks_nodes_and_edges_and_preserves_the_graph_shape() {
    let graph = json!({
        "nodes": [{ "id": "1", "data": { "customFieldId": "cf-1" } }],
        "edges": [{ "id": "e1", "source": "1", "target": "2" }],
    });

    let remapped = fc::remap_flow_graph_references(
        &graph,
        &maps(&[("customField", &[("cf-1", "target")])]),
        fc::RemapOptions::default(),
    );

    assert_eq!(remapped["nodes"][0]["data"]["customFieldId"], "target");
    assert_eq!(remapped["edges"], graph["edges"]);
}

// --- collectReferencesByKind ---

#[test]
fn collectreferencesbykind_groups_discovered_reference_ids_by_entity_kind() {
    let by_kind = fc::collect_references_by_kind(
        &json!({
            "customFieldId": "cf-1",
            "aiAgentId": "agent-1",
            "tagIds": ["tag-1", "tag-2"],
            "sequenceIds": ["seq-1"],
        }),
        None,
    );

    assert_eq!(by_kind.get("customField"), Some(&set(&["cf-1"])));
    assert_eq!(by_kind.get("aiAgent"), Some(&set(&["agent-1"])));
    assert_eq!(by_kind.get("tag"), Some(&set(&["tag-1", "tag-2"])));
    assert_eq!(by_kind.get("sequence"), Some(&set(&["seq-1"])));
}

#[test]
fn collectreferencesbykind_scopes_collection_to_the_given_kinds() {
    let by_kind = fc::collect_references_by_kind(
        &json!({ "customFieldId": "cf-1", "aiAgentId": "agent-1" }),
        Some(&["customField"]),
    );

    assert!(by_kind.contains_key("customField"));
    assert!(!by_kind.contains_key("aiAgent"));
}

#[test]
fn collectreferencesbykind_groups_a_bot_field_token_under_botfield_not_customfield() {
    let by_kind = fc::collect_references_by_kind(
        &json!({ "inputFieldId": "bot_field:7", "customFieldId": "42" }),
        None,
    );

    assert_eq!(by_kind.get("botField"), Some(&set(&["7"])));
    assert_eq!(by_kind.get("customField"), Some(&set(&["42"])));
}

// --- remapReferences — cross-flow node jumps ---

fn jump() -> Value {
    json!({ "flowId": "flow-1", "nodeId": "node-1" })
}

#[test]
fn remapreferences_cross_flow_node_jumps_remaps_the_node_id_when_the_sibling_flow_resolved() {
    let (remapped, refs) = remap_collect(
        &jump(),
        &maps(&[
            ("flow", &[("flow-1", "flow-target")]),
            ("flowNode", &[("node-1", "node-1")]),
        ]),
        None,
    );

    assert_eq!(
        remapped,
        json!({ "flowId": "flow-target", "nodeId": "node-1" })
    );
    assert!(kinds_of(&refs).is_empty());
}

#[test]
fn remapreferences_cross_flow_node_jumps_reports_only_the_flow_when_the_sibling_flow_is_unresolved()
{
    let (remapped, refs) = remap_collect(&jump(), &maps(&[("flowNode", &[])]), None);

    assert_eq!(remapped, jump());
    assert_eq!(kinds_of(&refs), vec!["flow"]);
}

// --- remapReferences — step overrides ---

fn step_in(step: Value) -> Value {
    json!({ "nodes": [{ "id": "n", "data": { "details": { "steps": [step] } } }], "edges": [] })
}

fn kind_values(refs: &[fc::UnresolvedReference]) -> Vec<String> {
    refs.iter()
        .map(|r| format!("{}:{}", r.entity_kind, r.value))
        .collect()
}

#[test]
fn remapreferences_step_overrides_activecampaign_tagids_are_third_party_ids_never_remapped_never_warned()
 {
    let flow = step_in(
        json!({ "id": "s", "stepType": "activeCampaignSyncContact", "tagIds": ["12", "34"] }),
    );

    let (remapped, refs) = graph_collect(&flow, &maps(&[("tag", &[("12", "99")])]));

    assert_eq!(remapped, flow);
    assert!(refs.is_empty());
}

#[test]
fn remapreferences_step_overrides_a_regular_step_s_tagids_are_still_remapped() {
    let remapped = fc::remap_flow_graph_references(
        &step_in(json!({ "id": "s", "stepType": "addContactTag", "tagIds": ["12"] })),
        &maps(&[("tag", &[("12", "99")])]),
        fc::RemapOptions::default(),
    );

    assert_match_object(
        &remapped["nodes"][0]["data"]["details"]["steps"][0],
        &json!({ "tagIds": ["99"] }),
    );
}

#[test]
fn remapreferences_step_overrides_assignconversation_assignedid_is_a_workspace_member_and_reported_when_unresolved()
 {
    let (_, refs) = graph_collect(
        &step_in(json!({ "id": "s", "stepType": "assignConversation", "assignedId": "77" })),
        &maps(&[]),
    );

    assert_eq!(kind_values(&refs), vec!["workspaceMember:77"]);
}

#[test]
fn remapreferences_step_overrides_a_nested_slot_is_matched_by_its_path_relative_to_the_step() {
    let (_, refs) = graph_collect(
        &step_in(
            json!({ "id": "s", "stepType": "whatsappFlow", "flow": { "id": "55", "sourceId": "x" } }),
        ),
        &maps(&[]),
    );

    assert!(kind_values(&refs).contains(&"whatsappFlow:55".to_string()));
}

#[test]
fn remapreferences_step_overrides_the_sendviewcatalog_thumbnail_is_a_meta_retailerid_not_a_product()
{
    let flow =
        step_in(json!({ "id": "s", "stepType": "sendViewCatalog", "thumbnailProductId": "SKU-1" }));

    let (_, refs) = graph_collect(&flow, &maps(&[]));

    assert!(refs.is_empty());
}

#[test]
fn remapreferences_step_overrides_an_override_only_applies_inside_its_own_step() {
    let (_, refs) = graph_collect(
        &step_in(json!({ "id": "s", "stepType": "someOtherStep", "assignedId": "77" })),
        &maps(&[]),
    );

    assert!(refs.is_empty());
}

// --- remapReferences — Condition row values ---

fn row(field: &str, value: Value) -> Value {
    json!({ "field": field, "operator": "in", "value": value })
}

#[test]
fn remapreferences_condition_row_values_remaps_tag_ids_held_in_a_condition_value() {
    let remapped = remap(
        &row("tags", json!(["12", "13"])),
        &maps(&[("tag", &[("12", "92"), ("13", "93")])]),
    );

    assert_eq!(remapped["value"], json!(["92", "93"]));
}

#[test]
fn remapreferences_condition_row_values_remaps_a_scalar_product_id() {
    let remapped = remap(
        &row("boughtItems", json!("5")),
        &maps(&[("product", &[("5", "55")])]),
    );

    assert_eq!(remapped["value"], "55");
}

#[test]
fn remapreferences_condition_row_values_never_touches_a_plain_value_like_a_country() {
    let value = row("country", json!(["VN"]));

    let (remapped, refs) = remap_collect(&value, &maps(&[]), None);

    assert_eq!(remapped, value);
    assert!(refs.is_empty());
}

#[test]
fn remapreferences_condition_row_values_third_party_ids_ads_posts_are_left_alone() {
    let (_, refs) = remap_collect(&row("clickedAd", json!(["ad-1"])), &maps(&[]), None);

    assert!(refs.is_empty());
}

#[test]
fn remapreferences_condition_row_values_a_value_key_without_the_field_and_operator_siblings_is_not_a_condition_row()
 {
    let value = json!({ "field": "tags", "value": ["12"] });

    let (remapped, refs) = remap_collect(&value, &maps(&[("tag", &[("12", "92")])]), None);

    assert_eq!(remapped, value);
    assert!(refs.is_empty());
}

#[test]
fn remapreferences_condition_row_values_an_unresolved_workspace_member_in_a_condition_value_is_removed()
 {
    let (remapped, refs) = remap_collect(
        &row("conversationAssigned", json!(["77"])),
        &maps(&[]),
        None,
    );

    assert_eq!(remapped["value"], json!([]));
    assert_eq!(kinds_of(&refs), vec!["workspaceMember"]);
}

#[test]
fn remapreferences_condition_row_values_an_unresolved_tag_in_a_condition_value_is_kept_and_reported()
 {
    let (remapped, refs) = remap_collect(&row("tags", json!(["12"])), &maps(&[]), None);

    assert_eq!(remapped["value"], json!(["12"]));
    assert_eq!(kinds_of(&refs), vec!["tag"]);
}

// --- remapReferences — assignee sentinels ---

#[test]
fn remapreferences_assignee_sentinels_keeps_the_unassigned_sentinel_and_strips_only_member_team_ids()
 {
    let (remapped, refs) = remap_collect(
        &json!({ "field": "conversationAssigned", "operator": "in", "value": ["unassigned", "u_77", "t_88"] }),
        &maps(&[]),
        None,
    );

    assert_eq!(remapped["value"], json!(["unassigned"]));
    assert_eq!(
        refs.iter().map(|r| r.value.clone()).collect::<Vec<_>>(),
        vec!["u_77", "t_88"]
    );
}
