//! Port of `packages/flow-config/src/nodes/quick-reply-carrier.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isQuickReplyCarrierStep:
// function isQuickReplyCarrierStep(channel,step){const carrierSteps=QUICK_REPLY_CARRIER_STEPS_BY_CHANNEL[channel??""]??DEFAULT_QUICK_REPLY_CARRIER_STEPS;return carrierSteps.has(step.stepType)}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn is_quick_reply_carrier_step(channel: &str, step: &Value) -> bool {
    todo!("P3.4")
}
