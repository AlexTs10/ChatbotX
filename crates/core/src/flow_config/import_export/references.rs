// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/import-export/references.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};

// HAND-PORT collectFlowReferenceWarnings:
// flow=>{const warnings=[];remapFlowGraphReferences(flow,{},{onUnresolved:__name(ref=>warnings.push(ref),"onUnresolved")});return warnings}

// HAND-PORT collectFieldReferences:
// flow=>{const ids={customFieldIds:new Set,botFieldIds:new Set};collectFieldReferenceIds(flow.nodes,ids);collectFieldReferenceIds(flow.edges,ids);return{customFieldIds:[...ids.customFieldIds],botFieldIds:[...ids.botFieldIds]}}

// HAND-PORT collectCustomFieldReferences:
// flow=>collectFieldReferences(flow).customFieldIds

// HAND-PORT remapCustomFieldReferences:
// (flow,idMap)=>remapFlowGraphReferences(flow,{customField:idMap},{kinds:["customField"]})

/// `FlowReferenceWarning`.
pub type FlowReferenceWarning = UnresolvedReference;

/// `collectFlowReferenceWarnings(flow)`: every reference slot of the graph
/// (empty id maps, so every in-scope reference is reported). Read-only.
pub fn collect_flow_reference_warnings(flow: &Value) -> Vec<FlowReferenceWarning> {
    todo!("P3.4b")
}

/// `FlowFieldReferenceIds`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowFieldReferenceIds {
    pub custom_field_ids: Vec<String>,
    pub bot_field_ids: Vec<String>,
}

/// `collectFieldReferences(flow)`: numeric custom-field ids and bot-field ids
/// referenced by the graph, deduplicated, in first-seen order.
pub fn collect_field_references(flow: &Value) -> FlowFieldReferenceIds {
    todo!("P3.4b")
}

/// `collectCustomFieldReferences(flow)`.
pub fn collect_custom_field_references(flow: &Value) -> Vec<String> {
    todo!("P3.4b")
}

/// `remapCustomFieldReferences(flow, idMap)`: custom-field slots only.
pub fn remap_custom_field_references(flow: &Value, id_map: &BTreeMap<String, String>) -> Value {
    todo!("P3.4b")
}
