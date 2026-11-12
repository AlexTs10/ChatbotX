//! Port of `packages/flow-config/src/import-export/references.ts`.

use std::collections::BTreeMap;

use crate::flow_config::prelude::*;
use crate::utils::id::is_numeric_id;

/// `FlowReferenceWarning`.
pub type FlowReferenceWarning = UnresolvedReference;

/// `collectFlowReferenceWarnings(flow)`: every reference slot of the graph
/// (empty id maps, so every in-scope reference is reported). Read-only.
pub fn collect_flow_reference_warnings(flow: &Value) -> Vec<FlowReferenceWarning> {
    let mut warnings = Vec::new();
    let mut push = |reference: &UnresolvedReference| warnings.push(reference.clone());
    remap_flow_graph_references(
        flow,
        &ReferenceIdMaps::new(),
        RemapOptions {
            kinds: None,
            on_unresolved: Some(&mut push),
        },
    );
    warnings
}

/// `FlowFieldReferenceIds`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowFieldReferenceIds {
    pub custom_field_ids: Vec<String>,
    pub bot_field_ids: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, id: &str) {
    if !list.iter().any(|existing| existing == id) {
        list.push(id.to_string());
    }
}

fn collect_field_reference_ids(value: &Value, ids: &mut FlowFieldReferenceIds, depth: usize) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    let object = match value {
        Value::Array(items) => {
            for item in items {
                collect_field_reference_ids(item, ids, depth + 1);
            }
            return;
        }
        Value::Object(object) => object,
        _ => return,
    };
    for (key, child) in object {
        let kind = reference_field_entity_kind()
            .get(key)
            .and_then(Value::as_str);
        match (kind, child.as_str()) {
            (Some("customField"), Some(text)) => match parse_field_reference(text) {
                FieldReference::BotField { id } => push_unique(&mut ids.bot_field_ids, &id),
                FieldReference::CustomField { .. } if is_numeric_id(text) => {
                    push_unique(&mut ids.custom_field_ids, text)
                }
                FieldReference::CustomField { .. } => {}
            },
            // A dedicated botField-kind key (the Condition step's `botFieldId`) holds a raw id.
            (Some("botField"), Some(text)) if is_numeric_id(text) => {
                push_unique(&mut ids.bot_field_ids, text)
            }
            _ => {}
        }
        collect_field_reference_ids(child, ids, depth + 1);
    }
}

/// `collectFieldReferences(flow)`: numeric custom-field ids and bot-field ids
/// referenced by the graph, deduplicated, in first-seen order.
pub fn collect_field_references(flow: &Value) -> FlowFieldReferenceIds {
    let mut ids = FlowFieldReferenceIds::default();
    collect_field_reference_ids(&flow["nodes"], &mut ids, 0);
    collect_field_reference_ids(&flow["edges"], &mut ids, 0);
    ids
}

/// `collectCustomFieldReferences(flow)`.
pub fn collect_custom_field_references(flow: &Value) -> Vec<String> {
    collect_field_references(flow).custom_field_ids
}

/// `remapCustomFieldReferences(flow, idMap)`: custom-field slots only.
pub fn remap_custom_field_references(flow: &Value, id_map: &BTreeMap<String, String>) -> Value {
    let mut id_maps = ReferenceIdMaps::new();
    id_maps.insert("customField".to_string(), id_map.clone());
    remap_flow_graph_references(
        flow,
        &id_maps,
        RemapOptions {
            kinds: Some(vec!["customField".to_string()]),
            on_unresolved: None,
        },
    )
}
