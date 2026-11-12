// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/import-export/remap.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};

// HAND-PORT remapReferences:
// (value,idMaps,options)=>remapValue(value,"",idMaps,options,0,void 0)

// HAND-PORT remapFlowGraphReferences:
// (value,idMaps,options)=>({...value,nodes:remapValue(value.nodes,"nodes",idMaps,options,0,void 0),edges:remapValue(value.edges,"edges",idMaps,options,0,void 0)})

// HAND-PORT collectReferencesByKind:
// (value,kinds)=>{const byKind=new Map;const onUnresolved=__name(({entityKind,value:id})=>{const set=byKind.get(entityKind)??new Set;set.add(id);byKind.set(entityKind,set)},"onUnresolved");remapReferences(value,{},{kinds,onUnresolved});return byKind}

/// `UnresolvedReference`: a reference slot whose value has no entry in its kind's id map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedReference {
    pub entity_kind: String,
    pub path: String,
    pub value: String,
}

/// `ReferenceIdMaps`: entity kind -> (source id -> target id).
pub type ReferenceIdMaps = BTreeMap<String, BTreeMap<String, String>>;

/// `RemapOptions`.
#[derive(Default)]
pub struct RemapOptions<'a> {
    /// Restrict remapping (and warning collection) to these entity kinds.
    pub kinds: Option<Vec<String>>,
    /// Called once per reference slot whose value has no entry in its kind's map.
    pub on_unresolved: Option<&'a mut dyn FnMut(&UnresolvedReference)>,
}

/// `remapReferences(value, idMaps, options)`: a copy with every known reference
/// slot rewritten through `idMaps`; misses pass through and are reported.
pub fn remap_references(
    value: &Value,
    id_maps: &ReferenceIdMaps,
    options: RemapOptions<'_>,
) -> Value {
    todo!("P3.4b")
}

/// `remapFlowGraphReferences(flow, idMaps, options)`: remaps `nodes` and `edges`
/// (paths rooted at `nodes` / `edges`).
pub fn remap_flow_graph_references(
    value: &Value,
    id_maps: &ReferenceIdMaps,
    options: RemapOptions<'_>,
) -> Value {
    todo!("P3.4b")
}

/// `collectReferencesByKind(value, kinds)`: every reference id found, by entity kind.
pub fn collect_references_by_kind(
    value: &Value,
    kinds: Option<&[&str]>,
) -> BTreeMap<String, BTreeSet<String>> {
    todo!("P3.4b")
}
