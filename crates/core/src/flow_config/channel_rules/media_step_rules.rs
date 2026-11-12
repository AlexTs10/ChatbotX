//! Port of `packages/flow-config/src/channel-rules/media-step-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isMediaStepType:
// stepType=>MEDIA_STEP_TYPES.includes(stepType)

// HAND-PORT resolveMediaStepSupport:
// props=>{if(!isMediaStepType(props.stepType)){return stepSupport.full}const channel=channelTypes.safeParse(props.channel);return channel.success?CHANNEL_FLOW_POLICIES[channel.data].steps[props.stepType]:stepSupport.full}

// HAND-PORT isMediaStepUnsupported:
// props=>resolveMediaStepSupport(props)===stepSupport.unsupported

// HAND-PORT isMediaButtonsDropped:
// props=>(props.buttons?.length??0)>0&&resolveMediaStepSupport(props)===stepSupport.noButtons

// HAND-PORT channelsWithMediaStepLimits:
// stepType=>channelTypes.options.filter(channel=>CHANNEL_FLOW_POLICIES[channel].steps[stepType]!==stepSupport.full)

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn is_media_buttons_dropped(channel: &str, step_type: &str, buttons: &[Value]) -> bool {
    todo!("P3.4")
}

pub fn is_media_step_unsupported(channel: &str, step_type: &str) -> bool {
    todo!("P3.4")
}

pub fn resolve_media_step_support(channel: &str, step_type: &str) -> &'static str {
    todo!("P3.4")
}
