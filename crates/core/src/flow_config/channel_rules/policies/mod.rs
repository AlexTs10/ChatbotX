//! `packages/flow-config/src/channel-rules/policies`.

pub mod api;
pub mod define;
pub mod instagram;
pub mod messenger;
pub mod omnichannel;
pub mod smtp;
pub mod telegram;
pub mod threads;
pub mod tiktok;
pub mod webchat;
pub mod whatsapp;
pub mod zalo;

pub use api::*;
pub use define::*;
pub use instagram::*;
pub use messenger::*;
pub use omnichannel::*;
pub use smtp::*;
pub use telegram::*;
pub use threads::*;
pub use tiktok::*;
pub use webchat::*;
pub use whatsapp::*;
pub use zalo::*;

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT index.ts: CHANNEL_FLOW_POLICIES, getChannelFlowPolicy, isNativeButtonType

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn channel_flow_policies()
-> &'static std::collections::BTreeMap<&'static str, ChannelFlowPolicy> {
    todo!("P3.4")
}

pub fn get_channel_flow_policy(channel: Option<&str>) -> Option<&'static ChannelFlowPolicy> {
    todo!("P3.4")
}

pub fn is_native_button_type(channel: Option<&str>, button_type: &str) -> bool {
    todo!("P3.4")
}
