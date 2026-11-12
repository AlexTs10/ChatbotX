//! Port of `packages/flow-config/__tests__/send-text-length-rules.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::SendTextLengthProps;

fn step(text: &str, buttons: Value) -> Value {
    json!({ "buttons": buttons, "id": "1", "stepType": "sendText", "text": text })
}

fn parse_for_channel(channel: &str, text: &str) -> SafeParse {
    fc::resolve_step_validator(&fc::send_text_validator(), channel)
        .safe_parse(&step(text, json!([])))
}

fn issue_codes(result: &SafeParse) -> Vec<String> {
    result.messages()
}

fn limits(
    channel: Option<&str>,
    has_buttons: Option<bool>,
    has_quick_replies: Option<bool>,
) -> fc::SendTextLengthLimits {
    fc::resolve_send_text_length_limits(SendTextLengthProps {
        channel,
        has_buttons,
        has_quick_replies,
    })
}

#[test]
fn resolvesendtextlengthlimits_text_limit_for_s_is_i() {
    let cases = [
        ("messenger", 2000),
        ("telegram", 4096),
        // The plain-text limit, not the 1024 of an interactive body.
        ("whatsapp", 4096),
        ("zalo", 2000),
        ("instagram", 1000),
        ("threads", 500),
        ("webchat", 6000),
        ("tiktok", 6000),
        // Any channel is reachable, so the budget stays at what the channels it
        // realistically lands on accept.
        ("omnichannel", 1000),
    ];
    for (channel, expected) in cases {
        assert_eq!(
            limits(Some(channel), None, None).text,
            expected,
            "{channel}"
        );
    }
}

#[test]
fn resolvesendtextlengthlimits_falls_back_to_omnichannel_for_an_unknown_or_missing_channel() {
    assert_eq!(limits(None, None, None).text, 1000);
    assert_eq!(limits(Some("myspace"), None, None).text, 1000);
}

#[test]
fn resolvesendtextlengthlimits_tiktok_drops_to_the_card_title_limit_once_buttons_are_attached() {
    assert_eq!(
        limits(Some("tiktok"), Some(true), None).text,
        fc::TIKTOK_CARD_TITLE_MAX
    );
    assert_eq!(limits(Some("tiktok"), Some(false), None).text, 6000);
}

#[test]
fn resolvesendtextlengthlimits_tiktok_drops_to_the_card_title_limit_for_quick_replies_alone() {
    assert_eq!(
        limits(Some("tiktok"), Some(false), Some(true)).text,
        fc::TIKTOK_CARD_TITLE_MAX
    );
}

#[test]
fn resolvesendtextlengthlimits_omnichannel_keeps_the_full_budget_even_with_buttons() {
    assert_eq!(limits(Some("omnichannel"), Some(true), None).text, 1000);
    assert_eq!(limits(Some("omnichannel"), None, Some(true)).text, 1000);
}

#[test]
fn resolvesendtextlengthlimits_label_limits_match_what_buttonstepschema_enforces() {
    let limits = limits(Some("messenger"), None, None);

    assert_eq!(limits.button_label, fc::BUTTON_LABEL_MAX);
    assert_eq!(limits.quick_reply_label, fc::BUTTON_LABEL_MAX);
}

#[test]
fn sendtextstepschema_caps_at_the_widest_limit_any_channel_accepts() {
    assert_eq!(fc::SEND_TEXT_MAX, 6000);
    assert!(
        fc::send_text_step_schema()
            .safe_parse(&step(&rep("a", 6000), json!([])))
            .success
    );
    assert!(
        !fc::send_text_step_schema()
            .safe_parse(&step(&rep("a", 6001), json!([])))
            .success
    );
}

#[test]
fn sendtextstepschema_counts_by_code_point_so_an_emoji_is_one_character() {
    let result = fc::send_text_step_schema().safe_parse(&step(&rep("👋", 6000), json!([])));

    assert!(result.success);
    assert!(
        !fc::send_text_step_schema()
            .safe_parse(&step(&rep("👋", 6001), json!([])))
            .success
    );
}

#[test]
fn sendtextstepschema_raises_a_mapped_code_rather_than_a_raw_zod_message() {
    let result = fc::send_text_step_schema().safe_parse(&step(&rep("a", 6001), json!([])));

    assert!(!result.success);
    assert!(issue_codes(&result).contains(&"sendTextTooLongForChannel".to_owned()));
}

#[test]
fn sendtextvalidator_length_rules_s_accepts_its_limit_and_rejects_one_past_it() {
    let cases = [
        ("messenger", 2000),
        ("telegram", 4096),
        ("whatsapp", 4096),
        ("instagram", 1000),
        ("threads", 500),
        ("omnichannel", 1000),
    ];
    for (channel, max) in cases {
        assert!(
            parse_for_channel(channel, &rep("a", max)).success,
            "{channel}"
        );

        let over_limit = parse_for_channel(channel, &rep("a", max + 1));

        assert!(!over_limit.success, "{channel}");
        assert!(
            issue_codes(&over_limit).contains(&"sendTextTooLongForChannel".to_owned()),
            "{channel}"
        );
    }
}

#[test]
fn sendtextvalidator_length_rules_every_channel_has_a_length_rule_of_its_own() {
    // A channel missing from the map would silently fall back to the
    // omnichannel base and be validated against the wrong limit.
    let telegram_text = rep("a", 4096);

    assert!(parse_for_channel("telegram", &telegram_text).success);
    assert!(!parse_for_channel("omnichannel", &telegram_text).success);
}

#[test]
fn sendtextvalidator_length_rules_a_text_over_the_schema_ceiling_is_rejected_on_any_channel() {
    assert!(!parse_for_channel("telegram", &rep("a", 6001)).success);
}

#[test]
fn sendtextvalidator_length_rules_a_text_over_the_schema_ceiling_raises_the_code_once_not_twice() {
    // The field cap and the channel rule share one code, so without a guard the
    // author would see the identical message twice at the same path.
    assert_eq!(
        issue_codes(&parse_for_channel("telegram", &rep("a", 6001))),
        vec!["sendTextTooLongForChannel"]
    );
}

#[test]
fn sendtextvalidator_length_rules_tiktok_keeps_its_card_title_rule_and_raises_it_once_not_twice() {
    let result = fc::resolve_step_validator(&fc::send_text_validator(), "tiktok").safe_parse(&step(
        &rep("a", fc::TIKTOK_CARD_TITLE_MAX + 1),
        json!([{ "beforeStep": null, "buttonType": null, "id": "2", "label": "Yes", "steps": [] }]),
    ));

    assert!(!result.success);
    assert_eq!(issue_codes(&result), vec!["tiktokCardTitleTooLong"]);
}

#[test]
fn countmessagecharacters_counts_an_emoji_built_from_a_surrogate_pair_as_one_character() {
    assert_eq!(fc::count_message_characters(Some("👋")), 1);
    assert_eq!(fc::count_message_characters(Some("hi 👋")), 4);
}

#[test]
fn countmessagecharacters_treats_null_and_undefined_as_empty() {
    assert_eq!(fc::count_message_characters(None), 0);
}
