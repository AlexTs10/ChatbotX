//! Port of `packages/flow-config/src/migrations/spreadsheet-write-mapping.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

fn is_step_record(value: &Value) -> bool {
    value.is_object() && value["stepType"].is_string()
}

fn map_step_array(value: &Value, transform: &dyn Fn(&Value) -> Value) -> Value {
    match value {
        Value::Array(steps) => Value::Array(
            steps
                .iter()
                .map(|s| map_step_record(s, transform))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn map_button_steps(value: &Value, transform: &dyn Fn(&Value) -> Value) -> Value {
    let Value::Object(map) = value else {
        return value.clone();
    };
    let mut next = map.clone();
    if let Some(before) = map.get("beforeStep") {
        next.insert("beforeStep".into(), map_step_record(before, transform));
    }
    if let Some(steps) = map.get("steps") {
        next.insert("steps".into(), map_step_array(steps, transform));
    }
    Value::Object(next)
}

fn map_button_array(value: &Value, transform: &dyn Fn(&Value) -> Value) -> Value {
    match value {
        Value::Array(buttons) => Value::Array(
            buttons
                .iter()
                .map(|b| map_button_steps(b, transform))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn map_step_record(value: &Value, transform: &dyn Fn(&Value) -> Value) -> Value {
    if !is_step_record(value) {
        return value.clone();
    }
    let mut step = transform(value);
    if step["buttons"].is_array() {
        step["buttons"] = map_button_array(&step["buttons"], transform);
    }
    if let Some(cards) = step["cards"].as_array().cloned() {
        step["cards"] = Value::Array(
            cards
                .into_iter()
                .map(|card| {
                    if card.is_object() && card["buttons"].is_array() {
                        let mut card = card;
                        card["buttons"] = map_button_array(&card["buttons"], transform);
                        card
                    } else {
                        card
                    }
                })
                .collect(),
        );
    }
    if step["elements"].is_array() {
        step["elements"] = map_button_array(&step["elements"], transform);
    }
    step
}

/// `mapNodeSteps(details, transform)`: applies `transform` to every step of a
/// node, nested ones (buttons, cards, page elements, quick replies) included.
pub fn map_node_steps(details: &Value, transform: &dyn Fn(&Value) -> Value) -> Value {
    let Value::Object(map) = details else {
        return details.clone();
    };
    let mut next = map.clone();
    if let Some(before) = map.get("beforeStep") {
        next.insert("beforeStep".into(), map_step_record(before, transform));
    }
    if let Some(steps) = map.get("steps") {
        next.insert("steps".into(), map_step_array(steps, transform));
    }
    if let Some(replies) = map.get("quickReplies") {
        let mapped = match replies {
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|b| map_button_steps(b, transform))
                    .collect(),
            ),
            other => other.clone(),
        };
        next.insert("quickReplies".into(), mapped);
    }
    Value::Object(next)
}

/// `toCustomFieldToken({ name })`.
pub fn to_custom_field_token(field: &Value) -> String {
    format!("{{{{raw:{}}}}}", str_field(field, "name"))
}

fn convert_legacy_map_item(value: &Value, lookup: &dyn Fn(&str) -> Option<Value>) -> Option<Value> {
    if !value.is_object() {
        return None;
    }
    let custom_field_id = value["customFieldId"].as_str().unwrap_or("");
    let mut out = value.clone();
    if custom_field_id.is_empty() {
        out["value"] = json!("");
        return Some(out);
    }
    let field = lookup(custom_field_id)?;
    let name = str_field(&field, "name");
    if name.contains(['{', '}', '\n', '\r']) {
        return None;
    }
    out["value"] = json!(to_custom_field_token(&field));
    Some(out)
}

/// `upgradeSpreadsheetWriteStep(step, lookup)`: a v1 step's custom-field
/// mappings become `{{raw:…}}` tokens, all or nothing.
pub fn upgrade_spreadsheet_write_step(
    step: &Value,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Value {
    if !step.is_object() || to_spreadsheet_step_version(step["version"].as_str()) == "v2" {
        return step.clone();
    }
    let map = arr_field(step, "map");
    if map.is_empty() {
        return step.clone();
    }
    let Some(converted) = map
        .iter()
        .map(|item| convert_legacy_map_item(item, lookup))
        .collect::<Option<Vec<_>>>()
    else {
        return step.clone();
    };
    let mut out = step.clone();
    out["version"] = json!("v2");
    out["map"] = Value::Array(converted);
    out
}

/// `tagSpreadsheetWriteStepVersion(step)`: an untagged step is `v1`.
pub fn tag_spreadsheet_write_step_version(step: &Value) -> Value {
    if !step.is_object() || step.get("version").is_some() {
        return step.clone();
    }
    let mut out = step.clone();
    out["version"] = json!("v1");
    out
}

fn is_spreadsheet_write_step(step: &Value) -> bool {
    matches!(
        step["stepType"].as_str(),
        Some("spreadsheetSendData" | "spreadsheetUpdateRow")
    )
}

/// `upgradeNodeSteps(details, lookup)`.
pub fn upgrade_node_steps(details: &Value, lookup: &dyn Fn(&str) -> Option<Value>) -> Value {
    map_node_steps(details, &|step| {
        if is_spreadsheet_write_step(step) {
            upgrade_spreadsheet_write_step(step, lookup)
        } else {
            step.clone()
        }
    })
}

/// `tagNodeSpreadsheetWriteStepVersions(details)`.
pub fn tag_node_spreadsheet_write_step_versions(details: &Value) -> Value {
    map_node_steps(details, &|step| {
        if is_spreadsheet_write_step(step) {
            tag_spreadsheet_write_step_version(step)
        } else {
            step.clone()
        }
    })
}
