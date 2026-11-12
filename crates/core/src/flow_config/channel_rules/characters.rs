//! Port of `packages/flow-config/src/channel-rules/characters.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT countMessageCharacters:
// value=>Array.from(value??"").length

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn count_message_characters(value: Option<&str>) -> usize {
    todo!("P3.4")
}
