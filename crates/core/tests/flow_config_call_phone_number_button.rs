//! Port of `packages/flow-config/__tests__/call-phone-number-button.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn call_button(phone_number: &str) -> Value {
    spread(
        &fc::button_step_default_fn(&json!({ "label": "Call us" })),
        json!({
            "buttonType": "callPhoneNumber",
            "beforeStep": spread(&fc::call_phone_number_step_default_fn(), json!({ "phoneNumber": phone_number })),
        }),
    )
}

fn template() -> Value {
    json!({ "id": "1", "name": "welcome", "language": "en", "params": {} })
}

fn code(name: &str) -> Option<&'static str> {
    fc::flow_validation_codes()[name].as_str()
}

fn first_message(result: &SafeParse) -> Option<String> {
    result
        .error
        .as_ref()
        .and_then(|e| e.issues.first())
        .map(|i| i.message.clone())
}

// --- callPhoneNumber button schema ---

#[test]
fn callphonenumber_button_schema_accepts_the_e_164_literal_s() {
    for phone_number in [
        "+14155552671",   // US
        "+16135550123",   // CA (shares +1)
        "+442079460958",  // GB
        "+4915123456789", // DE
        "+919876543210",  // IN
        "+5511987654321", // BR
        "+81312345678",   // JP
        "+390612345678",  // IT (keeps its leading 0)
        "+84912345678",   // VN
        "+80012345678",   // non-geographic
    ] {
        assert!(
            fc::button_step_schema()
                .safe_parse(&call_button(phone_number))
                .success,
            "{phone_number}"
        );
    }
}

#[test]
fn callphonenumber_button_schema_rejects_s() {
    for (name, phone_number) in [
        ("a national number", "0901234567"),
        ("junk", "call me"),
        ("an empty value", ""),
        ("more than 15 digits", "+1234567890123456"),
        ("a leading zero after the plus", "+0123456789"),
        ("a too-short number", "+12345"),
        (
            "a well-formed number with no real country plan",
            "+9991234567",
        ),
        (
            "a number with separators (must be stored as E.164)",
            "+1 415 555 2671",
        ),
    ] {
        let result = fc::button_step_schema().safe_parse(&call_button(phone_number));
        assert_eq!(
            first_message(&result).as_deref(),
            code("invalidPhoneNumber"),
            "{name}"
        );
    }
}

#[test]
fn callphonenumber_button_schema_accepts_an_expression_embedding_a_variable_even_beyond_a_literal_s_length()
 {
    let expression = "{{bot_field:123456789012345678}}";
    assert!(expression.len() > 16);
    assert!(
        fc::button_step_schema()
            .safe_parse(&call_button(expression))
            .success
    );
    assert!(
        fc::button_step_schema()
            .safe_parse(&call_button("{{contact.phone}}"))
            .success
    );
}

#[test]
fn callphonenumber_button_schema_rejects_an_expression_longer_than_the_expression_limit() {
    let too_long = format!(
        "{{{{contact.phone}}}}{}",
        "1".repeat(fc::PHONE_NUMBER_EXPRESSION_MAX)
    );
    assert!(
        !fc::button_step_schema()
            .safe_parse(&call_button(&too_long))
            .success
    );
}

#[test]
fn callphonenumber_button_schema_still_enforces_the_label_limit() {
    let result = fc::button_step_schema().safe_parse(&spread(
        &call_button("+14155552671"),
        json!({ "label": "x".repeat(fc::BUTTON_LABEL_MAX + 1) }),
    ));
    assert!(!result.success);
}

#[test]
fn callphonenumber_button_schema_cannot_carry_follow_up_steps() {
    let with_steps = spread(
        &call_button("+14155552671"),
        json!({ "steps": [fc::add_contact_tag_step_default_fn(&json!({ "tags": ["vip"] }))] }),
    );
    let result = fc::button_step_schema().safe_parse(&with_steps);

    assert!(!result.success);
    assert_eq!(result.paths(), vec!["steps"]);
}

// --- phone number helpers ---

#[test]
fn phone_number_helpers_a_literal_number_is_valid_only_when_it_is_a_real_number_already_in_e_164_form()
 {
    let is_valid = |value: &str| {
        fc::call_phone_number_step_schema()
            .field("phoneNumber")
            .safe_parse(&json!(value))
            .success
    };
    assert!(is_valid("+14155552671"));
    assert!(!is_valid("14155552671"));
    assert!(!is_valid("+1 415 555 2671"));
    assert!(!is_valid("+9991234567"));
}

