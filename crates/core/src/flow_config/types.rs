//! Port of `packages/flow-config/src/types.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn upload_modes = z::enum_(&["url", "file"]));

schema!(pub fn card_layouts = z::enum_(&["horizontal", "vertical"]));
