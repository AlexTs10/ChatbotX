//! Port of `packages/flow-config/__tests__/authoring/pipeline-spec.test.ts`.

mod flow_config_authoring_support;
mod flow_config_support;
use flow_config_authoring_support::*;
use flow_config_support::{Value, assert_match_object, json, spread};

fn stage(id: &str, name: &str) -> PipelineStageRef {
    PipelineStageRef {
        id: id.to_string(),
        name: name.to_string(),
    }
}

fn field(id: &str, name: &str, reference_name: &str) -> PipelineFieldRef {
    PipelineFieldRef {
        id: id.to_string(),
        name: name.to_string(),
        reference_name: reference_name.to_string(),
    }
}

fn ctx() -> FlowAuthoringContext {
    FlowAuthoringContext {
        templates_by_name: map_of(vec![]),
        custom_fields_by_name: map_of(vec![
            ("Follow up on", custom_field("7001", "datetime")),
            ("Plan", custom_field("7002", "text")),
        ]),
        flows_by_name: map_of(vec![]),
        pipelines_by_reference_name: Some(map_of(vec![
            (
                "sales",
                FlowAuthoringPipeline {
                    id: "5001".to_string(),
                    name: "Sales".to_string(),
                    reference_name: "sales".to_string(),
                    stages: vec![
                        stage("5101", "New"),
                        stage("5102", "Won deal"),
                        stage("5103", "Dup"),
                        stage("5104", "Dup"),
                    ],
                    fields: vec![
                        field("5201", "Budget", "budget"),
                        field("5202", "Source", "source"),
                    ],
                },
            ),
            (
                "support",
                FlowAuthoringPipeline {
                    id: "5002".to_string(),
                    name: "Support".to_string(),
                    reference_name: "support".to_string(),
                    stages: vec![stage("5301", "Triage")],
                    fields: vec![],
                },
            ),
        ])),
    }
}

fn base() -> Value {
    json!({ "type": "pipeline", "pipeline": "sales" })
}

fn with_base(extra: Value) -> Value {
    spread(&base(), extra)
}

fn parse(steps: Value) -> Value {
    fc::flow_spec_schema()
        .parse(&json!({ "formatVersion": 1, "name": "Pipelines", "steps": steps }))
        .expect("valid spec")
}

fn compile(steps: Value) -> Result<CompiledFlow, fc::FlowAuthoringException> {
    fc::compile_flow_spec(&parse(steps), &ctx())
}

fn step_of(steps: Value) -> Value {
    let compiled = compile(steps).expect("compiles");
    let node = &compiled.nodes[0];
    assert_eq!(
        node["type"], "performAction",
        "expected a performAction node"
    );
    node["data"]["details"]["steps"][0].clone()
}

fn expect_publishable_steps(steps: Value) {
    let compiled = compile(steps).expect("compiles");
    expect_publishable(&compiled.nodes, &compiled.edges);
}

/// `{ path: issue.path.join("."), message }` of each issue.
fn issues_of(steps: Value) -> Vec<(String, String)> {
    let result = fc::flow_spec_schema()
        .safe_parse(&json!({ "formatVersion": 1, "name": "Pipelines", "steps": steps }));
    assert!(!result.success, "expected the spec to be invalid");
    result
        .error
        .unwrap()
        .issues
        .iter()
        .map(|issue| {
            let path: Vec<String> = issue
                .path_json()
                .as_array()
                .unwrap()
                .iter()
                .map(|s| match s {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            (path.join("."), issue.message.clone())
        })
        .collect()
}

fn errors_of(steps: Value) -> Vec<FlowAuthoringError> {
    match compile(steps) {
        Ok(_) => panic!("expected compileFlowSpec to throw"),
        Err(exception) => exception.errors,
    }
}

fn error_json(error: &FlowAuthoringError) -> Value {
    serde_json::to_value(error).unwrap()
}

fn issue(path: &str, message: &str) -> (String, String) {
    (path.to_string(), message.to_string())
}

fn valid_steps() -> Vec<(&'static str, Value)> {
    vec![
        (
            "addToPipeline",
            json!({
                "title": "Deal for {{full_name}}",
                "description": "From chat",
                "status": "open",
                "stage": "Won deal",
                "assigneeMemberIds": ["11"],
                "assigneeInboxTeamIds": ["12"],
                "dueDate": { "mode": "fixed", "at": "2026-12-01T10:00:00Z" },
                "fieldValues": [{ "field": "budget", "value": "{{budget}}" }],
            }),
        ),
        ("moveToStage", json!({ "stage": "Won deal" })),
        ("changeValue", json!({ "value": "120.5" })),
        (
            "assign",
            json!({ "assigneeMemberIds": ["11"], "assigneeInboxTeamIds": [] }),
        ),
        ("changeStatus", json!({ "status": "won" })),
        ("changePriority", json!({ "priority": "high" })),
        (
            "changeDueDate",
            json!({ "dueDate": { "mode": "customField", "customFieldName": "Follow up on" } }),
        ),
        ("changeTitle", json!({ "title": "New title" })),
        (
            "setCustomField",
            json!({ "field": "source", "value": "ads" }),
        ),
        (
            "clearCustomField",
            json!({ "fields": ["budget", "source"] }),
        ),
        ("changeDescription", json!({ "description": "Updated" })),
        ("removeFromPipeline", json!({})),
        ("addComment", json!({ "text": "Called the customer" })),
        (
            "transfer",
            json!({ "destinationPipeline": "support", "destinationStage": "Triage" }),
        ),
    ]
}

fn valid_step(action: &str) -> Value {
    let extra = valid_steps()
        .into_iter()
        .find(|(a, _)| *a == action)
        .unwrap()
        .1;
    spread(&with_base(json!({ "action": action })), extra)
}

// --- pipeline spec step ---

#[test]
fn pipeline_spec_step_is_listed_in_the_spec_step_types() {
    let types: Vec<&str> = fc::flow_spec_step_types()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"pipeline"));
}

