//! Port of `packages/flow-config/src/import-export/references.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT collectFlowReferenceWarnings:
// flow=>{const warnings=[];remapFlowGraphReferences(flow,{},{onUnresolved:__name(ref=>warnings.push(ref),"onUnresolved")});return warnings}

// HAND-PORT collectFieldReferences:
// flow=>{const ids={customFieldIds:new Set,botFieldIds:new Set};collectFieldReferenceIds(flow.nodes,ids);collectFieldReferenceIds(flow.edges,ids);return{customFieldIds:[...ids.customFieldIds],botFieldIds:[...ids.botFieldIds]}}

// HAND-PORT collectCustomFieldReferences:
// flow=>collectFieldReferences(flow).customFieldIds

// HAND-PORT remapCustomFieldReferences:
// (flow,idMap)=>remapFlowGraphReferences(flow,{customField:idMap},{kinds:["customField"]})
