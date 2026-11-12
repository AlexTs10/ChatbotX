//! Port of `packages/flow-config/src/channel-rules/media-step-validators.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

fn build_media_step_validator(schema: Schema, step_type: &'static str) -> StepValidator {
    let overrides = channels_with_media_step_limits(step_type)
        .into_iter()
        .map(|channel| {
            let refined = schema.clone().super_refine(move |step, ctx| {
                let buttons = arr_field(step, "buttons");
                if is_media_buttons_dropped(channel, step_type, buttons) {
                    ctx.issue(path!["buttons"], "mediaButtonsUnsupported");
                }
            });
            (channel.to_owned(), refined)
        })
        .collect();
    StepValidator::PerChannel {
        base: schema,
        overrides,
    }
}

/// `sendImageValidator`.
pub fn send_image_validator() -> StepValidator {
    static V: LazyLock<StepValidator> =
        LazyLock::new(|| build_media_step_validator(send_image_step_schema(), "sendImage"));
    V.clone()
}

/// `sendVideoValidator`.
pub fn send_video_validator() -> StepValidator {
    static V: LazyLock<StepValidator> =
        LazyLock::new(|| build_media_step_validator(send_video_step_schema(), "sendVideo"));
    V.clone()
}

/// `sendAudioValidator`.
pub fn send_audio_validator() -> StepValidator {
    static V: LazyLock<StepValidator> =
        LazyLock::new(|| build_media_step_validator(send_audio_step_schema(), "sendAudio"));
    V.clone()
}

/// `sendFileValidator`.
pub fn send_file_validator() -> StepValidator {
    static V: LazyLock<StepValidator> =
        LazyLock::new(|| build_media_step_validator(send_file_step_schema(), "sendFile"));
    V.clone()
}

/// `sendGifValidator`.
pub fn send_gif_validator() -> StepValidator {
    static V: LazyLock<StepValidator> =
        LazyLock::new(|| build_media_step_validator(send_gif_step_schema(), "sendGif"));
    V.clone()
}
