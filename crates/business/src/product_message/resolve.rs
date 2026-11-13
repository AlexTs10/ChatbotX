//! `product-message/resolve-message.ts`: the content of a `sendProducts` step
//! (JSON, `SendProductsContent`) against the sendable items (`ProductMessageItem`
//! JSON keyed by product id) and the channel's `productMessages` capability.

use std::collections::HashMap;

use chatbotx_core::flow_config::channel_rules::{
    evaluate_product_message, to_product_message_shape,
};
use serde_json::{Map, Value, json};

const TEXT_FIELDS: [&str; 3] = ["header", "body", "footer"];

/// `value?.trim() || undefined`.
fn text_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

struct Draft {
    texts: Vec<(&'static str, Option<String>)>,
    sections: Vec<(Option<String>, Vec<Value>)>,
}

impl Draft {
    fn text(&self, key: &str) -> Option<&String> {
        self.texts
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.as_ref())
    }

    /// `draftToShape` (absent keys left out).
    fn shape(&self, display: &Value) -> Value {
        let mut out = Map::new();
        out.insert("display".into(), display.clone());
        for (key, value) in &self.texts {
            if let Some(value) = value {
                out.insert((*key).into(), json!(value));
            }
        }
        let sections: Vec<Value> = self
            .sections
            .iter()
            .map(|(title, items)| {
                let mut section = json!({ "productCount": items.len() });
                if let Some(title) = title {
                    section["title"] = json!(title);
                }
                section
            })
            .collect();
        out.insert("sections".into(), Value::Array(sections));
        Value::Object(out)
    }
}

/// `ProductMessageResolution`.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// `{ ok: true, message, missing }` (`missing` = product ids).
    Ok {
        message: Value,
        missing: Vec<String>,
    },
    /// `{ ok: false, reason }`: `noProductsSendable` or `invalidMessage`.
    Failed(&'static str),
}

/// `resolveProductMessage`.
pub fn resolve_product_message(
    capability: &Value,
    content: &Value,
    sendable_by_id: &HashMap<String, Value>,
) -> Resolution {
    if !evaluate_product_message(capability, &to_product_message_shape(content)).is_empty() {
        return Resolution::Failed("invalidMessage");
    }
    let display = content.get("display").cloned().unwrap_or(Value::Null);
    let empty = Vec::new();
    let product_ids = |section: &Value| -> Vec<String> {
        section
            .get("productIds")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .filter_map(|id| id.as_str().map(str::to_string))
            .collect()
    };
    let sections: Vec<(Option<String>, Vec<Value>)> = content
        .get("sections")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .map(|section| {
            let items = product_ids(section)
                .iter()
                .filter_map(|id| sendable_by_id.get(id).cloned())
                .collect::<Vec<_>>();
            (text_or_none(section.get("title")), items)
        })
        .filter(|(_, items)| !items.is_empty())
        .collect();
    let mut draft = Draft {
        texts: TEXT_FIELDS
            .iter()
            .map(|k| (*k, text_or_none(content.get(*k))))
            .collect(),
        sections,
    };
    // `withoutFieldsNoLongerAllowed`.
    let not_allowed: Vec<String> = evaluate_product_message(capability, &draft.shape(&display))
        .iter()
        .filter(|issue| issue["code"] == "sendProductsFieldNotAllowed")
        .filter_map(|issue| match issue["path"].as_array() {
            Some(path) if path.len() == 1 => path[0].as_str().map(str::to_string),
            _ => None,
        })
        .collect();
    for (key, value) in &mut draft.texts {
        if not_allowed.iter().any(|n| n == key) {
            *value = None;
        }
    }
    if !evaluate_product_message(capability, &draft.shape(&display)).is_empty() {
        return Resolution::Failed("noProductsSendable");
    }
    let message = if display == "carousel" {
        json!({
            "display": "carousel",
            "body": draft.text("body").cloned().unwrap_or_default(),
            "items": draft.sections.iter().flat_map(|(_, items)| items.iter().cloned()).collect::<Vec<_>>(),
        })
    } else {
        let mut out = Map::new();
        out.insert("display".into(), json!("products"));
        for key in TEXT_FIELDS {
            if let Some(value) = draft.text(key) {
                out.insert(key.into(), json!(value));
            }
        }
        let sections: Vec<Value> = draft
            .sections
            .iter()
            .map(|(title, items)| {
                let mut section = json!({ "items": items });
                if let Some(title) = title {
                    section["title"] = json!(title);
                }
                section
            })
            .collect();
        out.insert("sections".into(), Value::Array(sections));
        Value::Object(out)
    };
    let missing = content
        .get("sections")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .flat_map(product_ids)
        .filter(|id| !sendable_by_id.contains_key(id))
        .collect();
    Resolution::Ok { message, missing }
}
