//! Port of `packages/flow-config/__tests__/pipeline-action.test.ts`.

mod flow_config_support;
use flow_config_support::*;

/// Fills the reference pickers a user would pick, which the defaults leave empty.
fn filled(base: &Value) -> Value {
    let mut out = base.clone();
    for (key, value) in [
        ("stageId", "2"),
        ("pipelineFieldId", "3"),
        ("destinationPipelineId", "4"),
    ] {
        if base.get(key).is_some() {
            out[key] = json!(value);
        }
    }
    out
}

fn actions() -> Vec<String> {
    fc::pipeline_step_actions().enum_values()
}

#[test]
fn pipeline_trigger_action_schema_both_unions_cover_exactly_the_same_actions() {
    let options = |schema: Schema| {
        let mut out: Vec<String> = schema
            .options()
            .iter()
            .map(|option| {
                option.field("action").literal_values()[0]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        out.sort();
        out
    };
    let mut expected = actions();
    expected.sort();

    assert_eq!(options(fc::pipeline_action_schema()), expected);
    assert_eq!(options(fc::pipeline_step_schema()), expected);
}

#[test]
fn pipeline_trigger_action_schema_s_default_round_trips_as_a_trigger_action_without_the_step_envelope()
 {
    for action in actions() {
        let base = fc::pipeline_action_default_fn(&action, &json!({ "pipelineId": "1" }));
        let trigger = filled(&base);

        assert_eq!(trigger["type"], "pipeline");
        assert_eq!(trigger["action"], action.as_str());
        assert!(trigger.get("id").is_none());
        assert!(trigger.get("states").is_none());
        assert!(trigger.get("stepType").is_none());
        assert_eq!(
            fc::pipeline_action_schema().parse(&trigger).unwrap(),
            trigger,
            "{action}"
        );
    }
}

#[test]
fn pipeline_trigger_action_schema_s_shares_its_fields_with_the_flow_step() {
    for action in actions() {
        let step = filled(&fc::pipeline_step_default_fn(
            &action,
            &json!({ "pipelineId": "1" }),
        ));
        let fields = without(&step, &["id", "stepType", "states"]);

        assert!(
            fc::pipeline_action_schema()
                .safe_parse(&spread(&fields, json!({ "type": "pipeline" })))
                .success,
            "{action}"
        );
        assert!(
            fc::pipeline_step_schema().safe_parse(&step).success,
            "{action}"
        );
    }
}

#[test]
fn pipeline_trigger_action_schema_a_stored_flow_step_still_parses_exactly_as_before() {
    let stored = json!({
        "id": "1",
        "stepType": "pipeline",
        "pipelineId": "10",
        "action": "addToPipeline",
        "title": "{{full_name}}",
        "description": "",
        "assigneeMemberIds": [],
        "assigneeInboxTeamIds": [],
        "status": "open",
        "dueDate": { "mode": "none" },
        "fieldValues": [{ "pipelineFieldId": "5", "value": "{{email}}" }],
        "states": fc::pipeline_step_default_fn("addToPipeline", &json!({}))["states"],
    });

    assert_eq!(fc::pipeline_step_schema().parse(&stored).unwrap(), stored);
}

#[test]
fn pipeline_trigger_action_schema_rejects_s() {
    let cases = [
        (
            "an unknown action",
            json!({ "type": "pipeline", "pipelineId": "1", "action": "nope" }),
        ),
        (
            "another action type",
            json!({ "type": "addTag", "pipelineId": "1", "action": "removeFromPipeline" }),
        ),
        (
            "a missing pipeline",
            json!({ "type": "pipeline", "action": "removeFromPipeline" }),
        ),
        (
            "a negative-limit title",
            json!({ "type": "pipeline", "pipelineId": "1", "action": "changeTitle", "title": rep("x", 201) }),
        ),
        (
            "a move without a stage",
            json!({ "type": "pipeline", "pipelineId": "1", "action": "moveToStage" }),
        ),
    ];
    for (label, action) in cases {
        assert!(
            !fc::pipeline_action_schema().safe_parse(&action).success,
            "{label}"
        );
    }
}
