//! Port of `packages/flow-config/__tests__/authoring/layout.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::{LayoutEdge, LayoutPosition};

fn expect_finite_position(position: Option<&LayoutPosition>) {
    let position = position.expect("position is defined");
    assert!(position.x.is_finite());
    assert!(position.y.is_finite());
}

fn edge<'a>(source: &'a str, target: &'a str) -> LayoutEdge<'a> {
    LayoutEdge { source, target }
}

#[test]
fn layoutnodes_places_an_unreachable_node_in_a_fallback_column() {
    let positions = fc::layout_nodes(
        &["start", "reachable", "unreachable"],
        &[edge("start", "reachable")],
        "start",
    );
    let reachable_position = positions.get("reachable");
    let unreachable_position = positions.get("unreachable");

    expect_finite_position(unreachable_position);
    assert!(unreachable_position.unwrap().x > reachable_position.map_or(0.0, |p| p.x));
}

#[test]
fn layoutnodes_terminates_on_cycles_and_positions_every_node() {
    let node_ids = ["start", "first", "second"];
    let positions = fc::layout_nodes(
        &node_ids,
        &[
            edge("start", "first"),
            edge("first", "second"),
            edge("second", "first"),
        ],
        "start",
    );

    assert_eq!(positions.len(), node_ids.len());
    for node_id in node_ids {
        expect_finite_position(positions.get(node_id));
    }
}
