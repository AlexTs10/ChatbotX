//! Port of `packages/flow-config/src/step-input-resolution.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn step_input_resolution_reasons = z::enum_(&["timeout", "user_skipped"]));

pub const STEP_INPUT_RESOLUTION_PAYLOAD_TYPE: &str = "stepInputResolution";

// HAND-PORT encodeStepInputSkipPostback:
// function encodeStepInputSkipPostback(challengeId){return`${STEP_INPUT_SKIP_POSTBACK_PREFIX}${challengeId}`}

// HAND-PORT decodeStepInputSkipPostback:
// function decodeStepInputSkipPostback(payload){if(!payload?.startsWith(STEP_INPUT_SKIP_POSTBACK_PREFIX)){return null}const challengeId=payload.slice(STEP_INPUT_SKIP_POSTBACK_PREFIX.length);return isValidChallengeId(challengeId)?challengeId:null}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn decode_step_input_skip_postback(payload: Option<&str>) -> Option<String> {
    todo!("P3.4")
}

pub fn encode_step_input_skip_postback(challenge_id: &str) -> String {
    todo!("P3.4")
}
