//! Port of `packages/flow-config/src/step-input-resolution.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn step_input_resolution_reasons = z::enum_(&["timeout", "user_skipped"]));

pub const STEP_INPUT_RESOLUTION_PAYLOAD_TYPE: &str = "stepInputResolution";

const STEP_INPUT_SKIP_POSTBACK_PREFIX: &str = "STEP_INPUT_SKIP|";

fn is_valid_challenge_id(value: &str) -> bool {
    (1..=19).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|n| n <= i64::MAX as u64)
}

/// `encodeStepInputSkipPostback(challengeId)`.
pub fn encode_step_input_skip_postback(challenge_id: &str) -> String {
    format!("{STEP_INPUT_SKIP_POSTBACK_PREFIX}{challenge_id}")
}

/// `decodeStepInputSkipPostback(payload)`.
pub fn decode_step_input_skip_postback(payload: Option<&str>) -> Option<String> {
    let id = payload?.strip_prefix(STEP_INPUT_SKIP_POSTBACK_PREFIX)?;
    is_valid_challenge_id(id).then(|| id.to_owned())
}
