//! Port of `packages/flow-config/src/routable-handle.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn routable_handle_kinds() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"stepButtons":"stepButtons","cardButtons":"cardButtons","listOptions":"listOptions","pageElements":"pageElements","quickReplySettings":"quickReplySettings"}"#).expect("valid JSON")
    });
    &V
}

// HAND-PORT findButtonInNodes:
// (nodes,handleId)=>{for(const node of nodes){let firstMatch=null;for(const accessor of routableHandleAccessors){const found=accessor.findButton(node,handleId);if(found&&(!firstMatch||found.stepIndex<firstMatch.stepIndex)){firstMatch=found}}if(firstMatch){const{button,runtimeNodeId}=firstMatch;return{button,runtimeNodeId}}}return null}

// HAND-PORT applyRouteInNode:
// (sourceNode,handleId,route)=>{let firstUpdate=null;for(const accessor of routableHandleAccessors){const update=accessor.applyRoute(sourceNode,handleId,route);if(update&&(!firstUpdate||update.stepIndex<firstUpdate.stepIndex)){firstUpdate=update}}return firstUpdate?.node??null}

// HAND-PORT applyRouteUpdatesInNodes:
// (nodes,updates)=>{const updatesBySourceNode=new Map;for(const update of updates){const sourceUpdates=updatesBySourceNode.get(update.sourceNodeId);if(sourceUpdates){sourceUpdates.push(update)}else{updatesBySourceNode.set(update.sourceNodeId,[update])}}let hasChanges=false;const updatedNodes=nodes.map(node=>{const sourceUpdates=updatesBySourceNode.get(node.id);if(!sourceUpdates){return node}let updatedNode=node;for(const update of sourceUpdates){const nextNode=applyRouteInNode(updatedNode,update.handleId,update.route);if(nextNode){updatedNode=nextNode;hasChanges=true}}return updatedNode});return hasChanges?updatedNodes:nodes}
