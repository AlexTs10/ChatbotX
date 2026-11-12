//! Port of `packages/flow-config/src/import-export/contact-filter-references.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn contact_filter_field_reference() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"inbox":{"kind":"inbox"},"tags":{"kind":"tag"},"subscribedToDripCampaign":{"kind":"sequence"},"conversationAssigned":{"kind":"workspaceMember"},"entryPointsLinks":{"kind":"reflink"},"completedWhatsAppFlows":{"kind":"whatsappFlow"},"keywordsReceived":{"kind":"automatedResponse"},"executedFlow":{"kind":"flow"},"executedStep":{"kind":"flowNode"},"broadcastSent":{"kind":"broadcast"},"broadcastDelivered":{"kind":"broadcast"},"broadcastSeen":{"kind":"broadcast"},"broadcastClicked":{"kind":"broadcast"},"broadcastFailed":{"kind":"broadcast"},"boughtItems":{"kind":"product"},"shoppingCartContainsItems":{"kind":"product"},"commentedOnPost":{"kind":"external"},"clickedAd":{"kind":"external"},"emailSent":{"kind":"external"},"emailDelivered":{"kind":"external"},"emailOpened":{"kind":"external"},"emailClicked":{"kind":"external"}}"#).expect("valid JSON")
    });
    &V
}

// HAND-PORT contactFilterReferences:
// filter=>{const conditions=filter?.conditions;if(!Array.isArray(conditions)){return[]}const found=new Map;for(const condition of conditions){const{field,value}=condition??{};const reference=typeof field==="string"?CONTACT_FILTER_FIELD_REFERENCE[field]:void 0;if(!reference){continue}const ids=new Set;collectNumericStrings(value,ids);for(const id of ids){found.set(`${reference.kind}:${id}`,{kind:reference.kind,id})}}return[...found.values()]}
