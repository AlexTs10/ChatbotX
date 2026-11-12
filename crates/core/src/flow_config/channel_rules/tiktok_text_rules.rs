//! Port of `packages/flow-config/src/channel-rules/tiktok-text-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isTiktokCardTitleTruncated:
// props=>TIKTOK_REACHABLE_CHANNELS.has(props.channel??"")&&exceedsCardTitleMax(props)

// HAND-PORT refineTiktokSendTextStep:
// (step,ctx)=>{if(exceedsCardTitleMax(step)){ctx.addIssue({code:"custom",message:flowValidationCodes.tiktokCardTitleTooLong,path:["text"]})}}

// HAND-PORT isTiktokQuickReplyCardTitleTooLong:
// props=>props.step.buttons.length===0&&exceedsCardTitleMax({buttons:props.step.buttons,quickReplyCount:props.quickReplyCount,text:props.step.text})

// ---- stage 1 API (hand-ported in stage 2) ----

/// `isTiktokCardTitleTruncated` input.
#[derive(Debug, Clone, Copy, Default)]
pub struct TiktokCardTitleProps<'a> {
    pub channel: Option<&'a str>,
    pub buttons: Option<&'a [Value]>,
    pub quick_reply_count: Option<usize>,
    pub text: Option<&'a str>,
}

pub fn is_tiktok_card_title_truncated(props: TiktokCardTitleProps<'_>) -> bool {
    todo!("P3.4")
}

pub fn is_tiktok_quick_reply_card_title_too_long(step: &Value, quick_reply_count: usize) -> bool {
    todo!("P3.4")
}