#[test]
fn pipeline_spec_step_covers_every_pipeline_action() {
    let mut actions: Vec<String> = valid_steps()
        .into_iter()
        .map(|(a, _)| a.to_string())
        .collect();
    actions.sort();
    let mut options = fc::pipeline_step_actions().enum_values();
    options.sort();
    assert_eq!(actions, options);
}

#[test]
fn pipeline_spec_step_s_compiles_to_a_publishable_pipeline_step() {
    for (action, _) in valid_steps() {
        let steps = json!([valid_step(action)]);
        let step = step_of(steps.clone());
        assert_eq!(step["stepType"], "pipeline", "{action}");
        assert_eq!(step["action"], action);
        assert_eq!(step["pipelineId"], "5001", "{action}");
        expect_publishable_steps(steps);
    }
}

#[test]
fn pipeline_spec_step_resolves_ids_for_stages_fields_due_date_and_transfer() {
    assert_match_object(
        &step_of(json!([valid_step("addToPipeline")])),
        &json!({
            "stageId": "5102",
            "fieldValues": [{ "pipelineFieldId": "5201", "value": "{{budget}}" }],
            "dueDate": { "mode": "fixed", "at": "2026-12-01T10:00:00Z" },
            "assigneeMemberIds": ["11"],
            "assigneeInboxTeamIds": ["12"],
        }),
    );
    assert_match_object(
        &step_of(json!([valid_step("changeDueDate")])),
        &json!({ "dueDate": { "mode": "customField", "customFieldId": "7001" } }),
    );
    assert_match_object(
        &step_of(json!([valid_step("clearCustomField")])),
        &json!({ "pipelineFieldIds": ["5201", "5202"] }),
    );
    assert_match_object(
        &step_of(json!([valid_step("transfer")])),
        &json!({ "destinationPipelineId": "5002", "destinationStageId": "5301" }),
    );
}

#[test]
fn pipeline_spec_step_addtopipeline_defaults_match_the_editor_defaults_pipeline_may_be_an_id() {
    assert_match_object(
        &step_of(json!([{ "type": "pipeline", "pipeline": "5001", "action": "addToPipeline" }])),
        &json!({
            "pipelineId": "5001",
            "title": "{{full_name}}",
            "status": "open",
            "dueDate": { "mode": "none" },
            "fieldValues": [],
        }),
    );
}

// --- pipeline spec step > spec validation paths ---

#[test]
fn pipeline_spec_step_spec_validation_paths_requires_each_action_s_fields() {
    assert_eq!(
        issues_of(json!([with_base(json!({ "action": "moveToStage" }))])),
        vec![issue(
            "steps.0.stage",
            "action \"moveToStage\" requires stage"
        )]
    );
    assert_eq!(
        issues_of(json!([with_base(
            json!({ "action": "setCustomField", "field": "source" })
        )]))
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>(),
        vec!["steps.0.value"]
    );
    assert_eq!(
        issues_of(json!([with_base(
            json!({ "action": "clearCustomField", "fields": [] })
        )]))[0]
            .0,
        "steps.0.fields"
    );
    assert_eq!(
        issues_of(json!([with_base(json!({ "action": "transfer" }))]))
            .first()
            .map(|(path, _)| path.as_str()),
        Some("steps.0.destinationPipeline")
    );
}

