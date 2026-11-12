//! Port of `packages/flow-config/src/channel-rules/validators.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT channelAwareStepValidators (object): {"sendAudio":{"omnichannel":"<schema sendAudioStepSchema>","messenger":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"},"sendCarousel":{"omnichannel":"<schema sendCarouselStepSchema>","whatsapp":"<schema ?>"},"sendFile":{"omnichannel":"<schema sendFileStepSchema>","messenger":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"},"sendGif":{"omnichannel":"<schema sendGifStepSchema>","whatsapp":"<schema ?>","smtp":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>"},"sendImage":{"omnichannel":"<schema sendImageStepSchema>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"},"sendProducts":{"whatsapp":"<schema ?>","omnichannel":"<schema ?>"},"sendText":{"omnichannel":"<schema ?>","api":"<schema ?>","instagram":"<schema ?>","messenger":"<schema ?>","smtp":"<schema ?>","telegram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","webchat":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>"},"sendVideo":{"omnichannel":"<schema sendVideoStepSchema>","whatsapp":"<schema ?>","zalo":"<schema ?>","smtp":"<schema ?>","instagram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","api":"<schema ?>"},"sendWaTemplateMessage":{"omnichannel":"<schema ?>"}}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn channel_aware_step_validators()
-> &'static std::collections::BTreeMap<&'static str, StepValidator> {
    todo!("P3.4")
}
