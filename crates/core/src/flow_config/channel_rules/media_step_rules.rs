//! Port of `packages/flow-config/src/channel-rules/media-step-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

const MEDIA_STEP_TYPES: &[&str] = &["sendImage", "sendVideo", "sendAudio", "sendFile", "sendGif"];

/// `isMediaStepType`.
pub fn is_media_step_type(step_type: &str) -> bool {
    MEDIA_STEP_TYPES.contains(&step_type)
}

/// `resolveMediaStepSupport({ channel, stepType })`.
pub fn resolve_media_step_support(channel: &str, step_type: &str) -> &'static str {
    if !is_media_step_type(step_type) {
        return "full";
    }
    channel_flow_policies()
        .get(channel)
        .and_then(|p| p.steps.get(step_type).copied())
        .unwrap_or("full")
}

/// `isMediaStepUnsupported`.
pub fn is_media_step_unsupported(channel: &str, step_type: &str) -> bool {
    resolve_media_step_support(channel, step_type) == "unsupported"
}

/// `isMediaButtonsDropped`: buttons attached on a no-buttons channel.
pub fn is_media_buttons_dropped(channel: &str, step_type: &str, buttons: &[Value]) -> bool {
    !buttons.is_empty() && resolve_media_step_support(channel, step_type) == "noButtons"
}

/// `channelsWithMediaStepLimits(stepType)`.
pub fn channels_with_media_step_limits(step_type: &str) -> Vec<&'static str> {
    channel_flow_policies()
        .iter()
        .filter(|(_, p)| p.steps.get(step_type).copied() != Some("full"))
        .map(|(c, _)| *c)
        .collect()
}
