//! Port of `packages/flow-config/src/authoring/normalize-graph.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT normalizeFlowGraphDecisionMappings:
// nodes=>nodes.map(node=>{if(!node||typeof node!=="object"||Array.isArray(node)){return node}const nodeRecord=node;const data=nodeRecord.data;if(!data||typeof data!=="object"||Array.isArray(data)){return node}const details=data.details;if(!details||typeof details!=="object"||Array.isArray(details)){return node}const steps=details.steps;if(!Array.isArray(steps)){return node}return{...nodeRecord,data:{...data,details:{...details,steps:steps.map(step=>{if(!step||typeof step!=="object"||Array.isArray(step)){return step}const stepRecord=step;if(stepRecord.stepType!=="evaluateDecision"){return step}return normalizeEvaluateDecisionStep(step)})}}}})

// HAND-PORT normalizeAuthoredGraph:
// function normalizeAuthoredGraph(nodes,edges){const errors=[];if(nodes.length===0){errors.push({path:"nodes",code:"invalidGraph",message:"A flow needs at least one node."})}const seenNodeIds=new Set;for(const[index,node]of nodes.entries()){if(seenNodeIds.has(node.id)){errors.push({path:`nodes[${index}].id`,code:"invalidGraph",message:`Duplicate node id "${node.id}".`});continue}seenNodeIds.add(node.id)}for(const[index,edge]of edges.entries()){if(!seenNodeIds.has(edge.source)){errors.push({path:`edges[${index}].source`,code:"invalidGraph",message:`Edge source "${edge.source}" does not match any node id.`})}if(!seenNodeIds.has(edge.target)){errors.push({path:`edges[${index}].target`,code:"invalidGraph",message:`Edge target "${edge.target}" does not match any node id.`})}}if(errors.length>0){throw new FlowAuthoringException(errors)}const idByAuthoredId=new Map(nodes.map(node=>[node.id,createId()]));const internalId=__name(authoredId=>{const mapped=idByAuthoredId.get(authoredId);if(!mapped){throw new Error(`Unmapped authored node id "${authoredId}".`)}return mapped},"internalId");const startNodeAuthoredId=nodes.find(node=>node.data?.isStartNode===true)?.id??nodes[0].id;const startNodeId=internalId(startNodeAuthoredId);const positions=layoutNodes(nodes.map(node=>node.id),edges,startNodeAuthoredId);const resolvedNodes=nodes.map(node=>{const id=internalId(node.id);return{...node,id,position:node.position??positions.get(node.id),measured:node.measured??DEFAULT_NODE_MEASURED,data:{...node.data,isStartNode:id===startNodeId}}});const resolvedEdges=edges.map(edge=>{const source=internalId(edge.source);const target=internalId(edge.target);return{id:edge.id??createId(),source,sourceHandle:edge.sourceHandle??source,target,targetHandle:edge.targetHandle??target}});return{nodes:resolvedNodes,edges:resolvedEdges,startNodeId,nodeIds:Object.fromEntries(idByAuthoredId)}}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `NormalizedAuthoredGraph`.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedAuthoredGraph {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub start_node_id: String,
    pub node_ids: std::collections::BTreeMap<String, String>,
}

pub fn normalize_authored_graph(
    nodes: &[Value],
    edges: &[Value],
) -> Result<NormalizedAuthoredGraph, FlowAuthoringException> {
    todo!("P3.4")
}
