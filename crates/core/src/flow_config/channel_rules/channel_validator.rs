//! Port of `packages/flow-config/src/channel-rules/channel-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT resolveStepValidator:
// (validator,channel)=>{if(!isChannelValidatorMap(validator)){return validator}const override=validator[channel];return override??validator[channelTypes.enum.omnichannel]}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `StepValidator`: one schema for every channel, or a base (omnichannel)
/// schema plus per-channel overrides.
#[derive(Clone, Debug)]
pub enum StepValidator {
    Schema(Schema),
    PerChannel {
        base: Schema,
        overrides: Vec<(String, Schema)>,
    },
}

impl StepValidator {
    pub fn per_channel(base: Schema, overrides: Vec<(&str, Schema)>) -> StepValidator {
        StepValidator::PerChannel {
            base,
            overrides: overrides
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        }
    }

    /// `validator.omnichannel` (or the schema itself).
    pub fn base(&self) -> Schema {
        todo!("P3.4")
    }

    /// `validator[channel]` (no fallback).
    pub fn channel(&self, channel: &str) -> Option<Schema> {
        let _ = channel;
        todo!("P3.4")
    }
}

pub fn resolve_step_validator(validator: &StepValidator, channel: &str) -> Schema {
    todo!("P3.4")
}
