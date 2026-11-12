//! Port of `packages/flow-config/__tests__/ai-segment-image.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::HashSet;

fn valid_step() -> Value {
    fc::ai_segment_image_default_fn(
        &json!({ "inputFieldId": "1", "target": "product box", "outputFieldId": "2" }),
    )
}

fn parse(v: Value) -> SafeParse {
    fc::ai_segment_image_schema().safe_parse(&v)
}

fn published(steps: Value) -> SafeParse {
    let node = fc::perform_action_node_default_fn(&json!({ "detailProps": { "steps": steps } }));
    fc::publish_flow_schema().safe_parse(&json!({ "nodes": [node], "edges": [] }))
}

#[test]
fn aisegmentimage_step_defaults_to_largest_match_and_success_skip_error_states() {
    let step = fc::ai_segment_image_default_fn(&json!({}));

    assert_eq!(step["stepType"], "aiSegmentImage");
    assert_eq!(step["matchSelection"], "largest");
    assert_eq!(step["inputFieldId"], "");
    assert_eq!(step["outputFieldId"], "");
    assert!(step.get("detailsOutputFieldId").is_none());
    let types: Vec<&Value> = step["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["stateType"])
        .collect();
    assert_eq!(types, vec!["success", "skip", "error"]);
}

#[test]
fn aisegmentimage_step_accepts_every_setting() {
    let parsed = fc::ai_segment_image_schema()
        .parse(&spread(
            &valid_step(),
            json!({ "matchSelection": "all", "detailsOutputFieldId": "3" }),
        ))
        .unwrap();

    assert_match_object(
        &parsed,
        &json!({ "matchSelection": "all", "detailsOutputFieldId": "3" }),
    );
}

#[test]
fn aisegmentimage_step_accepts_no_details_output() {
    assert!(parse(valid_step()).success);
}

#[test]
fn aisegmentimage_step_accepts_replacing_the_source_image_with_the_output() {
    assert!(parse(spread(&valid_step(), json!({ "outputFieldId": "1" }))).success);
}

#[test]
fn aisegmentimage_step_rejects_missing_fields() {
    assert!(!parse(fc::ai_segment_image_default_fn(&json!({}))).success);
    assert!(!parse(spread(&valid_step(), json!({ "inputFieldId": "" }))).success);
    assert!(!parse(spread(&valid_step(), json!({ "outputFieldId": "" }))).success);
}

#[test]
fn aisegmentimage_step_rejects_a_whitespace_or_overlong_target() {
    assert!(!parse(spread(&valid_step(), json!({ "target": "   " }))).success);
    assert!(
        !parse(spread(
            &valid_step(),
            json!({ "target": rep("a", fc::AI_SEGMENT_IMAGE_TARGET_MAX_LENGTH + 1) })
        ))
        .success
    );
}

#[test]
fn aisegmentimage_step_trims_the_target() {
    let parsed = fc::ai_segment_image_schema()
        .parse(&spread(&valid_step(), json!({ "target": "  red car  " })))
        .unwrap();

    assert_eq!(parsed["target"], "red car");
}

#[test]
fn aisegmentimage_step_rejects_an_invalid_match_selection() {
    assert!(!parse(spread(&valid_step(), json!({ "matchSelection": "first" }))).success);
}

#[test]
fn aisegmentimage_step_drops_the_outputformat_key_of_steps_saved_before_crop_only_output() {
    let parsed = fc::ai_segment_image_schema()
        .parse(&spread(&valid_step(), json!({ "outputFormat": "mask" })))
        .unwrap();

    assert!(parsed.get("outputFormat").is_none());
}

#[test]
fn aisegmentimage_step_rejects_details_sharing_the_output_image_field() {
    let result = parse(spread(
        &valid_step(),
        json!({ "detailsOutputFieldId": "2" }),
    ));

    assert!(!result.success);
    assert_eq!(
        result.issues()[0].path_json(),
        json!(["detailsOutputFieldId"])
    );
}

#[test]
fn aisegmentimage_step_rejects_details_sharing_the_source_image_field() {
    let result = parse(spread(
        &valid_step(),
        json!({ "detailsOutputFieldId": "1" }),
    ));

    assert!(!result.success);
    assert_eq!(
        result.issues()[0].path_json(),
        json!(["detailsOutputFieldId"])
    );
}

#[test]
fn aisegmentimage_step_rejects_states_in_the_wrong_order() {
    let step = valid_step();
    let states = step["states"].as_array().unwrap().clone();

    assert!(
        !parse(spread(
            &step,
            json!({ "states": [states[0], states[2], states[1]] })
        ))
        .success
    );
}

#[test]
fn aisegmentimage_step_is_accepted_inside_a_published_flow() {
    assert!(published(json!([valid_step()])).success);
}

#[test]
fn aisegmentimage_step_a_malformed_step_fails_published_flow_validation() {
    assert!(
        !published(json!([spread(
            &valid_step(),
            json!({ "detailsOutputFieldId": "2" })
        )]))
        .success
    );
}

#[test]
fn aisegmentimage_step_accepts_account_field_bot_field_references_in_every_field_slot() {
    let parsed = parse(spread(
        &valid_step(),
        json!({
            "inputFieldId": "bot_field:11",
            "outputFieldId": "bot_field:12",
            "detailsOutputFieldId": "bot_field:13",
        }),
    ));

    assert!(parsed.success);
}

#[test]
fn aisegmentimage_step_rejects_a_malformed_bot_field_token_in_s() {
    for field in ["inputFieldId", "outputFieldId", "detailsOutputFieldId"] {
        let mut step = valid_step();
        step[field] = json!("bot_field:abc");
        assert!(!parse(step).success, "{field}");
    }
}

#[test]
fn aisegmentimage_step_rejects_bot_field_tokens_that_collide_like_plain_ids() {
    assert!(
        !parse(spread(
            &valid_step(),
            json!({ "outputFieldId": "bot_field:12", "detailsOutputFieldId": "bot_field:12" }),
        ))
        .success
    );
}

#[test]
fn aisegmentimage_step_accepts_a_target_of_exactly_the_maximum_length() {
    assert!(
        parse(spread(
            &valid_step(),
            json!({ "target": rep("a", fc::AI_SEGMENT_IMAGE_TARGET_MAX_LENGTH) })
        ))
        .success
    );
}

#[test]
fn aisegmentimage_step_rejects_an_empty_string_details_field_instead_of_treating_it_as_unset() {
    assert!(!parse(spread(&valid_step(), json!({ "detailsOutputFieldId": "" }))).success);
}

#[test]
fn aisegmentimage_step_requires_the_states_tuple() {
    assert!(!parse(without(&valid_step(), &["states"])).success);
}

#[test]
fn aisegmentimage_step_rejects_a_wrong_step_type_discriminator() {
    assert!(!parse(spread(&valid_step(), json!({ "stepType": "aiEditImage" }))).success);
}

#[test]
fn aisegmentimage_step_drops_provider_model_keys_instead_of_persisting_them() {
    let parsed = fc::ai_segment_image_schema()
        .parse(&spread(
            &valid_step(),
            json!({ "model": "sam-3", "provider": "openai" }),
        ))
        .unwrap();

    assert!(parsed.get("provider").is_none());
    assert!(parsed.get("model").is_none());
}

#[test]
fn aisegmentimage_step_defaults_give_every_state_a_distinct_id() {
    let step = fc::ai_segment_image_default_fn(&json!({}));
    let ids: HashSet<&str> = step["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();

    assert_eq!(ids.len(), 3);
}

#[test]
fn aisegmentimage_step_caller_props_cannot_override_the_step_type_or_the_states() {
    let step = fc::ai_segment_image_default_fn(&json!({ "stepType": "aiEditImage", "states": [] }));

    assert_eq!(step["stepType"], "aiSegmentImage");
    assert_eq!(step["states"].as_array().unwrap().len(), 3);
}

#[test]
fn aisegmentimage_step_runs_on_the_worker_so_every_channel_supports_it() {
    for policy in fc::channel_flow_policies().values() {
        assert_eq!(policy.steps["aiSegmentImage"], "full");
    }
}

#[test]
fn aisegmentimage_step_a_published_flow_keeps_the_segment_step_next_to_other_actions() {
    let result = published(json!([
        valid_step(),
        spread(&valid_step(), json!({ "outputFieldId": "1" }))
    ]));
    assert!(result.success);
    let data = result.data.unwrap();
    let node = &data["nodes"][0];
    let steps: Vec<&Value> = if node["type"] == "performAction" {
        node["data"]["details"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["stepType"])
            .collect()
    } else {
        vec![]
    };
    assert_eq!(steps, vec!["aiSegmentImage", "aiSegmentImage"]);
}