#[test]
fn pipeline_spec_step_spec_validation_paths_transfer_cannot_target_the_step_s_own_pipeline_by_name_or_by_id()
 {
    let sales_id = "5001";
    for destination_pipeline in ["sales", sales_id] {
        let errors = errors_of(json!([with_base(json!({
            "pipeline": "sales",
            "action": "transfer",
            "destinationPipeline": destination_pipeline,
        }))]));
        assert_eq!(errors.len(), 1, "{destination_pipeline}");
        assert_match_object(
            &error_json(&errors[0]),
            &json!({ "path": "steps[0].destinationPipeline", "code": "invalidStep" }),
        );
    }
}

// The runtime rejects an assign with nobody to assign, so the spec must too.
#[test]
fn pipeline_spec_step_spec_validation_paths_assign_needs_at_least_one_member_or_team() {
    let expected = vec![issue(
        "steps.0.assigneeMemberIds",
        "action \"assign\" requires at least one of assigneeMemberIds, assigneeInboxTeamIds",
    )];
    assert_eq!(
        issues_of(json!([with_base(json!({ "action": "assign" }))])),
        expected
    );
    assert_eq!(
        issues_of(json!([with_base(json!({
            "action": "assign",
            "assigneeMemberIds": [],
            "assigneeInboxTeamIds": [],
        }))])),
        expected
    );
    assert!(
        fc::flow_spec_schema()
            .safe_parse(&json!({
                "formatVersion": 1,
                "name": "Pipelines",
                "steps": [with_base(json!({ "action": "assign", "assigneeInboxTeamIds": ["12"] }))],
            }))
            .success
    );
}

#[test]
fn pipeline_spec_step_spec_validation_paths_rejects_fields_that_the_action_does_not_accept() {
    assert_eq!(
        issues_of(json!([with_base(
            json!({ "action": "removeFromPipeline", "stage": "New" })
        )])),
        vec![issue(
            "steps.0.stage",
            "action \"removeFromPipeline\" does not accept stage"
        )]
    );
}

#[test]
fn pipeline_spec_step_spec_validation_paths_rejects_onskip_for_addtopipeline_only() {
    assert_eq!(
        issues_of(json!([with_base(json!({
            "action": "addToPipeline",
            "onSkip": [{ "type": "addNote", "note": "x" }],
        }))])),
        vec![issue(
            "steps.0.onSkip",
            "action \"addToPipeline\" never skips, so onSkip would never run"
        )]
    );
    parse(json!([with_base(json!({
        "action": "moveToStage",
        "stage": "New",
        "onSkip": [{ "type": "addNote", "note": "x" }],
    }))]));
}

#[test]
fn pipeline_spec_step_spec_validation_paths_rejects_a_malformed_due_date() {
    assert!(
        !issues_of(json!([with_base(json!({
            "action": "changeDueDate",
            "dueDate": { "mode": "fixed", "at": "soon" },
        }))]))
        .is_empty()
    );
}

#[test]
fn pipeline_spec_step_spec_validation_paths_compiler_also_rejects_onskip_for_addtopipeline_when_parsing_was_bypassed()
 {
    let spec = json!({
        "formatVersion": 1,
        "name": "x",
        "steps": [{
            "type": "pipeline",
            "action": "addToPipeline",
            "pipeline": "sales",
            "onSkip": [{ "type": "addNote", "note": "x" }],
        }],
    });
    assert!(fc::compile_flow_spec(&spec, &ctx()).is_err());
}

// --- pipeline spec step > reference errors ---

#[test]
fn pipeline_spec_step_reference_errors_unknown_pipeline_lists_candidates() {
    let errors = errors_of(json!([with_base(
        json!({ "pipeline": "sale", "action": "removeFromPipeline" })
    )]));
    let error = &errors[0];
    assert_match_object(
        &error_json(error),
        &json!({ "path": "steps[0].pipeline", "code": "unknownPipeline" }),
    );
    assert!(
        error
            .candidates
            .as_ref()
            .unwrap()
            .contains(&"sales".to_string())
    );
}

#[test]
fn pipeline_spec_step_reference_errors_unknown_stage_lists_candidates_duplicate_names_are_ambiguous()
 {
    let unknown = &errors_of(json!([with_base(
        json!({ "action": "moveToStage", "stage": "Neww" })
    )]))[0];
    assert_match_object(
        &error_json(unknown),
        &json!({ "path": "steps[0].stage", "code": "unknownPipelineStage" }),
    );
    assert!(
        unknown
            .candidates
            .as_ref()
            .unwrap()
            .contains(&"New".to_string())
    );
    let dup = &errors_of(json!([with_base(
        json!({ "action": "moveToStage", "stage": "Dup" })
    )]))[0];
    assert_match_object(
        &error_json(dup),
        &json!({ "code": "ambiguousPipelineStage" }),
    );
}

