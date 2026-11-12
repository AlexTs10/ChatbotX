//! Port of `packages/flow-config/src/steps/upstream-location-fields.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT findUpstreamLocationFieldKeys:
// function findUpstreamLocationFieldKeys(input){const{nodes,edges,stepId}=input;const start=findStartNode(nodes,stepId);if(!start){return[]}const nodesById=new Map(nodes.map(node=>[node.id,node]));const sourcesByTarget=new Map;for(const{source,target}of edges){if(typeof source!=="string"||typeof target!=="string"){continue}const sources=sourcesByTarget.get(target)??[];sources.push(source);sourcesByTarget.set(target,sources)}const keys=new Set;const collect=__name(steps=>{for(const step of steps){if(isLocationSource(step)){keys.add(step.outputFieldId)}}},"collect");collect((start.node.data?.details?.steps??[]).slice(0,start.stepsBefore));const visited=new Set;const queue=[...sourcesByTarget.get(start.node.id)??[]];while(queue.length>0){const nodeId=queue.shift();if(visited.has(nodeId)){continue}visited.add(nodeId);collect(nodesById.get(nodeId)?.data?.details?.steps??[]);queue.push(...sourcesByTarget.get(nodeId)??[])}return[...keys]}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn find_upstream_location_field_keys(
    nodes: &[Value],
    edges: &[Value],
    step_id: &str,
) -> Vec<String> {
    todo!("P3.4")
}
