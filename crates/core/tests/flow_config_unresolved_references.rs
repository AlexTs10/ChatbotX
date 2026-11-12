//! Port of `packages/flow-config/__tests__/unresolved-references.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn flow_with(step: Value) -> Value {
    json!({ "nodes": [{ "id": "n1", "data": { "details": { "steps": [step] } } }], "edges": [] })
}

fn step_of(flow: &Value) -> &Value {
    &flow["nodes"][0]["data"]["details"]["steps"][0]
}

fn slot(key: &str) -> String {
    format!("nodes[0].data.details.steps[0].{key}")
}

fn remove(flow: &Value, paths: &[String]) -> Value {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    fc::remove_unresolved_references(flow, &refs)
}

#[test]
fn removeunresolvedreferences_drops_an_array_element() {
    let flow =
        flow_with(json!({ "id": "s", "stepType": "addContactTag", "tagIds": ["1", "2", "3"] }));

    let cleaned = remove(&flow, &[slot("tagIds[0]"), slot("tagIds[2]")]);

    assert_eq!(step_of(&cleaned)["tagIds"], json!(["2"]));
}

#[test]
fn removeunresolvedreferences_deletes_an_optional_slot() {
    let flow = flow_with(json!({ "id": "s", "stepType": "subscribeSequence", "sequenceId": "9" }));

    let cleaned = remove(&flow, &[slot("sequenceId")]);

    assert!(step_of(&cleaned).get("sequenceId").is_none());
}

#[test]
fn removeunresolvedreferences_sets_a_nullable_slot_to_null() {
    let flow = flow_with(json!({ "id": "s", "stepType": "sendWaTemplateMessage", "inboxId": "5" }));

    let cleaned = remove(&flow, &[slot("inboxId")]);

    assert!(step_of(&cleaned)["inboxId"].is_null());
    assert!(step_of(&cleaned).get("inboxId").is_some());
}

#[test]
fn removeunresolvedreferences_clears_a_required_slot_to_the_editor_s_empty_value() {
    let flow = flow_with(json!({ "id": "s", "stepType": "setCustomField", "inputFieldId": "77" }));

    let cleaned = remove(&flow, &[slot("inputFieldId")]);

    assert_eq!(step_of(&cleaned)["inputFieldId"], "");
}

#[test]
fn removeunresolvedreferences_treats_a_condition_row_s_id_slot_as_optional() {
    let flow = flow_with(json!({
        "id": "s",
        "stepType": "condition",
        "conditions": [{ "field": "customField", "operator": "eq", "customFieldId": "77" }],
    }));

    let cleaned = remove(&flow, &[slot("conditions[0].customFieldId")]);

    assert!(
        step_of(&cleaned)["conditions"][0]
            .get("customFieldId")
            .is_none()
    );
}

#[test]
fn removeunresolvedreferences_never_mutates_the_input_and_skips_a_path_that_no_longer_resolves() {
    let flow = flow_with(json!({ "id": "s", "stepType": "addContactTag", "tagIds": ["1"] }));

    let cleaned = remove(&flow, &[slot("tagIds[0]"), "nodes[9].data".to_owned()]);

    assert_eq!(step_of(&flow)["tagIds"], json!(["1"]));
    assert_eq!(step_of(&cleaned)["tagIds"], json!([]));
}

#[test]
fn removeunresolvedreferences_removes_several_elements_of_one_array_whatever_order_they_are_reported_in()
 {
    let flow = flow_with(
        json!({ "id": "s", "stepType": "addContactTag", "tagIds": ["a", "b", "c", "d"] }),
    );

    let cleaned = remove(
        &flow,
        &[slot("tagIds[1]"), slot("tagIds[3]"), slot("tagIds[0]")],
    );

    assert_eq!(step_of(&cleaned)["tagIds"], json!(["c"]));
}

#[test]
fn removeunresolvedreferences_a_required_pipeline_stage_is_cleared_not_deleted() {
    let flow = flow_with(json!({ "id": "s", "stepType": "pipeline", "stageId": "9" }));

    let cleaned = remove(&flow, &[slot("stageId")]);

    assert_eq!(step_of(&cleaned)["stageId"], "");
}

#[test]
fn removeunresolvedreferences_a_nullable_whatsapp_flow_field_mapping_becomes_null() {
    let flow = flow_with(json!({
        "id": "s",
        "stepType": "whatsappFlow",
        "flow": { "fieldMappings": [{ "customFieldId": "7" }] },
    }));

    let cleaned = remove(&flow, &[slot("flow.fieldMappings[0].customFieldId")]);

    assert!(step_of(&cleaned)["flow"]["fieldMappings"][0]["customFieldId"].is_null());
}

