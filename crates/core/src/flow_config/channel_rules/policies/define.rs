//! Port of `packages/flow-config/src/channel-rules/policies/define.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const CHANNEL_POLICY_VERSION: usize = 3;

pub fn step_support() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"full":"full","noButtons":"noButtons","unsupported":"unsupported"}"#,
        )
        .expect("valid JSON")
    });
    &V
}

pub const CHANNEL_DELIVERABLE_STEP_TYPES: &[&str] = &[
    "sendAudio",
    "sendCard",
    "sendCarousel",
    "sendCollection",
    "sendFile",
    "sendGif",
    "sendImage",
    "sendMessengerTemplateMessage",
    "sendMultipleImages",
    "sendProducts",
    "sendQuickReply",
    "sendText",
    "sendVideo",
    "sendViewCatalog",
    "sendWaTemplateMessage",
    "whatsappCallButton",
    "whatsappFlow",
    "whatsappLocation",
    "whatsappOptionList",
];

pub const OMNICHANNEL_DELIVERABLE_STEP_TYPES: &[&str] = &[
    "sendAudio",
    "sendCard",
    "sendCarousel",
    "sendFile",
    "sendGif",
    "sendImage",
    "sendMessengerTemplateMessage",
    "sendMultipleImages",
    "sendQuickReply",
    "sendText",
    "sendVideo",
    "sendViewCatalog",
    "sendWaTemplateMessage",
    "whatsappCallButton",
    "whatsappFlow",
    "whatsappLocation",
    "whatsappOptionList",
];

// HAND-PORT defineChannelFlowPolicy:
// definition=>{const steps=createDefaultStepSupport();for(const stepType of definition.supported??[]){steps[stepType]=stepSupport.full}for(const stepType of definition.noButtons??[]){steps[stepType]=stepSupport.noButtons}return{limits:definition.limits,quickRepliesShareButtonSlots:definition.quickRepliesShareButtonSlots??false,nativeButtonTypes:definition.nativeButtonTypes??[],steps,...definition.productMessages?{productMessages:definition.productMessages}:{}}}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `ChannelFlowPolicy.limits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelLimits {
    pub button_count: usize,
    pub button_label: usize,
    pub card_title: Option<usize>,
    pub text: usize,
    pub view_catalog: Option<SendViewCatalogLengthLimits>,
}

/// `ChannelFlowPolicy`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelFlowPolicy {
    pub limits: ChannelLimits,
    pub native_button_types: Vec<&'static str>,
    pub quick_replies_share_button_slots: bool,
    pub steps: std::collections::BTreeMap<&'static str, &'static str>,
    pub product_messages: Option<Value>,
}
