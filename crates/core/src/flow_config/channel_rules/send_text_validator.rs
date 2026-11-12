//! Port of `packages/flow-config/src/channel-rules/send-text-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `sendTextValidator`: the per-channel length rules (and TikTok's card title).
pub fn send_text_validator() -> StepValidator {
    static V: LazyLock<StepValidator> = LazyLock::new(|| {
        let with = |channel: &'static str| {
            send_text_step_schema().super_refine(refine_send_text_length_for_channel(channel))
        };
        let overrides = [
            "api",
            "instagram",
            "messenger",
            "smtp",
            "telegram",
            "threads",
            "webchat",
            "whatsapp",
            "zalo",
        ]
        .into_iter()
        .map(|c| (c.to_owned(), with(c)))
        .chain(std::iter::once((
            "tiktok".to_owned(),
            send_text_step_schema()
                .super_refine(refine_tiktok_send_text_step)
                .super_refine(refine_send_text_length_for_channel("tiktok")),
        )))
        .collect();
        StepValidator::PerChannel {
            base: with("omnichannel"),
            overrides,
        }
    });
    V.clone()
}
