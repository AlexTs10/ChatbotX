//! Port of `packages/flow-config/__tests__/quick-reply-settings.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use chatbotx_core::TestClock;

fn node_target() -> Value {
    json!({
        "buttonType": "startAnotherNode",
        "beforeStep": fc::start_another_node_step_default_fn(&json!({ "nodeId": "2", "viewOnly": true })),
    })
}

fn quick_reply() -> Value {
    json!({ "id": "10", "label": "Yes", "buttonType": null, "beforeStep": null, "steps": [] })
}

fn make_node(
    quick_replies: Option<Value>,
    steps: Option<Value>,
    quick_reply_settings: Option<Value>,
) -> Value {
    let node = fc::send_message_node_default_fn(
        &json!({ "nodeProps": { "id": "1", "position": { "x": 0, "y": 0 } } }),
    );
    let mut details = spread(
        &node["data"]["details"],
        json!({
            "steps": steps.unwrap_or_else(|| json!([fc::send_text_step_default_fn(&json!({ "id": "3", "text": "Hi" }))])),
            "quickReplies": quick_replies.unwrap_or_else(|| json!([quick_reply()])),
        }),
    );
    if let Some(settings) = quick_reply_settings {
        details["quickReplySettings"] = settings;
    }
    spread(
        &node,
        json!({ "data": spread(&node["data"], json!({ "details": details })) }),
    )
}

fn issue_paths(node: &Value) -> Vec<String> {
    fc::send_message_node_schema().safe_parse(node).paths()
}

fn issue_messages(node: &Value) -> Vec<String> {
    fc::send_message_node_schema().safe_parse(node).messages()
}

fn with_settings(settings: Value) -> Value {
    make_node(None, None, Some(settings))
}

#[test]
fn quickreplysettingsdefaultfn_creates_both_sections_disabled_with_stable_ids() {
    let settings = fc::quick_reply_settings_default_fn();
    assert_match_object(
        &settings["followUp"],
        &json!({ "enabled": false, "duration": 1, "unit": "days", "target": null }),
    );
    assert_match_object(
        &settings["retry"],
        &json!({ "enabled": false, "message": "", "maxRetries": 3, "target": null }),
    );
    assert_ne!(settings["followUp"]["id"], settings["retry"]["id"]);
}

#[test]
fn quickreplysettingsdefaultfn_new_send_message_nodes_carry_default_settings() {
    let node = fc::send_message_node_default_fn(
        &json!({ "nodeProps": { "id": "1", "position": { "x": 0, "y": 0 } } }),
    );
    assert_eq!(
        node["data"]["details"]["quickReplySettings"]["retry"]["maxRetries"],
        3
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_accepts_a_node_saved_before_the_feature_no_quickreplysettings()
 {
    let node = make_node(None, None, None);
    let details = without(&node["data"]["details"], &["quickReplySettings"]);
    assert!(
        fc::send_message_node_schema()
            .safe_parse(&spread(
                &node,
                json!({ "data": spread(&node["data"], json!({ "details": details })) })
            ))
            .success
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_requires_a_target_for_an_enabled_follow_up() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"]["enabled"] = json!(true);
    assert!(
        issue_messages(&with_settings(settings)).contains(&"quickReplyNextStepRequired".to_owned())
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_requires_a_target_for_an_enabled_retry() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"]["enabled"] = json!(true);
    settings["retry"]["message"] = json!("Tap one");
    assert!(
        issue_messages(&with_settings(settings)).contains(&"quickReplyNextStepRequired".to_owned())
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_requires_a_retry_message_when_retries_1() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "target": node_target() }),
    );
    assert!(
        issue_messages(&with_settings(settings))
            .contains(&"quickReplyRetryMessageRequired".to_owned())
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_allows_an_empty_retry_message_when_retries_0() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "maxRetries": 0, "target": node_target() }),
    );
    assert!(issue_messages(&with_settings(settings)).is_empty());
}

#[test]
fn sendmessagenodeschema_quickreplysettings_rejects_retries_above_5() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "maxRetries": 6, "message": "Tap", "target": node_target() }),
    );
    assert!(
        issue_paths(&with_settings(settings))
            .contains(&"data.details.quickReplySettings.retry.maxRetries".to_owned())
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_rejects_a_follow_up_longer_than_366_days() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"] = spread(
        &settings["followUp"],
        json!({ "enabled": true, "duration": 367, "unit": "days", "target": node_target() }),
    );
    assert!(
        issue_paths(&with_settings(settings))
            .contains(&"data.details.quickReplySettings.followUp.duration".to_owned())
    );
}

#[test]
fn sendmessagenodeschema_quickreplysettings_rejects_retry_together_with_a_getuserdata_step() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "message": "Tap", "target": node_target() }),
    );
    let node = make_node(
        None,
        Some(json!([fc::get_user_data_step_default_fn(&json!({}))])),
        Some(settings),
    );
    assert!(issue_messages(&node).contains(&"quickReplyRetryWithGetUserData".to_owned()));
}

