// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/import-export/remap.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT remapReferences:
// (value,idMaps,options)=>remapValue(value,"",idMaps,options,0,void 0)

// HAND-PORT remapFlowGraphReferences:
// (value,idMaps,options)=>({...value,nodes:remapValue(value.nodes,"nodes",idMaps,options,0,void 0),edges:remapValue(value.edges,"edges",idMaps,options,0,void 0)})

// HAND-PORT collectReferencesByKind:
// (value,kinds)=>{const byKind=new Map;const onUnresolved=__name(({entityKind,value:id})=>{const set=byKind.get(entityKind)??new Set;set.add(id);byKind.set(entityKind,set)},"onUnresolved");remapReferences(value,{},{kinds,onUnresolved});return byKind}