#[test]
fn phone_number_helpers_compactphonenumber_drops_spaces_dashes_dots_and_brackets() {
    assert_eq!(
        fc::compact_phone_number(" +44 (20) 7946-0958 "),
        "+442079460958"
    );
    assert_eq!(fc::compact_phone_number("030.1234.5678"), "03012345678");
}

// --- callPhoneNumber button placement ---

#[test]
fn callphonenumber_button_placement_a_whatsapp_template_step_rejects_it() {
    let step = fc::send_wa_template_message_step_default_fn(&json!({ "template": template() }));
    let mut buttons = step["buttons"].as_array().cloned().unwrap_or_default();
    buttons.push(call_button("+14155552671"));
    let result = fc::send_wa_template_message_step_schema()
        .safe_parse(&spread(&step, json!({ "buttons": buttons })));
    assert_eq!(
        first_message(&result).as_deref(),
        code("buttonCallPhoneNumberUnsupported")
    );
}

#[test]
fn callphonenumber_button_placement_a_messenger_template_step_rejects_it() {
    let step =
        fc::send_messenger_template_message_step_default_fn(&json!({ "template": template() }));
    let result = fc::send_messenger_template_message_step_schema().safe_parse(&spread(
        &step,
        json!({ "buttons": [call_button("+14155552671")] }),
    ));
    assert_eq!(
        first_message(&result).as_deref(),
        code("buttonCallPhoneNumberUnsupported")
    );
}

#[test]
fn callphonenumber_button_placement_a_quick_reply_step_rejects_it() {
    let result = fc::send_quick_reply_step_schema().safe_parse(&json!({
        "id": "1",
        "stepType": "sendQuickReply",
        "message": "Pick one",
        "buttons": [call_button("+14155552671")],
    }));
    assert_eq!(
        first_message(&result).as_deref(),
        code("buttonCallPhoneNumberUnsupported")
    );
}

#[test]
fn callphonenumber_button_placement_a_send_message_node_s_quick_replies_reject_it() {
    let shape = fc::send_message_node_schema()
        .field("data")
        .field("details");
    let result = shape
        .field("quickReplies")
        .safe_parse(&json!([call_button("+14155552671")]));
    assert_eq!(
        first_message(&result).as_deref(),
        code("buttonCallPhoneNumberUnsupported")
    );
}

// --- callPhoneNumber public schema ---

#[test]
fn callphonenumber_public_schema_the_flowspec_json_schema_exposes_the_call_field() {
    assert!(
        z::to_json_schema(&fc::flow_spec_schema())
            .to_string()
            .contains("\"call\":{")
    );
}

// --- callPhoneNumber channel policy ---

#[test]
fn callphonenumber_channel_policy_only_channels_with_a_native_call_control_declare_it() {
    let native = |channel: &str| {
        fc::get_channel_flow_policy(Some(channel)).map(|p| p.native_button_types.clone())
    };
    for channel in ["messenger", "zalo", "webchat"] {
        assert_eq!(native(channel), Some(vec!["callPhoneNumber"]), "{channel}");
    }
    assert_eq!(native("telegram"), Some(vec!["copyText"]));
    for channel in ["instagram", "tiktok", "whatsapp", "omnichannel"] {
        assert_eq!(native(channel), Some(vec![]), "{channel}");
    }
}

// --- isNativeButtonType ---

#[test]
fn isnativebuttontype_follows_the_channel_policy() {
    assert!(fc::is_native_button_type(
        Some("messenger"),
        "callPhoneNumber"
    ));
    assert!(!fc::is_native_button_type(
        Some("telegram"),
        "callPhoneNumber"
    ));
}

#[test]
fn isnativebuttontype_is_false_for_s_so_the_reply_form_is_used() {
    for (name, channel) in [("an unknown channel", Some("fax")), ("no channel", None)] {
        assert!(
            !fc::is_native_button_type(channel, "callPhoneNumber"),
            "{name}"
        );
    }
}

// --- FlowSpec call button ---

fn spec(button: Value) -> Value {
    json!({ "formatVersion": 1, "name": "Call flow", "steps": [{ "type": "send", "text": "Hi", "buttons": [button] }] })
}

#[test]
fn flowspec_call_button_accepts_a_call_button() {
    assert!(
        fc::flow_spec_schema()
            .safe_parse(&spec(json!({ "text": "Call", "call": "+14155552671" })))
            .success
    );
}

