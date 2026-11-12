//! Port of `packages/flow-config/src/import-export/contact-filter-references.ts`.

use crate::flow_config::prelude::*;

pub fn contact_filter_field_reference() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"inbox":{"kind":"inbox"},"tags":{"kind":"tag"},"subscribedToDripCampaign":{"kind":"sequence"},"conversationAssigned":{"kind":"workspaceMember"},"entryPointsLinks":{"kind":"reflink"},"completedWhatsAppFlows":{"kind":"whatsappFlow"},"keywordsReceived":{"kind":"automatedResponse"},"executedFlow":{"kind":"flow"},"executedStep":{"kind":"flowNode"},"broadcastSent":{"kind":"broadcast"},"broadcastDelivered":{"kind":"broadcast"},"broadcastSeen":{"kind":"broadcast"},"broadcastClicked":{"kind":"broadcast"},"broadcastFailed":{"kind":"broadcast"},"boughtItems":{"kind":"product"},"shoppingCartContainsItems":{"kind":"product"},"commentedOnPost":{"kind":"external"},"clickedAd":{"kind":"external"},"emailSent":{"kind":"external"},"emailDelivered":{"kind":"external"},"emailOpened":{"kind":"external"},"emailClicked":{"kind":"external"}}"#).expect("valid JSON")
    });
    &V
}

/// One reference a contact filter makes (`{ kind, id }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactFilterReference {
    pub kind: String,
    pub id: String,
}

fn collect_numeric_strings(node: &Value, into: &mut Vec<String>) {
    match node {
        Value::String(text) => {
            if crate::utils::id::is_numeric_id(text) && !into.contains(text) {
                into.push(text.clone());
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_numeric_strings(item, into)),
        Value::Object(map) => map
            .values()
            .for_each(|child| collect_numeric_strings(child, into)),
        _ => {}
    }
}

/// `contactFilterReferences(filter)`: the workspace references of a contact
/// filter's conditions, deduplicated by `kind:id`, in first-seen order.
pub fn contact_filter_references(filter: &Value) -> Vec<ContactFilterReference> {
    let Some(conditions) = filter.get("conditions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut found: Vec<ContactFilterReference> = Vec::new();
    for condition in conditions {
        let Some(kind) = condition
            .get("field")
            .and_then(Value::as_str)
            .and_then(|field| contact_filter_field_reference().get(field))
            .and_then(|reference| reference["kind"].as_str())
        else {
            continue;
        };
        let mut ids = Vec::new();
        collect_numeric_strings(condition.get("value").unwrap_or(&Value::Null), &mut ids);
        for id in ids {
            let reference = ContactFilterReference {
                kind: kind.to_string(),
                id,
            };
            if !found.contains(&reference) {
                found.push(reference);
            }
        }
    }
    found
}
