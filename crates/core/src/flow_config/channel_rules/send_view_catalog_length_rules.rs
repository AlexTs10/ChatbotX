//! Port of `packages/flow-config/src/channel-rules/send-view-catalog-length-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const SEND_VIEW_CATALOG_TEXT_MAX: usize = 1024;

pub const SEND_VIEW_CATALOG_FOOTER_MAX: usize = 60;

/// `SendViewCatalogLengthLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendViewCatalogLengthLimits {
    pub text: usize,
    pub footer: usize,
}

/// `resolveSendViewCatalogLengthLimits(channel)`: the channel's declared
/// limits, else the widest supported ones.
pub fn resolve_send_view_catalog_length_limits(
    channel: Option<&str>,
) -> SendViewCatalogLengthLimits {
    channel
        .and_then(|c| channel_flow_policies().get(c))
        .and_then(|p| p.limits.view_catalog)
        .unwrap_or(SendViewCatalogLengthLimits {
            text: SEND_VIEW_CATALOG_TEXT_MAX,
            footer: SEND_VIEW_CATALOG_FOOTER_MAX,
        })
}
