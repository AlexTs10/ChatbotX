//! Port of `packages/flow-config/src/channel-rules/channel-step-refinement.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `refineStepsByChannel(nodes, ctx)`: per-channel step support, button
/// counts, TikTok card titles and the channel-aware step validators.
pub fn refine_steps_by_channel(nodes: &[Value], ctx: &mut RefineCtx) {
    for (node_index, node) in nodes.iter().enumerate() {
        if node["type"] != "sendMessage" {
            continue;
        }
        let channel = get_send_message_channel(node);
        let Some(policy) = channel_flow_policies().get(channel.as_str()) else {
            continue;
        };
        let details = &node["data"]["details"];
        let quick_reply_count = arr_field(details, "quickReplies").len();
        let mut quick_reply_overflow_reported = false;
        for (step_index, step) in arr_field(details, "steps").iter().enumerate() {
            let step_type = str_field(step, "stepType");
            let step_path = |extra: Vec<PathSegment>| {
                let mut path = path![node_index, "data", "details", "steps", step_index];
                path.extend(extra);
                path
            };
            let add_step_issue = |ctx: &mut RefineCtx,
                                  message: String,
                                  extra: Vec<PathSegment>,
                                  capability: Option<Value>| {
                let mut issue = RawIssue::custom(step_path(extra), message);
                if let Some(capability) = capability {
                    issue.params = Some(json!({ "capability": capability }));
                }
                ctx.add(issue);
            };
            if policy.steps.get(step_type).copied() == Some("unsupported") {
                add_step_issue(
                    ctx,
                    format!("The {channel} channel does not support {step_type}."),
                    vec![],
                    Some(json!({
                        "block": step_type,
                        "channel": channel,
                        "code": "unsupportedBlock",
                        "policyVersion": CHANNEL_POLICY_VERSION,
                    })),
                );
                continue;
            }
            let buttons = arr_field(step, "buttons").len();
            let counted_quick_replies =
                if step_type == "sendText" && policy.quick_replies_share_button_slots {
                    quick_reply_count
                } else {
                    0
                };
            let total = buttons + counted_quick_replies;
            let limit = policy.limits.button_count;
            let quick_replies_cause_overflow =
                counted_quick_replies > 0 && total > limit && buttons <= limit;
            if total > limit {
                let capability = json!({
                    "actual": total,
                    "allowed": limit,
                    "block": step_type,
                    "channel": channel,
                    "code": "constraintExceeded",
                    "constraintId": "maxButtonCount",
                    "policyVersion": CHANNEL_POLICY_VERSION,
                    "unit": "buttons",
                });
                let message =
                    format!("{step_type} exceeds the {channel} maximum of {limit} buttons.");
                if quick_replies_cause_overflow && !quick_reply_overflow_reported {
                    quick_reply_overflow_reported = true;
                    let mut issue = RawIssue::custom(
                        path![node_index, "data", "details", "quickReplies"],
                        message,
                    );
                    issue.params = Some(json!({ "capability": capability }));
                    ctx.add(issue);
                } else if !quick_replies_cause_overflow {
                    add_step_issue(ctx, message, path!["buttons"], Some(capability));
                }
            }
            if channel == "tiktok"
                && step_type == "sendText"
                && is_tiktok_quick_reply_card_title_too_long(step, quick_reply_count)
            {
                add_step_issue(
                    ctx,
                    "tiktokCardTitleTooLong".to_owned(),
                    path!["text"],
                    None,
                );
            }
            let Some(validator) = channel_aware_step_validators().get(step_type) else {
                continue;
            };
            let result = resolve_step_validator(validator, &channel).safe_parse(step);
            for issue in result.issues() {
                add_step_issue(ctx, issue.message.clone(), issue.path.clone(), None);
            }
        }
    }
}
