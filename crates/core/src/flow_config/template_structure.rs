//! Port of `packages/flow-config/src/template-structure.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

fn as_upper(value: &Value) -> Option<String> {
    value.as_str().map(crate::js::to_upper)
}

fn record_array(value: &Value) -> Vec<&Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v.is_object())
        .collect()
}

fn insert_opt(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), v);
    }
}

fn normalize_button(button: &Value) -> Value {
    let url = button["url"].as_str();
    let payload = button["payload"].as_str();
    let kind = as_upper(&button["type"]).unwrap_or_default();
    let mut out = Map::new();
    out.insert("type".into(), json!(kind));
    out.insert("text".into(), json!(button["text"].as_str().unwrap_or("")));
    if kind == "PAYMENT_REQUEST" {
        let settings = if button["payment_setting"].is_object() {
            json!({ "type": button["payment_setting"]["type"].as_str() })
        } else {
            Value::Null
        };
        let probe =
            json!({ "text": button["text"].as_str().unwrap_or(""), "payment_setting": settings });
        out.insert(
            "paymentMethod".into(),
            json!(resolve_wa_payment_request_method(&probe)),
        );
    }
    insert_opt(&mut out, "url", url.map(|u| json!(u)));
    out.insert(
        "isDynamicUrl".into(),
        json!(url.is_some_and(|u| u.contains("{{1}}"))),
    );
    insert_opt(
        &mut out,
        "phoneNumber",
        button["phone_number"].as_str().map(|p| json!(p)),
    );
    out.insert(
        "hasDynamicPayload".into(),
        json!(payload.is_some_and(|p| p.contains("{{"))),
    );
    insert_opt(
        &mut out,
        "flowSourceId",
        to_optional_string(button.get("flow_id")).map(Value::String),
    );
    insert_opt(
        &mut out,
        "navigateScreenId",
        button["navigate_screen"].as_str().map(|s| json!(s)),
    );
    Value::Object(out)
}

fn normalize_component(component: &Value) -> Value {
    let cards = record_array(&component["cards"]);
    let buttons = record_array(&component["buttons"]);
    let mut out = Map::new();
    out.insert(
        "type".into(),
        json!(as_upper(&component["type"]).unwrap_or_default()),
    );
    insert_opt(
        &mut out,
        "format",
        as_upper(&component["format"]).map(Value::String),
    );
    insert_opt(
        &mut out,
        "text",
        component["text"].as_str().map(|t| json!(t)),
    );
    if !buttons.is_empty() {
        out.insert(
            "buttons".into(),
            Value::Array(buttons.into_iter().map(normalize_button).collect()),
        );
    }
    if !cards.is_empty() {
        let cards: Vec<Value> = cards
            .into_iter()
            .map(|card| {
                json!({
                    "cardIndex": if card["card_index"].is_number() { card["card_index"].clone() } else { json!(0) },
                    "components": record_array(&card["components"]).into_iter().map(normalize_component).collect::<Vec<_>>(),
                })
            })
            .collect();
        out.insert("cards".into(), Value::Array(cards));
    }
    if component["limited_time_offer"].is_object() {
        out.insert(
            "hasExpiration".into(),
            json!(component["limited_time_offer"]["has_expiration"] == true),
        );
    }
    Value::Object(out)
}

/// `buildTemplateStructure(source)`: the structural part of a template,
/// without page-specific examples.
pub fn build_template_structure(template: &Value) -> Value {
    let format = template["parameterFormat"].as_str().unwrap_or("POSITIONAL");
    json!({
        "name": template["name"],
        "language": template["language"],
        "category": crate::js::to_upper(template["category"].as_str().unwrap_or("")),
        "parameterFormat": crate::js::to_upper(format),
        "components": record_array(&template["components"]).into_iter().map(normalize_component).collect::<Vec<_>>(),
    })
}

fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                canonical_json(&map[*key], out);
            }
            out.push('}');
        }
        other => out.push_str(&crate::js::json_stringify(other)),
    }
}

/// `templateStructureKey(source)`: canonical JSON of the structure.
pub fn template_structure_key(template: &Value) -> String {
    let mut out = String::new();
    canonical_json(&build_template_structure(template), &mut out);
    out
}
