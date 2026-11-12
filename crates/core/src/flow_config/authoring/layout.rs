//! Port of `packages/flow-config/src/authoring/layout.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT layoutNodes:
// function layoutNodes(nodeIds,edges,startNodeId){const childrenBySource=new Map;for(const edge of edges){const children=childrenBySource.get(edge.source);if(children){children.push(edge.target)}else{childrenBySource.set(edge.source,[edge.target])}}const positions=new Map;const rowCountByDepth=new Map;const place=__name((nodeId,depth2)=>{if(positions.has(nodeId)){return}const row=rowCountByDepth.get(depth2)??0;rowCountByDepth.set(depth2,row+1);positions.set(nodeId,{x:ORIGIN.x+depth2*COLUMN_WIDTH,y:ORIGIN.y+row*ROW_HEIGHT})},"place");let frontier=[startNodeId];let depth=0;const visited=new Set;while(frontier.length>0){const nextFrontier=[];for(const nodeId of frontier){if(visited.has(nodeId)){continue}visited.add(nodeId);place(nodeId,depth);for(const child of childrenBySource.get(nodeId)??[]){if(!visited.has(child)){nextFrontier.push(child)}}}frontier=nextFrontier;depth+=1}let fallbackDepth=depth;for(const nodeId of nodeIds){if(!positions.has(nodeId)){place(nodeId,fallbackDepth);fallbackDepth+=1}}return positions}

// ---- stage 1 API (hand-ported in stage 2) ----

/// An edge for layout (`Pick<EdgeSchema, "source" | "target">`).
#[derive(Debug, Clone, Copy)]
pub struct LayoutEdge<'a> {
    pub source: &'a str,
    pub target: &'a str,
}

/// `LayoutPosition`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutPosition {
    pub x: f64,
    pub y: f64,
}

pub fn layout_nodes(
    node_ids: &[&str],
    edges: &[LayoutEdge<'_>],
    start_node_id: &str,
) -> std::collections::HashMap<String, LayoutPosition> {
    todo!("P3.4")
}
