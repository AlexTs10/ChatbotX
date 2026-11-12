//! Port of `packages/flow-config/__tests__/tiktok-text-rules.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::TiktokCardTitleProps;

fn button(label: &str) -> Value {
    fc::button_step_default_fn(&json!({ "label": label }))
}

fn long_text() -> String {
    rep("x", fc::TIKTOK_CARD_TITLE_MAX + 1)
}

fn truncated(
    channel: &str,
    buttons: &[Value],
    quick_reply_count: Option<usize>,
    text: &str,
) -> bool {
    fc::is_tiktok_card_title_truncated(TiktokCardTitleProps {
        channel: Some(channel),
        buttons: Some(buttons),
        quick_reply_count,
        text: Some(text),
    })
}

#[test]
fn istiktokcardtitletruncated_true_once_buttons_are_attached_and_the_text_exceeds_the_limit() {
    assert!(truncated("tiktok", &[button("Yes")], None, &long_text()));
}

#[test]
fn istiktokcardtitletruncated_false_when_the_text_fits_within_the_limit() {
    assert!(!truncated(
        "tiktok",
        &[button("Yes")],
        None,
        &rep("x", fc::TIKTOK_CARD_TITLE_MAX)
    ));
}

#[test]
fn istiktokcardtitletruncated_false_when_no_buttons_are_attached() {
    assert!(!truncated("tiktok", &[], None, &long_text()));
}

#[test]
fn istiktokcardtitletruncated_true_when_the_node_s_quick_replies_are_the_only_buttons() {
    // convertFlowStepText builds a TEMPLATE as soon as either the step's
    // buttons or the node's quick replies are present.
    assert!(truncated("tiktok", &[], Some(1), &long_text()));
}

#[test]
fn istiktokcardtitletruncated_true_on_omnichannel_too_since_it_may_still_reach_a_tiktok_contact() {
    assert!(truncated(
        "omnichannel",
        &[button("Yes")],
        None,
        &long_text()
    ));
}

#[test]
fn istiktokcardtitletruncated_false_on_a_channel_that_can_never_reach_tiktok() {
    assert!(!truncated(
        "messenger",
        &[button("Yes")],
        None,
        &long_text()
    ));
}

fn validator_step() -> Value {
    json!({ "id": "1", "stepType": "sendText", "text": long_text(), "buttons": [button("Yes")] })
}

fn tiktok_validator() -> Schema {
    fc::send_text_validator()
        .channel("tiktok")
        .expect("tiktok rule")
}

#[test]
fn sendtextvalidator_rejects_a_tiktok_sendtext_step_whose_text_is_too_long_once_buttoned() {
    let result = tiktok_validator().safe_parse(&validator_step());

    assert!(!result.success);
    assert_eq!(result.issues()[0].message, "tiktokCardTitleTooLong");
}

#[test]
fn sendtextvalidator_accepts_the_same_step_on_omnichannel_which_may_never_reach_tiktok() {
    let result = fc::send_text_validator()
        .base()
        .safe_parse(&validator_step());

    assert!(result.success);
}

#[test]
fn sendtextvalidator_accepts_a_tiktok_sendtext_step_whose_text_fits_once_buttoned() {
    let result =
        tiktok_validator().safe_parse(&spread(&validator_step(), json!({ "text": "Choose one" })));

    assert!(result.success);
}

#[test]
fn sendtextvalidator_stays_a_plain_sendtextstepschema_shape_for_tiktok_when_no_buttons_are_attached()
 {
    let result =
        tiktok_validator().safe_parse(&spread(&validator_step(), json!({ "buttons": [] })));

    assert!(result.success);
}

fn tiktok_node(
    text: &str,
    buttons: Vec<Value>,
    quick_replies: Vec<Value>,
    channel: Option<&str>,
) -> Value {
    fc::send_message_node_default_fn(&json!({
        "nodeProps": {},
        "detailProps": {
            "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": channel.unwrap_or("tiktok") })),
            "steps": [fc::send_text_step_default_fn(&json!({ "text": text, "buttons": buttons }))],
            "quickReplies": quick_replies,
        },
    }))
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

#[test]
fn refinestepsbychannel_node_level_quick_replies_blocks_publish_when_quick_replies_alone_overflow_the_card_title()
 {
    let issues = collect_issues(&tiktok_node(
        &long_text(),
        vec![],
        vec![button("Yes")],
        None,
    ));

    let messages: Vec<&str> = issues.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(messages, vec!["tiktokCardTitleTooLong"]);
    assert_eq!(
        issues[0].1,
        json!([0, "data", "details", "steps", 0, "text"])
    );
}

#[test]
fn refinestepsbychannel_node_level_quick_replies_raises_the_code_once_not_twice_when_the_step_also_has_buttons()
 {
    // refineTiktokSendTextStep already catches this through the validator map.
    let issues = collect_issues(&tiktok_node(
        &long_text(),
        vec![button("Yes")],
        vec![button("No")],
        None,
    ));

    let messages: Vec<&str> = issues.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(messages, vec!["tiktokCardTitleTooLong"]);
}

#[test]
fn refinestepsbychannel_node_level_quick_replies_stays_quiet_when_the_text_fits_the_card_title() {
    assert!(
        collect_issues(&tiktok_node(
            "Choose one",
            vec![],
            vec![button("Yes")],
            None
        ))
        .is_empty()
    );
}

#[test]
fn refinestepsbychannel_node_level_quick_replies_stays_quiet_with_no_quick_replies_and_no_buttons()
{
    assert!(collect_issues(&tiktok_node(&long_text(), vec![], vec![], None)).is_empty());
}

#[test]
fn refinestepsbychannel_node_level_quick_replies_stays_quiet_on_omnichannel_which_may_never_reach_a_tiktok_contact()
 {
    assert!(
        collect_issues(&tiktok_node(
            &long_text(),
            vec![],
            vec![button("Yes")],
            Some("omnichannel")
        ))
        .is_empty()
    );
}

#[test]
fn istiktokquickreplycardtitletoolong_false_once_the_step_carries_buttons_of_its_own() {
    assert!(!fc::is_tiktok_quick_reply_card_title_too_long(
        &json!({ "text": long_text(), "buttons": [button("Yes")] }),
        1
    ));
}

#[test]
fn istiktokquickreplycardtitletoolong_true_for_quick_replies_alone() {
    assert!(fc::is_tiktok_quick_reply_card_title_too_long(
        &json!({ "text": long_text(), "buttons": [] }),
        1
    ));
}

#[test]
fn istiktokquickreplycardtitletoolong_false_when_there_are_no_quick_replies() {
    assert!(!fc::is_tiktok_quick_reply_card_title_too_long(
        &json!({ "text": long_text(), "buttons": [] }),
        0
    ));
}

#[test]
fn sendtextstepschema_sendtextvalidator_omnichannel_carries_no_tiktok_rule() {
    let buttoned_step = json!({ "buttons": [button("Yes")], "id": "1", "stepType": "sendText", "text": long_text() });
    // What matters here is that TikTok's card title rule stays off the base.
    assert!(
        fc::send_text_validator()
            .base()
            .safe_parse(&buttoned_step)
            .success
    );
    assert!(
        fc::send_text_step_schema()
            .safe_parse(&buttoned_step)
            .success
    );
}
