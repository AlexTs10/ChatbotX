//! Port of `packages/flow-config/__tests__/get-user-data-step.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn reply_format(key: &str) -> Value {
    fc::reply_format()[key].clone()
}

fn make_step(overrides: Value) -> Value {
    spread(
        &spread(
            &fc::get_user_data_step_default_fn(&json!({})),
            json!({
                "message": "What is your phone number?",
                "replyFormat": reply_format("phone"),
                "outputFieldId": "field-1",
            }),
        ),
        overrides,
    )
}

#[test]
fn getuserdatastepschema_showhint_defaults_showhint_to_false() {
    assert_eq!(
        fc::get_user_data_step_default_fn(&json!({}))["showHint"],
        false
    );
}

#[test]
fn getuserdatastepschema_showhint_accepts_showhint_true() {
    let result =
        fc::get_user_data_step_schema().safe_parse(&make_step(json!({ "showHint": true })));

    assert!(result.success);
    assert_eq!(result.data.unwrap()["showHint"], true);
}

#[test]
fn getuserdatastepschema_showhint_accepts_a_saved_step_without_showhint() {
    let legacy_step = without(&make_step(json!({})), &["showHint"]);

    let result = fc::get_user_data_step_schema().safe_parse(&legacy_step);

    assert!(result.success);
    assert!(result.data.unwrap().get("showHint").is_none());
}

#[test]
fn getuserdatastepschema_showhint_rejects_a_non_boolean_showhint() {
    let result =
        fc::get_user_data_step_schema().safe_parse(&make_step(json!({ "showHint": "yes" })));

    assert!(!result.success);
}

#[test]
fn contact_hint_quick_reply_payload_round_trips_the_full_value() {
    let payload = fc::encode_contact_hint_quick_reply_payload("taubancam171995.1@gmail.com");

    assert_eq!(
        fc::decode_contact_hint_quick_reply_payload(Some(&payload)).as_deref(),
        Some("taubancam171995.1@gmail.com")
    );
}

#[test]
fn contact_hint_quick_reply_payload_returns_null_for_other_payloads_empty_values_and_null() {
    assert!(fc::decode_contact_hint_quick_reply_payload(Some("1234567890123")).is_none());
    assert!(
        fc::decode_contact_hint_quick_reply_payload(Some(
            &fc::encode_contact_hint_quick_reply_payload("")
        ))
        .is_none()
    );
    assert!(fc::decode_contact_hint_quick_reply_payload(None).is_none());
}

#[test]
fn contact_hint_quick_reply_payload_is_never_decodable_as_a_flow_action() {
    for value in ["0708122593", "1234567890123", "a:b@example.com"] {
        let payload = fc::encode_contact_hint_quick_reply_payload(value);

        assert!(
            fc::decode_button_payload(&payload, true).is_none(),
            "{value}"
        );
    }
}

fn sparse_node() -> Value {
    json!({
        "id": "1783440960229",
        "stepType": "getUserData",
        "message": "What is your email?",
        "replyFormat": reply_format("email"),
        "outputFieldId": "field-1",
        "states": fc::get_user_data_step_default_fn(&json!({}))["states"],
    })
}

#[test]
fn getuserdatastepschema_auto_skip_and_skip_button_defaults_a_public_api_node_can_omit_every_optional_knob()
 {
    let result = fc::get_user_data_step_schema().safe_parse(&sparse_node());

    assert!(result.success);
    assert_match_object(
        &result.data.unwrap(),
        &json!({
            "retryMessage": "",
            "skipButtonLabel": "Skip",
            "autoSkip": false,
            "autoSkipTimeUnit": "minutes",
            "autoSkipTimeValue": 3,
            "autoSkipFailAttempts": 3,
        }),
    );
}

#[test]
fn getuserdatastepschema_auto_skip_and_skip_button_defaults_keeps_stored_values_including_an_empty_skip_button_label()
 {
    let result = fc::get_user_data_step_schema().safe_parse(&spread(
        &sparse_node(),
        json!({
            "skipButtonLabel": "",
            "autoSkip": true,
            "autoSkipTimeUnit": "seconds",
            "autoSkipTimeValue": "30",
            "autoSkipFailAttempts": 2,
        }),
    ));

    assert_match_object(
        &result.data.unwrap(),
        &json!({
            "skipButtonLabel": "",
            "autoSkip": true,
            "autoSkipTimeUnit": "seconds",
            "autoSkipTimeValue": 30,
            "autoSkipFailAttempts": 2,
        }),
    );
}

#[test]
fn getuserdatastepschema_auto_skip_and_skip_button_defaults_still_accepts_a_long_stored_label_clamped_only_on_the_wire()
 {
    let result = fc::get_user_data_step_schema().safe_parse(&spread(
        &sparse_node(),
        json!({ "skipButtonLabel": rep("x", 255) }),
    ));

    assert!(result.success);
}

#[test]
fn getuserdatastepschema_auto_skip_and_skip_button_defaults_defaultfn_takes_the_builder_s_ui_language_label_but_never_its_id()
 {
    let step =
        fc::get_user_data_step_default_fn(&json!({ "skipButtonLabel": "Bỏ qua", "id": "1" }));

    assert_eq!(step["skipButtonLabel"], "Bỏ qua");
    assert_ne!(step["id"], "1");
    assert_eq!(
        fc::get_user_data_step_default_fn(&json!({}))["skipButtonLabel"],
        "Skip"
    );
}

fn publish_with(step: Value) -> SafeParse {
    let node = fc::send_message_node_default_fn(&json!({ "nodeProps": { "id": "1" } }));
    let node = spread(
        &node,
        json!({ "data": spread(&node["data"], json!({ "details": spread(&node["data"]["details"], json!({ "steps": [step] })) })) }),
    );
    fc::publish_flow_schema().safe_parse(&json!({ "nodes": [node], "edges": [] }))
}

#[test]
fn get_user_data_in_a_published_graph_public_api_an_agent_can_omit_every_optional_knob_and_still_publish()
 {
    let result = publish_with(sparse_node());

    assert!(result.success);
}

#[test]
fn get_user_data_in_a_published_graph_public_api_the_omitted_knobs_arrive_with_their_defaults_skip_button_on_auto_skip_off()
 {
    let result = publish_with(sparse_node());

    let data = result.data.unwrap();
    let published = &data["nodes"][0]["data"]["details"];
    assert_match_object(
        &published["steps"][0],
        &json!({
            "skipButtonLabel": "Skip",
            "autoSkip": false,
            "autoSkipTimeUnit": "minutes",
            "autoSkipTimeValue": 3,
            "autoSkipFailAttempts": 3,
            "retryMessage": "",
        }),
    );
}

#[test]
fn get_user_data_in_a_published_graph_public_api_an_out_of_range_knob_is_still_rejected() {
    let result = publish_with(spread(&sparse_node(), json!({ "autoSkipTimeValue": 0 })));

    assert!(!result.success);
}
