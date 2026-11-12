//! Port of `packages/flow-config/src/channel-rules/validators.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `channelAwareStepValidators`.
pub fn channel_aware_step_validators()
-> &'static std::collections::BTreeMap<&'static str, StepValidator> {
    static V: LazyLock<std::collections::BTreeMap<&'static str, StepValidator>> =
        LazyLock::new(|| {
            [
                ("sendAudio", send_audio_validator()),
                ("sendCarousel", send_carousel_validator()),
                ("sendFile", send_file_validator()),
                ("sendGif", send_gif_validator()),
                ("sendImage", send_image_validator()),
                ("sendProducts", send_products_validator()),
                ("sendText", send_text_validator()),
                ("sendVideo", send_video_validator()),
                (
                    "sendWaTemplateMessage",
                    send_wa_template_message_validator(),
                ),
            ]
            .into_iter()
            .collect()
        });
    &V
}
