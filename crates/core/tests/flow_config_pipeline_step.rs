//! Port of `packages/flow-config/__tests__/pipeline-step.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn default(action: &str, carry: Value) -> Value {
    fc::pipeline_step_default_fn(action, &carry)
}

#[test]
fn pipeline_step_schema_s_default_round_trips_and_is_an_action_step() {
    for action in fc::pipeline_step_actions().enum_values() {
        let base = default(&action, json!({ "pipelineId": "1" }));
        // Defaults leave the reference pickers empty; fill them as a user would.
        let mut step = base.clone();
        for (key, value) in [
            ("stageId", "2"),
            ("pipelineFieldId", "3"),
            ("destinationPipelineId", "4"),
        ] {
            if base.get(key).is_some() {
                step[key] = json!(value);
            }
        }
        assert_eq!(step["stepType"], "pipeline");
        assert_eq!(step["action"], action.as_str());
        assert_eq!(
            fc::pipeline_step_schema().parse(&step).unwrap(),
            step,
            "{action}"
        );
        assert!(any_ok(&fc::action_steps(), &step), "{action}");
    }
}

#[test]
fn pipeline_step_schema_carries_id_pipeline_and_routing_states_across_an_action_switch() {
    let first = default("addToPipeline", json!({ "pipelineId": "9" }));
    let next = default(
        "moveToStage",
        json!({ "id": first["id"], "pipelineId": first["pipelineId"], "states": first["states"] }),
    );
    assert_eq!(next["id"], first["id"]);
    assert_eq!(next["states"], first["states"]);
    let types: Vec<&Value> = next["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["stateType"])
        .collect();
    assert_eq!(types, vec!["success", "error", "skip"]);
}

#[test]
fn pipeline_step_schema_rejects_an_unknown_action_missing_action_fields_and_a_wrong_state_tuple() {
    let step = default("moveToStage", json!({ "pipelineId": "1" }));
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&spread(&step, json!({ "action": "nope" })))
            .success
    );
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&without(&step, &["stageId"]))
            .success
    );
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&spread(&step, json!({ "states": [step["states"][0]] })))
            .success
    );
}

#[test]
fn pipeline_step_schema_validates_the_due_date_modes() {
    let base = default("changeDueDate", json!({ "pipelineId": "1" }));
    let parse = |due_date: Value| {
        fc::pipeline_step_schema()
            .safe_parse(&spread(&base, json!({ "dueDate": due_date })))
            .success
    };
    assert!(parse(json!({ "mode": "none" })));
    assert!(parse(
        json!({ "mode": "customField", "customFieldId": "123" })
    ));
    assert!(parse(
        json!({ "mode": "fixed", "at": "2026-01-01T00:00:00Z" })
    ));
    assert!(!parse(json!({ "mode": "fixed", "at": "tomorrow" })));
    assert!(!parse(json!({ "mode": "customField" })));
    assert!(parse(json!({ "mode": "customField", "customFieldId": "" })));
    assert!(!parse(
        json!({ "mode": "customField", "customFieldId": "abc" })
    ));
    assert!(!parse(
        json!({ "mode": "customField", "customFieldId": rep("1", 19) })
    ));
}

#[test]
fn pipeline_step_schema_enforces_text_and_list_limits() {
    let title = default("changeTitle", json!({ "pipelineId": "1" }));
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&spread(&title, json!({ "title": rep("x", 201) })))
            .success
    );
    let comment = default("addComment", json!({ "pipelineId": "1" }));
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&spread(&comment, json!({ "text": rep("x", 2001) })))
            .success
    );
    let assign = default("assign", json!({ "pipelineId": "1" }));
    let ids: Vec<String> = (0..21).map(|i| i.to_string()).collect();
    assert!(
        !fc::pipeline_step_schema()
            .safe_parse(&spread(&assign, json!({ "assigneeMemberIds": ids })))
            .success
    );
}

#[test]
fn pipeline_step_schema_rejects_empty_or_non_numeric_reference_ids() {
    let schema = fc::pipeline_step_schema();
    let mv = default("moveToStage", json!({ "pipelineId": "1" }));
    let ok = spread(&mv, json!({ "stageId": "2" }));
    assert!(schema.safe_parse(&ok).success);
    for bad in [
        spread(&ok, json!({ "pipelineId": "abc" })),
        spread(&ok, json!({ "pipelineId": "" })),
        spread(&ok, json!({ "stageId": "" })),
        spread(&ok, json!({ "stageId": "1; drop" })),
        spread(&ok, json!({ "pipelineId": rep("1", 19) })),
    ] {
        assert!(!schema.safe_parse(&bad).success, "{bad}");
    }
    let assign = default("assign", json!({ "pipelineId": "1" }));
    assert!(
        !schema
            .safe_parse(&spread(&assign, json!({ "assigneeMemberIds": ["x"] })))
            .success
    );
    assert!(
        schema
            .safe_parse(&spread(&assign, json!({ "assigneeInboxTeamIds": ["5"] })))
            .success
    );
    let add = default("addToPipeline", json!({ "pipelineId": "1" }));
    assert!(
        schema
            .safe_parse(&spread(&add, json!({ "stageId": "" })))
            .success
    );
    assert!(
        !schema
            .safe_parse(&spread(&add, json!({ "stageId": "nope" })))
            .success
    );
}

#[test]
fn pipeline_step_schema_destination_stage_and_custom_field_ids_are_bigint_like_or_empty() {
    let schema = fc::pipeline_step_schema();
    let transfer = default("transfer", json!({ "pipelineId": "1" }));
    let ok = spread(&transfer, json!({ "destinationPipelineId": "2" }));
    for destination_stage_id in [None, Some(""), Some("7")] {
        let value = match destination_stage_id {
            None => without(&ok, &["destinationStageId"]),
            Some(id) => spread(&ok, json!({ "destinationStageId": id })),
        };
        assert!(
            schema.safe_parse(&value).success,
            "{destination_stage_id:?}"
        );
    }
    assert!(
        !schema
            .safe_parse(&spread(&ok, json!({ "destinationStageId": "x" })))
            .success
    );
    let field = default("setCustomField", json!({ "pipelineId": "1" }));
    assert!(schema.safe_parse(&field).success);
    assert!(
        !schema
            .safe_parse(&spread(&field, json!({ "pipelineFieldId": "x" })))
            .success
    );
}
