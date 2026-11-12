//! `packages/flow-config/src/channel-rules/policies`.

mod api;
mod define;
mod instagram;
mod messenger;
mod omnichannel;
mod smtp;
mod telegram;
mod threads;
mod tiktok;
mod webchat;
mod whatsapp;
mod zalo;

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

fn policy_from(value: &'static Value) -> ChannelFlowPolicy {
    let limits = &value["limits"];
    let as_usize = |v: &Value| v.as_u64().map(|n| n as usize);
    ChannelFlowPolicy {
        limits: ChannelLimits {
            button_count: as_usize(&limits["buttonCount"]).unwrap_or(0),
            button_label: as_usize(&limits["buttonLabel"]).unwrap_or(0),
            card_title: as_usize(&limits["cardTitle"]),
            text: as_usize(&limits["text"]).unwrap_or(0),
            view_catalog: limits
                .get("viewCatalog")
                .map(|vc| SendViewCatalogLengthLimits {
                    text: as_usize(&vc["text"]).unwrap_or(0),
                    footer: as_usize(&vc["footer"]).unwrap_or(0),
                }),
        },
        native_button_types: value["nativeButtonTypes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect(),
        quick_replies_share_button_slots: value["quickRepliesShareButtonSlots"] == true,
        steps: value["steps"]
            .as_object()
            .into_iter()
            .flatten()
            // The Execute JavaScript step is dropped (DECISIONS 2026-11-11).
            .filter(|(k, _)| k.as_str() != "executeJavascript")
            .filter_map(|(k, v)| Some((k.as_str(), v.as_str()?)))
            .collect(),
        product_messages: value.get("productMessages").cloned(),
    }
}

/// `CHANNEL_FLOW_POLICIES`.
pub fn channel_flow_policies()
-> &'static std::collections::BTreeMap<&'static str, ChannelFlowPolicy> {
    static V: LazyLock<std::collections::BTreeMap<&'static str, ChannelFlowPolicy>> =
        LazyLock::new(|| {
            [
                ("api", api_flow_policy()),
                ("instagram", instagram_flow_policy()),
                ("messenger", messenger_flow_policy()),
                ("omnichannel", omnichannel_flow_policy()),
                ("smtp", smtp_flow_policy()),
                ("telegram", telegram_flow_policy()),
                ("threads", threads_flow_policy()),
                ("tiktok", tiktok_flow_policy()),
                ("webchat", webchat_flow_policy()),
                ("whatsapp", whatsapp_flow_policy()),
                ("zalo", zalo_flow_policy()),
            ]
            .into_iter()
            .map(|(k, v)| (k, policy_from(v)))
            .collect()
        });
    &V
}

/// `getChannelFlowPolicy(channel)`: `None` for an unknown channel.
pub fn get_channel_flow_policy(channel: Option<&str>) -> Option<&'static ChannelFlowPolicy> {
    channel_flow_policies().get(channel?)
}

/// `isNativeButtonType(channel, buttonType)`.
pub fn is_native_button_type(channel: Option<&str>, button_type: &str) -> bool {
    get_channel_flow_policy(channel).is_some_and(|p| p.native_button_types.contains(&button_type))
}