#[test]
fn flowspec_call_button_rejects_an_invalid_number() {
    assert!(
        !fc::flow_spec_schema()
            .safe_parse(&spec(json!({ "text": "Call", "call": "0901234567" })))
            .success
    );
}

#[test]
fn flowspec_call_button_rejects_call_combined_with_then() {
    assert!(
        !fc::flow_spec_schema()
            .safe_parse(&spec(json!({
                "text": "Call",
                "call": "+14155552671",
                "then": [{ "type": "addNote", "note": "x" }],
            })))
            .success
    );
}

// --- callPhoneNumber in a published graph ---

fn node_with(channel: &str, steps: Value) -> Value {
    let node = fc::send_message_node_default_fn(&json!({
        "nodeProps": { "id": "1" },
        "detailProps": { "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": channel })) },
    }));
    spread(
        &node,
        json!({ "data": spread(&node["data"], json!({ "details": spread(&node["data"]["details"], json!({ "steps": steps })) })) }),
    )
}

fn publish(channel: &str, steps: Value) -> SafeParse {
    fc::publish_flow_schema()
        .safe_parse(&json!({ "nodes": [node_with(channel, steps)], "edges": [] }))
}

fn card(buttons: Value) -> Value {
    spread(
        &fc::send_card_step_default_fn(),
        json!({ "title": "Card", "buttons": buttons }),
    )
}

fn reply_button() -> Value {
    fc::button_step_default_fn(&json!({ "label": "More" }))
}

#[test]
fn callphonenumber_in_a_published_graph_a_text_step_with_a_literal_and_a_variable_call_button_publishes_on_every_channel()
 {
    let step = spread(
        &fc::send_text_step_default_fn(&json!({ "text": "Need help?" })),
        json!({ "buttons": [call_button("+14155552671"), call_button("{{contact.phone}}")] }),
    );
    for channel in [
        "messenger",
        "zalo",
        "webchat",
        "whatsapp",
        "telegram",
        "instagram",
        "tiktok",
        "omnichannel",
    ] {
        assert!(publish(channel, json!([step.clone()])).success, "{channel}");
    }
}

#[test]
fn callphonenumber_in_a_published_graph_a_whatsapp_carousel_whose_cards_each_carry_a_call_button_publishes()
 {
    let carousel = spread(
        &fc::send_carousel_step_default_fn(),
        json!({ "cards": [card(json!([call_button("+14155552671")])), card(json!([call_button("{{contact.phone}}")]))] }),
    );
    assert!(publish("whatsapp", json!([carousel])).success);
}

#[test]
fn callphonenumber_in_a_published_graph_a_whatsapp_carousel_mixing_a_call_button_and_a_reply_card_publishes()
 {
    let carousel = spread(
        &fc::send_carousel_step_default_fn(),
        json!({ "cards": [card(json!([call_button("+14155552671")])), card(json!([reply_button()]))] }),
    );
    assert!(publish("whatsapp", json!([carousel])).success);
}

#[test]
fn callphonenumber_in_a_published_graph_a_whatsapp_flow_step_rejects_a_call_button_as_its_cta() {
    let result = fc::whatsapp_flow_step_schema()
        .field("buttons")
        .safe_parse(&json!([call_button("+14155552671")]));
    assert_eq!(
        first_message(&result).as_deref(),
        code("buttonCallPhoneNumberUnsupported")
    );
}

#[test]
fn callphonenumber_in_a_published_graph_a_call_button_survives_export_import_with_its_number_and_no_steps()
 {
    let node = node_with(
        "messenger",
        json!([spread(
            &fc::send_text_step_default_fn(&json!({ "text": "Need help?" })),
            json!({ "buttons": [call_button("+14155552671")] }),
        )]),
    );
    let envelope = json!({
        "formatVersion": fc::FLOW_EXPORT_FORMAT_VERSION,
        "exportedAt": chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "source": { "workspaceId": "1", "flowId": "1" },
        "flows": [{
            "name": "Call flow",
            "active": true,
            "enableInInbox": false,
            "startNodeId": node["id"],
            "nodes": [node],
            "edges": [],
        }],
    });

    let result = fc::parse_flow_export(&serde_json::from_str(&envelope.to_string()).unwrap());

    let data = result.expect("parses");
    let button = &data["flows"][0]["nodes"][0]["data"]["details"]["steps"][0]["buttons"][0];
    assert_match_object(
        button,
        &json!({
            "buttonType": "callPhoneNumber",
            "beforeStep": { "phoneNumber": "+14155552671" },
            "steps": [],
        }),
    );
}
