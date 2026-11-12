//! Port of `packages/flow-config/src/channel-rules/send-view-catalog-length-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const SEND_VIEW_CATALOG_TEXT_MAX: usize = 1024;

pub const SEND_VIEW_CATALOG_FOOTER_MAX: usize = 60;

// HAND-PORT resolveSendViewCatalogLengthLimits:
// channel=>{const parsed=channelTypes.safeParse(channel);const declared=parsed.success?CHANNEL_FLOW_POLICIES[parsed.data].limits.viewCatalog:void 0;return declared??{footer:SEND_VIEW_CATALOG_FOOTER_MAX,text:SEND_VIEW_CATALOG_TEXT_MAX}}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `SendViewCatalogLengthLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendViewCatalogLengthLimits {
    pub text: usize,
    pub footer: usize,
}

pub fn resolve_send_view_catalog_length_limits(
    channel: Option<&str>,
) -> SendViewCatalogLengthLimits {
    todo!("P3.4")
}
