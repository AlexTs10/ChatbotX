//! Port of `packages/flow-config/src/channel-rules/characters.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `countMessageCharacters`: code points (`Array.from(value).length`).
pub fn count_message_characters(value: Option<&str>) -> usize {
    value.map_or(0, |v| v.chars().count())
}
