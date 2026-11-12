//! Port of `packages/flow-config/src/channel-rules/send-text-length-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const SEND_TEXT_MAX: usize = 6000;

// HAND-PORT resolveSendTextLengthLimits:
// props=>{const channel=resolveChannelKey(props.channel);const policy=CHANNEL_FLOW_POLICIES[channel];const isCardTitle=policy.limits.cardTitle!==void 0&&(props.hasButtons===true||props.hasQuickReplies===true);return{buttonLabel:policy.limits.buttonLabel,quickReplyLabel:policy.limits.buttonLabel,text:isCardTitle?policy.limits.cardTitle??policy.limits.text:policy.limits.text}}

// HAND-PORT refineSendTextLengthForChannel:
// channel=>(step,ctx)=>{const length=countMessageCharacters(step.text);if(length>SEND_TEXT_MAX||length<=CHANNEL_FLOW_POLICIES[channel].limits.text){return}ctx.addIssue({code:"custom",message:flowValidationCodes.sendTextTooLongForChannel,path:["text"]})}

// ---- stage 1 API (hand-ported in stage 2) ----

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

pub fn resolve_send_text_length_limits(props: SendTextLengthProps<'_>) -> SendTextLengthLimits {
    todo!("P3.4")
}
