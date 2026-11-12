//! Port of `packages/flow-config/src/channel-rules/send-text-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT sendTextValidator (object): {"omnichannel":"<schema ?>","api":"<schema ?>","instagram":"<schema ?>","messenger":"<schema ?>","smtp":"<schema ?>","telegram":"<schema ?>","threads":"<schema ?>","tiktok":"<schema ?>","webchat":"<schema ?>","whatsapp":"<schema ?>","zalo":"<schema ?>"}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_text_validator() -> StepValidator {
    todo!("P3.4")
}
