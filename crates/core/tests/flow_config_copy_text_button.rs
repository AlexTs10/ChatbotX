//! Port of `packages/flow-config/__tests__/copy-text-button.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn copy_button(text: &str) -> Value {
    spread(
        &fc::button_step_default_fn(&json!({ "label": "Copiar PIX" })),
        json!({
            "buttonType": "copyText",
            "beforeStep": spread(&fc::copy_text_step_default_fn(), json!({ "text": text })),
        }),
    )
}

fn copy_text_unsupported() -> &'static Value {
    &fc::flow_validation_codes()["buttonCopyTextUnsupported"]
}

fn first_message(result: &SafeParse) -> Option<String> {
    result
        .error
        .as_ref()
        .and_then(|e| e.issues.first())
        .map(|i| i.message.clone())
}

// --- copyText button schema ---

#[test]
fn copytext_button_schema_accepts_a_literal_up_to_the_copy_limit() {
    assert!(
        fc::button_step_schema()
            .safe_parse(&copy_button(&"x".repeat(fc::COPY_TEXT_MAX)))
            .success
    );
}

#[test]
fn copytext_button_schema_rejects_a_literal_longer_than_the_copy_limit() {
    assert!(
        !fc::button_step_schema()
            .safe_parse(&copy_button(&"x".repeat(fc::COPY_TEXT_MAX + 1)))
            .success
    );
}

#[test]
fn copytext_button_schema_rejects_an_empty_text() {
    assert!(
        !fc::button_step_schema()
            .safe_parse(&copy_button("  "))
            .success
    );
}

#[test]
fn copytext_button_schema_accepts_a_variable_expression_longer_than_the_copy_limit() {
    let text = format!("{{{{bot_field:123}}}}{}", "x".repeat(fc::COPY_TEXT_MAX));
    assert!(
        fc::button_step_schema()
            .safe_parse(&copy_button(&text))
            .success
    );
}

#[test]
fn copytext_button_schema_caps_a_variable_expression() {
    let text = format!(
        "{{{{pix_code}}}}{}",
        "x".repeat(fc::COPY_TEXT_EXPRESSION_MAX)
    );
    assert!(
        !fc::button_step_schema()
            .safe_parse(&copy_button(&text))
            .success
    );
}

#[test]
fn copytext_button_schema_cannot_run_follow_up_steps() {
    let button = spread(
        &copy_button("abc"),
        json!({ "steps": [{ "id": "9", "stepType": "addContactTag", "tagIds": [] }] }),
    );
    assert!(!fc::button_step_schema().safe_parse(&button).success);
}

#[test]
fn copytext_button_schema_labels_follow_the_shared_label_limit() {
    let button = spread(
        &copy_button("abc"),
        json!({ "label": "x".repeat(fc::BUTTON_LABEL_MAX + 1) }),
    );
    assert!(!fc::button_step_schema().safe_parse(&button).success);
}

// --- isCopyableText ---

#[test]
fn iscopyabletext_s() {
    for (name, text, expected) in [
        ("an empty text", String::new(), false),
        ("a PIX-sized text", "x".repeat(150), true),
        ("the limit", "x".repeat(fc::COPY_TEXT_MAX), true),
        ("past the limit", "x".repeat(fc::COPY_TEXT_MAX + 1), false),
    ] {
        assert_eq!(fc::is_copyable_text(&text), expected, "{name}");
    }
}

// --- copyText channel policy ---

#[test]
fn copytext_channel_policy_only_telegram_copies_natively() {
    assert!(fc::is_native_button_type(Some("telegram"), "copyText"));
    for channel in [
        "messenger",
        "zalo",
        "webchat",
        "instagram",
        "tiktok",
        "whatsapp",
        "omnichannel",
    ] {
        assert!(
            !fc::is_native_button_type(Some(channel), "copyText"),
            "{channel}"
        );
    }
}

#[test]
fn copytext_channel_policy_telegram_takes_up_to_10_buttons() {
    assert_eq!(
        fc::get_channel_flow_policy(Some("telegram")).map(|p| p.limits.button_count),
        Some(10)
    );
}

#[test]
fn copytext_channel_policy_sendsasnativecopybutton_needs_a_native_channel_and_a_copyable_text() {
    assert!(fc::sends_as_native_copy_button(
        Some("telegram"),
        &copy_button("abc")
    ));
    assert!(!fc::sends_as_native_copy_button(
        Some("messenger"),
        &copy_button("abc")
    ));
    assert!(!fc::sends_as_native_copy_button(
        Some("telegram"),
        &copy_button("")
    ));
    assert!(!fc::sends_as_native_copy_button(
        Some("telegram"),
        &copy_button(&"x".repeat(fc::COPY_TEXT_MAX + 1))
    ));
    assert!(!fc::sends_as_native_copy_button(
        Some("telegram"),
        &fc::button_step_default_fn(&json!({ "label": "x" }))
    ));
}

