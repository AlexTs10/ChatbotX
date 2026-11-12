//! Port of `packages/flow-config/src/nodes/quick-reply-carrier.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `isQuickReplyCarrierStep(channel, step)`: whether the step can carry the
/// node's quick replies on this channel.
pub fn is_quick_reply_carrier_step(channel: &str, step: &Value) -> bool {
    const MEDIA: &[&str] = &[
        "sendText",
        "sendImage",
        "sendVideo",
        "sendAudio",
        "sendFile",
        "sendGif",
    ];
    const META: &[&str] = &[
        "sendText",
        "sendImage",
        "sendVideo",
        "sendAudio",
        "sendFile",
        "sendGif",
        "sendMultipleImages",
    ];
    let carriers: &[&str] = match channel {
        "instagram" | "messenger" => META,
        "telegram" => MEDIA,
        "whatsapp" => &["sendText", "sendImage", "sendCarousel"],
        "zalo" => &["sendText", "sendImage"],
        _ => &["sendText"],
    };
    step["stepType"]
        .as_str()
        .is_some_and(|t| carriers.contains(&t))
}
