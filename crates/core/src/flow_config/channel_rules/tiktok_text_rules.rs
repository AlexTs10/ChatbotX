//! Port of `packages/flow-config/src/channel-rules/tiktok-text-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `isTiktokCardTitleTruncated` input.
#[derive(Debug, Clone, Copy, Default)]
pub struct TiktokCardTitleProps<'a> {
    pub channel: Option<&'a str>,
    pub buttons: Option<&'a [Value]>,
    pub quick_reply_count: Option<usize>,
    pub text: Option<&'a str>,
}

fn exceeds_card_title_max(buttons: usize, quick_reply_count: usize, text: Option<&str>) -> bool {
    buttons + quick_reply_count > 0 && count_message_characters(text) > TIKTOK_CARD_TITLE_MAX
}

/// `isTiktokCardTitleTruncated(props)`.
pub fn is_tiktok_card_title_truncated(props: TiktokCardTitleProps<'_>) -> bool {
    matches!(props.channel, Some("tiktok" | "omnichannel"))
        && exceeds_card_title_max(
            props.buttons.map_or(0, <[Value]>::len),
            props.quick_reply_count.unwrap_or(0),
            props.text,
        )
}

/// `refineTiktokSendTextStep(step, ctx)`.
pub fn refine_tiktok_send_text_step(step: &Value, ctx: &mut RefineCtx) {
    if exceeds_card_title_max(arr_field(step, "buttons").len(), 0, step["text"].as_str()) {
        ctx.issue(path!["text"], "tiktokCardTitleTooLong");
    }
}

/// `isTiktokQuickReplyCardTitleTooLong({ step, quickReplyCount })`.
pub fn is_tiktok_quick_reply_card_title_too_long(step: &Value, quick_reply_count: usize) -> bool {
    let buttons = arr_field(step, "buttons").len();
    buttons == 0 && exceeds_card_title_max(buttons, quick_reply_count, step["text"].as_str())
}
