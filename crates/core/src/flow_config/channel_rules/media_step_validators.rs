//! Port of `packages/flow-config/src/channel-rules/media-step-validators.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT sendImageValidator (object): {"omnichannel":"<schema sendImageStepSchema>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"}

// HAND-PORT sendVideoValidator (object): {"omnichannel":"<schema sendVideoStepSchema>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"}

// HAND-PORT sendAudioValidator (object): {"omnichannel":"<schema sendAudioStepSchema>","messenger":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"}

// HAND-PORT sendFileValidator (object): {"omnichannel":"<schema sendFileStepSchema>","messenger":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"}

// HAND-PORT sendGifValidator (object): {"omnichannel":"<schema sendGifStepSchema>","whatsapp":"<schema ?>","smtp":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>"}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_audio_validator() -> StepValidator {
    todo!("P3.4")
}

pub fn send_file_validator() -> StepValidator {
    todo!("P3.4")
}

pub fn send_gif_validator() -> StepValidator {
    todo!("P3.4")
}

pub fn send_image_validator() -> StepValidator {
    todo!("P3.4")
}

pub fn send_video_validator() -> StepValidator {
    todo!("P3.4")
}
