//! Port of `packages/flow-config/__tests__/whatsapp-flow-step.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn complete_step() -> Value {
    fc::whatsapp_flow_step_default_fn(&json!({
        "text": "Fill in the form",
        "inboxId": "11",
        "flow": {
            "id": "22",
            "sourceId": "1234567890",
            "startScreenId": "WELCOME",
            "fieldMappings": [],
        },
    }))
}

fn issues(step: &Value) -> Vec<Issue> {
    fc::whatsapp_flow_step_schema()
        .safe_parse(step)
        .issues()
        .to_vec()
}

fn issue_messages(step: &Value) -> Vec<String> {
    issues(step)
        .into_iter()
        .map(|issue| issue.message)
        .collect()
}

fn with_flow(step: &Value, patch: Value) -> Value {
    spread(step, json!({ "flow": spread(&step["flow"], patch) }))
}

#[test]
fn whatsappflowstepschema_accepts_a_step_whose_dialog_was_completed() {
    assert!(
        fc::whatsapp_flow_step_schema()
            .safe_parse(&complete_step())
            .success
    );
}

fn step_with_mapping(mapping: Value) -> Value {
    let step = complete_step();
    let mapping = spread(
        &json!({ "paramKey": "docs", "customFieldId": "33" }),
        mapping,
    );
    with_flow(&step, json!({ "fieldMappings": [mapping] }))
}

#[test]
fn whatsappflowstepschema_field_mapping_file_format_is_optional_so_mappings_saved_before_it_existed_stay_valid()
 {
    let result = fc::whatsapp_flow_step_schema().safe_parse(&step_with_mapping(json!({})));
    assert!(result.success);
    assert!(
        result.data.unwrap()["flow"]["fieldMappings"][0]
            .get("fileFormat")
            .is_none()
    );
}

#[test]
fn whatsappflowstepschema_field_mapping_file_format_accepts_s() {
    for file_format in ["allUrls", "firstUrl"] {
        let result = fc::whatsapp_flow_step_schema()
            .safe_parse(&step_with_mapping(json!({ "fileFormat": file_format })));
        assert_eq!(
            result.data.unwrap()["flow"]["fieldMappings"][0]["fileFormat"],
            file_format
        );
    }
}

#[test]
fn whatsappflowstepschema_field_mapping_file_format_rejects_an_unknown_format() {
    assert!(
        !fc::whatsapp_flow_step_schema()
            .safe_parse(&step_with_mapping(json!({ "fileFormat": "csv" })))
            .success
    );
}

#[test]
fn whatsappflowstepschema_accepts_an_empty_sourceid_the_worker_resolves_it_from_flow_id() {
    let step = with_flow(&complete_step(), json!({ "sourceId": "" }));
    assert!(fc::whatsapp_flow_step_schema().safe_parse(&step).success);
}

#[test]
fn whatsappflowstepschema_rejects_a_step_with_no_whatsapp_flow_picked_once_on_flow() {
    let step = with_flow(
        &complete_step(),
        json!({ "id": null, "sourceId": "", "startScreenId": null }),
    );
    let got: Vec<Value> = issues(&step)
        .iter()
        .map(|issue| json!({ "path": issue.path_json(), "message": issue.message }))
        .collect();
    assert_eq!(
        got,
        vec![json!({ "path": ["flow"], "message": "whatsappFlowIncomplete" })]
    );
}

#[test]
fn whatsappflowstepschema_rejects_a_step_with_no_start_screen_picked() {
    let step = with_flow(&complete_step(), json!({ "startScreenId": null }));
    assert_eq!(issue_messages(&step), vec!["whatsappFlowIncomplete"]);
}

#[test]
fn whatsappflowstepschema_a_new_step_starts_incomplete() {
    assert!(
        issue_messages(&fc::whatsapp_flow_step_default_fn(&json!({})))
            .contains(&"whatsappFlowIncomplete".to_owned())
    );
}
