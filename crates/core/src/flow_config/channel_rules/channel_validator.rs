//! Port of `packages/flow-config/src/channel-rules/channel-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

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
}

impl StepValidator {
    /// `validator.omnichannel` (or the schema itself).
    pub fn base(&self) -> Schema {
        match self {
            StepValidator::Schema(s) | StepValidator::PerChannel { base: s, .. } => s.clone(),
        }
    }

    /// `validator[channel]` (no fallback).
    pub fn channel(&self, channel: &str) -> Option<Schema> {
        match self {
            StepValidator::Schema(_) => None,
            StepValidator::PerChannel { base, overrides } => {
                if channel == "omnichannel" {
                    return Some(base.clone());
                }
                overrides
                    .iter()
                    .find(|(c, _)| c == channel)
                    .map(|(_, s)| s.clone())
            }
        }
    }
}

/// `resolveStepValidator(validator, channel)`: the channel's own rule, else
/// the omnichannel base.
pub fn resolve_step_validator(validator: &StepValidator, channel: &str) -> Schema {
    validator
        .channel(channel)
        .unwrap_or_else(|| validator.base())
}
