//! Port of `packages/flow-config/src/channel-rules/send-text-length-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const SEND_TEXT_MAX: usize = 6000;

/// `resolveSendTextLengthLimits` input.
#[derive(Debug, Clone, Copy, Default)]
pub struct SendTextLengthProps<'a> {
    pub channel: Option<&'a str>,
    pub has_buttons: Option<bool>,
    pub has_quick_replies: Option<bool>,
}

/// `SendTextLengthLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendTextLengthLimits {
    pub button_label: usize,
    pub quick_reply_label: usize,
    pub text: usize,
}

fn resolve_channel_key(channel: Option<&str>) -> &str {
    match channel {
        Some(c) if channel_flow_policies().contains_key(c) => c,
        _ => "omnichannel",
    }
}

/// `resolveSendTextLengthLimits({ channel, hasButtons, hasQuickReplies })`.
pub fn resolve_send_text_length_limits(props: SendTextLengthProps<'_>) -> SendTextLengthLimits {
    let policy = &channel_flow_policies()[resolve_channel_key(props.channel)];
    let is_card_title = policy.limits.card_title.is_some()
        && (props.has_buttons == Some(true) || props.has_quick_replies == Some(true));
    SendTextLengthLimits {
        button_label: policy.limits.button_label,
        quick_reply_label: policy.limits.button_label,
        text: if is_card_title {
            policy.limits.card_title.unwrap_or(policy.limits.text)
        } else {
            policy.limits.text
        },
    }
}

/// `refineSendTextLengthForChannel(channel)`.
pub fn refine_send_text_length_for_channel(
    channel: &'static str,
) -> impl Fn(&Value, &mut RefineCtx) + Send + Sync + 'static {
    move |step, ctx| {
        let length = count_message_characters(step["text"].as_str());
        if length > SEND_TEXT_MAX || length <= channel_flow_policies()[channel].limits.text {
            return;
        }
        ctx.issue(path!["text"], "sendTextTooLongForChannel");
    }
}
