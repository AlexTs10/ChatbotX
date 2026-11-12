//! Port of `packages/flow-config/__tests__/channel-policies.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::SendTextLengthProps;

fn media_support_by_channel(channel: &str) -> [&'static str; 5] {
    match channel {
        "api" => ["noButtons", "noButtons", "noButtons", "noButtons", "full"],
        "instagram" => ["noButtons", "noButtons", "noButtons", "noButtons", "full"],
        "messenger" => ["full", "full", "noButtons", "noButtons", "full"],
        "omnichannel" => ["full", "full", "full", "full", "full"],
        "smtp" => [
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
        ],
        "telegram" => ["full", "full", "full", "full", "full"],
        "threads" => [
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
        ],
        "tiktok" => [
            "noButtons",
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
        ],
        "webchat" => ["full", "full", "full", "full", "full"],
        "whatsapp" => [
            "full",
            "unsupported",
            "unsupported",
            "unsupported",
            "unsupported",
        ],
        "zalo" => ["full", "unsupported", "unsupported", "noButtons", "full"],
        other => panic!("unknown channel {other}"),
    }
}

const MEDIA_STEP_TYPES: [&str; 5] = ["sendImage", "sendVideo", "sendAudio", "sendFile", "sendGif"];

fn text_limit(channel: &str) -> usize {
    match channel {
        "api" => 6000,
        "instagram" => 1000,
        "messenger" => 2000,
        "omnichannel" => 1000,
        "smtp" => 6000,
        "telegram" => 4096,
        "threads" => 500,
        "tiktok" => 6000,
        "webchat" => 6000,
        "whatsapp" => 4096,
        "zalo" => 2000,
        other => panic!("unknown channel {other}"),
    }
}

fn collect_issues(node: &Value) -> Vec<(String, Value)> {
    let mut ctx = z::RefineCtx::default();
    fc::refine_steps_by_channel(std::slice::from_ref(node), &mut ctx);
    ctx.issues
        .into_iter()
        .map(|issue| {
            let path = Value::Array(issue.path.iter().map(PathSegment::to_json).collect());
            (issue.message.unwrap_or_default(), path)
        })
        .collect()
}

fn channel_types() -> Vec<String> {
    fc::deps::channel_types().enum_values()
}

#[test]
fn channel_flow_policies_defines_a_complete_policy_for_every_channel_and_step_type() {
    let mut policy_channels: Vec<String> = fc::channel_flow_policies()
        .keys()
        .map(|k| (*k).to_owned())
        .collect();
    policy_channels.sort();
    let mut expected = channel_types();
    expected.sort();
    assert_eq!(policy_channels, expected);

    let mut step_types = fc::step_types().enum_values();
    step_types.sort();
    for policy in fc::channel_flow_policies().values() {
        let mut keys: Vec<String> = policy.steps.keys().map(|k| (*k).to_owned()).collect();
        keys.sort();
        assert_eq!(keys, step_types);
    }
}

#[test]
fn channel_flow_policies_preserves_media_support_for_every_channel_and_media_step() {
    for channel in channel_types() {
        let expected = media_support_by_channel(&channel);

        for (index, step_type) in MEDIA_STEP_TYPES.iter().enumerate() {
            assert_eq!(
                fc::resolve_media_step_support(&channel, step_type),
                expected[index],
                "{channel}/{step_type}"
            );
        }
    }
}

#[test]
fn channel_flow_policies_preserves_text_limits() {
    for channel in channel_types() {
        let limits = fc::resolve_send_text_length_limits(SendTextLengthProps {
            channel: Some(&channel),
            has_buttons: None,
            has_quick_replies: None,
        });
        assert_eq!(limits.text, text_limit(&channel), "{channel}");
    }
}

#[test]
fn channel_flow_policies_keeps_tiktok_s_three_button_and_40_character_card_title_limits() {
    let tiktok = &fc::channel_flow_policies()["tiktok"];
    assert_eq!(tiktok.limits.button_count, 3);
    assert_eq!(tiktok.limits.card_title, Some(40));
}

fn node(channel: &str, quick_replies: Value, steps: Value) -> Value {
    fc::send_message_node_default_fn(&json!({
        "nodeProps": {},
        "detailProps": {
            "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": channel })),
            "quickReplies": quick_replies,
            "steps": steps,
        },
    }))
}

fn buttons(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|_| fc::button_step_default_fn(&json!({ "label": "Option" })))
            .collect(),
    )
}

#[test]
fn channel_flow_policies_blocks_unsupported_steps_on_every_channel_policy() {
    let node = node(
        "instagram",
        json!([]),
        json!([fc::send_card_step_default_fn()]),
    );

    assert!(collect_issues(&node).contains(&(
        "The instagram channel does not support sendCard.".to_owned(),
        json!([0, "data", "details", "steps", 0]),
    )));
}

#[test]
fn channel_flow_policies_allows_three_tiktok_buttons_and_rejects_a_fourth() {
    let make_node = |button_count: usize| {
        node(
            "tiktok",
            json!([]),
            json!([fc::send_text_step_default_fn(
                &json!({ "text": "Choose one", "buttons": buttons(button_count) })
            )]),
        )
    };

    assert!(collect_issues(&make_node(3)).is_empty());
    assert!(collect_issues(&make_node(4)).contains(&(
        "sendText exceeds the tiktok maximum of 3 buttons.".to_owned(),
        json!([0, "data", "details", "steps", 0, "buttons"]),
    )));
}

#[test]
fn channel_flow_policies_allows_ten_quick_replies_for_messenger_and_omnichannel_text_steps() {
    let make_node = |channel: &str| {
        node(
            channel,
            buttons(10),
            json!([fc::send_text_step_default_fn(
                &json!({ "text": "Choose one" })
            )]),
        )
    };

    assert!(collect_issues(&make_node("messenger")).is_empty());
    assert!(collect_issues(&make_node("omnichannel")).is_empty());
}

#[test]
fn channel_flow_policies_counts_tiktok_quick_replies_with_text_step_buttons() {
    let node = node(
        "tiktok",
        buttons(2),
        json!([fc::send_text_step_default_fn(
            &json!({ "text": "Choose one", "buttons": buttons(2) })
        )]),
    );

    assert!(collect_issues(&node).contains(&(
        "sendText exceeds the tiktok maximum of 3 buttons.".to_owned(),
        json!([0, "data", "details", "quickReplies"]),
    )));
}

#[test]
fn channel_flow_policies_uses_omnichannel_policy_for_legacy_channel_values() {
    let node = node(
        "legacy-channel",
        json!([]),
        json!([fc::send_card_step_default_fn()]),
    );

    assert!(collect_issues(&node).is_empty());
}