#[test]
fn sendmessagenodeschema_quickreplysettings_ignores_settings_when_the_node_has_no_quick_replies() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"]["enabled"] = json!(true);
    settings["retry"]["enabled"] = json!(true);
    assert!(issue_messages(&make_node(Some(json!([])), None, Some(settings))).is_empty());
}

#[test]
fn resolveactivequickreplysettings_returns_nothing_for_legacy_details() {
    assert_eq!(fc::resolve_active_quick_reply_settings(None), json!({}));
    assert_eq!(
        fc::resolve_active_quick_reply_settings(Some(
            &json!({ "steps": [], "quickReplies": [quick_reply()] })
        )),
        json!({})
    );
}

#[test]
fn resolveactivequickreplysettings_returns_only_enabled_sections_that_have_a_target() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"] = spread(
        &settings["followUp"],
        json!({ "enabled": true, "target": node_target() }),
    );
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "message": "Tap" }),
    );
    let active = fc::resolve_active_quick_reply_settings(Some(&json!({
        "steps": [],
        "quickReplies": [quick_reply()],
        "quickReplySettings": settings,
    })));
    assert_eq!(active["followUp"]["id"], settings["followUp"]["id"]);
    assert!(active.get("retry").is_none());
}

#[test]
fn resolveactivequickreplysettings_drops_retry_when_the_node_contains_getuserdata() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "message": "Tap", "target": node_target() }),
    );
    let active = fc::resolve_active_quick_reply_settings(Some(&json!({
        "steps": [fc::get_user_data_step_default_fn(&json!({}))],
        "quickReplies": [quick_reply()],
        "quickReplySettings": settings,
    })));
    assert!(active.get("retry").is_none());
}

#[test]
fn resolveactivequickreplysettings_coerces_a_string_maxretries_to_a_number() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"] = spread(
        &settings["retry"],
        json!({ "enabled": true, "message": "Tap", "maxRetries": "2", "target": node_target() }),
    );
    let active = fc::resolve_active_quick_reply_settings(Some(&json!({
        "steps": [],
        "quickReplies": [quick_reply()],
        "quickReplySettings": settings,
    })));
    assert_eq!(active["retry"]["maxRetries"], 2);
}

#[test]
fn listquickreplysettingshandles_lists_a_handle_for_an_enabled_section_with_no_target_yet() {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"]["enabled"] = json!(true);
    assert_eq!(
        fc::list_quick_reply_settings_handles(&json!({
            "steps": [],
            "quickReplies": [quick_reply()],
            "quickReplySettings": settings,
        })),
        vec![json!({ "kind": "followUp", "id": settings["followUp"]["id"] })]
    );
}

#[test]
fn listquickreplysettingshandles_omits_a_handle_for_an_external_target_and_when_there_are_no_quick_replies()
 {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"] = spread(
        &settings["followUp"],
        json!({
            "enabled": true,
            "target": { "buttonType": "startExternalFlow", "beforeStep": fc::start_external_flow_step_default_fn(&json!({})) },
        }),
    );
    settings["retry"]["enabled"] = json!(true);
    assert_eq!(
        fc::list_quick_reply_settings_handles(&json!({
            "steps": [],
            "quickReplies": [quick_reply()],
            "quickReplySettings": settings,
        })),
        vec![json!({ "kind": "retry", "id": settings["retry"]["id"] })]
    );
    assert!(
        fc::list_quick_reply_settings_handles(&json!({
            "steps": [],
            "quickReplies": [],
            "quickReplySettings": settings,
        }))
        .is_empty()
    );
}

#[test]
fn listquickreplysettingshandles_partial_settings_a_settings_object_missing_a_section_does_not_throw()
 {
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["retry"]["enabled"] = json!(true);
    assert_eq!(
        fc::list_quick_reply_settings_handles(&json!({
            "steps": [],
            "quickReplies": [quick_reply()],
            "quickReplySettings": { "retry": settings["retry"] },
        })),
        vec![json!({ "kind": "retry", "id": settings["retry"]["id"] })]
    );
    assert!(
        fc::list_quick_reply_settings_handles(&json!({
            "steps": [],
            "quickReplies": [quick_reply()],
            "quickReplySettings": {},
        }))
        .is_empty()
    );
}

#[test]
fn quickreplynextstepfollowsedge_true_for_node_jumps_and_empty_targets_false_for_external() {
    assert!(fc::quick_reply_next_step_follows_edge(&Value::Null));
    assert!(fc::quick_reply_next_step_follows_edge(&node_target()));
    assert!(!fc::quick_reply_next_step_follows_edge(&json!({
        "buttonType": "startExternalFlow",
        "beforeStep": fc::start_external_flow_step_default_fn(&json!({})),
    })));
}

#[test]
fn computequickreplyfollowuptriggerat_adds_the_configured_delay_to_now() {
    let now: chrono::DateTime<chrono::Utc> = "2026-09-30T00:00:00.000Z".parse().unwrap();
    let clock = TestClock::new(now.timestamp_millis());
    let trigger_at = fc::compute_quick_reply_follow_up_trigger_at(
        &json!({ "duration": 2, "unit": "hours" }),
        &clock,
    );
    assert_eq!(
        trigger_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "2026-09-30T02:00:00.000Z"
    );
}