// --- copyText button placement ---

#[test]
fn copytext_button_placement_a_quick_reply_step_rejects_it() {
    let result = fc::send_quick_reply_step_schema().safe_parse(&json!({
        "id": "1",
        "stepType": "sendQuickReply",
        "message": "Pick one",
        "buttons": [copy_button("abc")],
    }));
    assert_eq!(
        first_message(&result).as_deref(),
        copy_text_unsupported().as_str()
    );
}

#[test]
fn copytext_button_placement_a_send_message_node_s_quick_replies_reject_it() {
    let shape = fc::send_message_node_schema()
        .field("data")
        .field("details");
    let result = shape
        .field("quickReplies")
        .safe_parse(&json!([copy_button("abc")]));
    assert_eq!(
        first_message(&result).as_deref(),
        copy_text_unsupported().as_str()
    );
}

#[test]
fn copytext_button_placement_a_whatsapp_flow_step_rejects_it_as_its_cta() {
    let result = fc::whatsapp_flow_step_schema()
        .field("buttons")
        .safe_parse(&json!([copy_button("abc")]));
    assert_eq!(
        first_message(&result).as_deref(),
        copy_text_unsupported().as_str()
    );
}

// --- copyText in a published graph ---

fn publish(channel: &str, buttons: Vec<Value>) -> SafeParse {
    let node = fc::send_message_node_default_fn(&json!({
        "nodeProps": { "id": "1" },
        "detailProps": { "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": channel })) },
    }));
    let step = spread(
        &fc::send_text_step_default_fn(&json!({ "text": "Pague com PIX" })),
        json!({ "buttons": buttons }),
    );
    fc::publish_flow_schema().safe_parse(&json!({
        "nodes": [spread(
            &node,
            json!({ "data": spread(&node["data"], json!({ "details": spread(&node["data"]["details"], json!({ "steps": [step] })) })) }),
        )],
        "edges": [],
    }))
}

#[test]
fn copytext_in_a_published_graph_a_text_step_with_a_copy_button_publishes_on_every_channel() {
    for channel in [
        "telegram",
        "messenger",
        "zalo",
        "webchat",
        "whatsapp",
        "instagram",
        "tiktok",
        "omnichannel",
    ] {
        assert!(
            publish(channel, vec![copy_button("{{pix_code}}")]).success,
            "{channel}"
        );
    }
}

#[test]
fn copytext_in_a_published_graph_telegram_publishes_10_buttons_but_not_11() {
    let buttons = |count: usize| (0..count).map(|_| copy_button("abc")).collect::<Vec<_>>();
    assert!(publish("telegram", buttons(10)).success);
    assert!(!publish("telegram", buttons(11)).success);
}

#[test]
fn copytext_in_a_published_graph_other_channels_keep_the_3_button_limit() {
    let buttons = (0..4).map(|_| copy_button("abc")).collect();
    assert!(!publish("messenger", buttons).success);
}

// --- FlowSpec copy button ---

fn spec(button: Value) -> Value {
    json!({ "formatVersion": 1, "name": "PIX flow", "steps": [{ "type": "send", "text": "Hi", "buttons": [button] }] })
}

#[test]
fn flowspec_copy_button_the_flowspec_json_schema_exposes_the_copy_field() {
    assert!(
        z::to_json_schema(&fc::flow_spec_schema())
            .to_string()
            .contains("\"copy\":{")
    );
}

#[test]
fn flowspec_copy_button_accepts_a_copy_button() {
    assert!(
        fc::flow_spec_schema()
            .safe_parse(&spec(json!({ "text": "Copiar", "copy": "{{pix_code}}" })))
            .success
    );
}

#[test]
fn flowspec_copy_button_rejects_copy_combined_with_then() {
    assert!(
        !fc::flow_spec_schema()
            .safe_parse(&spec(json!({ "text": "Copiar", "copy": "abc", "then": [{ "type": "addNote", "note": "x" }] })))
            .success
    );
}

#[test]
fn flowspec_copy_button_rejects_copy_combined_with_call() {
    assert!(
        !fc::flow_spec_schema()
            .safe_parse(&spec(
                json!({ "text": "Copiar", "copy": "abc", "call": "+14155552671" })
            ))
            .success
    );
}