#[test]
fn pipeline_spec_step_reference_errors_unknown_field_destination_and_due_date_field() {
    assert_match_object(
        &error_json(
            &errors_of(json!([with_base(
                json!({ "action": "setCustomField", "field": "bugdet", "value": "1" })
            )]))[0],
        ),
        &json!({ "path": "steps[0].field", "code": "unknownPipelineField" }),
    );
    assert_eq!(
        errors_of(json!([with_base(json!({
            "action": "addToPipeline",
            "fieldValues": [{ "field": "nope", "value": "1" }],
        }))]))[0]
            .path,
        "steps[0].fieldValues[0].field"
    );
    assert_match_object(
        &error_json(
            &errors_of(json!([with_base(json!({
                "action": "transfer",
                "destinationPipeline": "x",
                "destinationStage": "y",
            }))]))[0],
        ),
        &json!({ "path": "steps[0].destinationPipeline", "code": "unknownPipeline" }),
    );
    assert_match_object(
        &error_json(
            &errors_of(json!([with_base(json!({
                "action": "changeDueDate",
                "dueDate": { "mode": "customField", "customFieldName": "Pla" },
            }))]))[0],
        ),
        &json!({ "path": "steps[0].dueDate.customFieldName", "code": "unknownCustomField" }),
    );
    assert_match_object(
        &error_json(
            &errors_of(json!([with_base(json!({
                "action": "changeDueDate",
                "dueDate": { "mode": "customField", "customFieldName": "Plan" },
            }))]))[0],
        ),
        &json!({ "code": "invalidSpec" }),
    );
}

#[test]
fn pipeline_spec_step_reference_errors_stage_of_a_destination_pipeline_resolves_against_that_pipeline()
 {
    assert_match_object(
        &error_json(
            &errors_of(json!([with_base(json!({
                "action": "transfer",
                "destinationPipeline": "support",
                "destinationStage": "New",
            }))]))[0],
        ),
        &json!({ "path": "steps[0].destinationStage" }),
    );
}

// --- pipeline spec step > routing ---

fn routing_steps() -> Value {
    json!([
        with_base(json!({
            "id": "move",
            "action": "moveToStage",
            "stage": "New",
            "onError": [{ "type": "addNote", "note": "failed" }],
            "onSkip": [{ "type": "addNote", "note": "no item" }],
        })),
        { "type": "addNote", "note": "after" },
    ])
}

#[test]
fn pipeline_spec_step_routing_success_continues_to_the_next_step_error_and_skip_use_their_state_handles()
 {
    let compiled = compile(routing_steps()).expect("compiles");
    let pipeline_node = &compiled.nodes[0];
    assert_eq!(
        pipeline_node["type"], "performAction",
        "expected a performAction node"
    );
    let step = &pipeline_node["data"]["details"]["steps"][0];
    let handle = |state_type: &str| -> Value {
        step["states"]
            .as_array()
            .unwrap()
            .iter()
            .find(|state| state["stateType"] == state_type)
            .map(|state| state["id"].clone())
            .unwrap_or(Value::Null)
    };
    let outgoing: Vec<&Value> = compiled
        .edges
        .iter()
        .filter(|edge| edge["source"] == pipeline_node["id"])
        .collect();
    assert_eq!(outgoing.len(), 3);

    let note_of = |target_id: &Value| -> Option<String> {
        let node = compiled
            .nodes
            .iter()
            .find(|item| &item["id"] == target_id)?;
        (node["type"] == "addNotes").then(|| {
            node["data"]["details"]["beforeStep"]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
    };
    let target_for = |source_handle: &Value| -> Value {
        outgoing
            .iter()
            .find(|edge| &edge["sourceHandle"] == source_handle)
            .map(|edge| edge["target"].clone())
            .unwrap_or_else(|| json!(""))
    };
    assert_eq!(
        note_of(&target_for(&handle("error"))).as_deref(),
        Some("failed")
    );
    assert_eq!(
        note_of(&target_for(&handle("skip"))).as_deref(),
        Some("no item")
    );
    assert_eq!(
        note_of(&target_for(&pipeline_node["id"])).as_deref(),
        Some("after")
    );
    expect_publishable_steps(routing_steps());
}

#[test]
fn pipeline_spec_step_routing_is_not_terminal_and_nested_errors_carry_spec_paths() {
    let errors = errors_of(json!([with_base(json!({
        "action": "removeFromPipeline",
        "onError": [{ "type": "startFlow", "flowName": "Missing" }],
    }))]));
    assert_eq!(errors[0].path, "steps[0].onError[0].flowName");
}

#[test]
fn pipeline_spec_step_routing_onerror_can_goto_an_earlier_step() {
    expect_publishable_steps(json!([
        { "type": "addNote", "id": "start", "note": "hi" },
        with_base(json!({ "action": "removeFromPipeline", "onError": [{ "type": "goto", "targetId": "start" }] })),
    ]));
}