#[test]
fn removeunresolvedreferences_returns_the_input_itself_when_there_is_nothing_to_remove() {
    let flow = flow_with(json!({ "id": "s", "stepType": "addContactTag", "tagIds": ["1"] }));

    assert_eq!(fc::remove_unresolved_references(&flow, &[]), flow);
}

#[test]
fn removeunresolvedreferencesdetailed_an_assign_step_keeps_its_place_with_nobody_in_it_instead_of_blocking_the_flow()
 {
    let flow = json!({
        "nodes": [{ "data": { "details": { "steps": [{ "stepType": "assignConversation", "assignedId": "u_7" }] } } }],
    });

    let result = fc::remove_unresolved_references_detailed(
        &flow,
        &["nodes[0].data.details.steps[0].assignedId"],
    );

    assert!(result.blocked.is_empty());
    assert_eq!(result.removed.len(), 1);
    assert_eq!(
        result.value["nodes"][0]["data"]["details"]["steps"][0]["assignedId"],
        ""
    );
}

fn steps_flow() -> Value {
    json!({
        "nodes": [{
            "id": "1",
            "data": { "details": { "steps": [
                { "id": "a", "stepType": "aiSpeechToText", "inputFieldId": "9" },
                { "id": "b", "stepType": "setCustomField", "inputFieldId": "10" },
            ] } },
        }],
        "edges": [],
    })
}

#[test]
fn removeunresolvedreferencesdetailed_a_required_slot_inside_a_flow_step_blocks_the_flow_instead_of_dropping_the_step()
 {
    let flow = steps_flow();
    let result = fc::remove_unresolved_references_detailed(
        &flow,
        &["nodes[0].data.details.steps[0].inputFieldId"],
    );

    assert_eq!(
        result.blocked,
        vec!["nodes[0].data.details.steps[0].inputFieldId"]
    );
    assert!(result.dropped_elements.is_empty());
    let steps = result.value["nodes"][0]["data"]["details"]["steps"]
        .as_array()
        .unwrap();
    let ids: Vec<&str> = steps.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(steps[0]["inputFieldId"], "");
}

#[test]
fn removeunresolvedreferencesdetailed_a_required_slot_inside_an_action_row_drops_that_row_only() {
    let flow = json!({
        "nodes": [{
            "id": "1",
            "data": { "details": { "steps": [{
                "id": "a",
                "stepType": "performActions",
                "actions": [
                    { "actionType": "setField", "customFieldId": "9" },
                    { "actionType": "setField", "customFieldId": "10" },
                ],
            }] } },
        }],
        "edges": [],
    });
    let result = fc::remove_unresolved_references_detailed(
        &flow,
        &["nodes[0].data.details.steps[0].actions[0].customFieldId"],
    );

    assert_eq!(result.dropped_elements.len(), 1);
    assert!(result.blocked.is_empty());
    let ids: Vec<&Value> = result.value["nodes"][0]["data"]["details"]["steps"][0]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| &a["customFieldId"])
        .collect();
    assert_eq!(ids, vec!["10"]);
}

#[test]
fn removeunresolvedreferencesdetailed_an_optional_slot_is_deleted_and_a_nullable_slot_nulled_without_touching_the_step()
 {
    let flow = json!({
        "nodes": [{
            "id": "1",
            "data": { "details": { "steps": [
                { "id": "a", "stepType": "subscribeSequence", "sequenceId": "9" },
                { "id": "b", "stepType": "sendWaTemplateMessage", "inboxId": "5" },
            ] } },
        }],
        "edges": [],
    });
    let result = fc::remove_unresolved_references_detailed(
        &flow,
        &[
            "nodes[0].data.details.steps[0].sequenceId",
            "nodes[0].data.details.steps[1].inboxId",
        ],
    );

    assert!(result.blocked.is_empty());
    assert!(result.dropped_elements.is_empty());
    let steps = &result.value["nodes"][0]["data"]["details"]["steps"];
    assert!(steps[0].get("sequenceId").is_none());
    assert!(steps[1]["inboxId"].is_null());
}

#[test]
fn removeunresolvedreferencesdetailed_reports_a_required_slot_outside_any_droppable_element_as_blocked()
 {
    let flow = json!({ "nodes": [{ "id": "1", "data": { "flowId": "7" } }], "edges": [] });
    let result = fc::remove_unresolved_references_detailed(&flow, &["nodes[0].data.flowId"]);
    assert_eq!(result.blocked, vec!["nodes[0].data.flowId"]);
}
