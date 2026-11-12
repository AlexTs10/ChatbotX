//! Port of `packages/flow-config/src/authoring/layout.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

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

const COLUMN_GAP: f64 = 120.0;
const ROW_GAP: f64 = 80.0;
const ORIGIN_X: f64 = 100.0;
const ORIGIN_Y: f64 = 100.0;

/// `layoutNodes(nodeIds, edges, startNodeId)`: column by BFS depth, row by
/// first visit within the depth; unreachable nodes get trailing columns.
pub fn layout_nodes(
    node_ids: &[&str],
    edges: &[LayoutEdge<'_>],
    start_node_id: &str,
) -> std::collections::HashMap<String, LayoutPosition> {
    let measured = default_node_measured();
    let column_width = measured["width"].as_f64().unwrap_or(288.0) + COLUMN_GAP;
    let row_height = measured["height"].as_f64().unwrap_or(100.0) + ROW_GAP;
    let mut children: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for edge in edges {
        children.entry(edge.source).or_default().push(edge.target);
    }
    let mut positions = std::collections::HashMap::new();
    let mut rows: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut place = |positions: &mut std::collections::HashMap<String, LayoutPosition>,
                     node_id: &str,
                     depth: usize| {
        if positions.contains_key(node_id) {
            return;
        }
        let row = rows.entry(depth).or_insert(0);
        let current = *row;
        *row += 1;
        positions.insert(
            node_id.to_owned(),
            LayoutPosition {
                x: ORIGIN_X + depth as f64 * column_width,
                y: ORIGIN_Y + current as f64 * row_height,
            },
        );
    };
    let mut frontier = vec![start_node_id];
    let mut depth = 0usize;
    let mut visited = std::collections::HashSet::new();
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for node_id in frontier {
            if !visited.insert(node_id) {
                continue;
            }
            place(&mut positions, node_id, depth);
            for child in children.get(node_id).into_iter().flatten() {
                if !visited.contains(child) {
                    next.push(*child);
                }
            }
        }
        frontier = next;
        depth += 1;
    }
    let mut fallback = depth;
    for node_id in node_ids {
        if !positions.contains_key(*node_id) {
            place(&mut positions, node_id, fallback);
            fallback += 1;
        }
    }
    positions
}
