//! Port of `packages/flow-config/__tests__/authoring/normalize-graph.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::HashSet;

fn column_width() -> f64 {
    fc::default_node_measured()["width"].as_f64().unwrap() + 120.0
}

#[test]
fn normalizeauthoredgraph_lays_out_a_chain_with_no_positions_like_the_compiler_would() {
    let result = fc::normalize_authored_graph(
        &[
            json!({ "id": "a" }),
            json!({ "id": "b" }),
            json!({ "id": "c" }),
        ],
        &[
            json!({ "source": "a", "target": "b" }),
            json!({ "source": "b", "target": "c" }),
        ],
    )
    .unwrap();

    // Node order is preserved, so index still tracks the authored a/b/c order
    // even though the returned ids are freshly generated, not "a"/"b"/"c".
    let w = column_width();
    assert_eq!(result.nodes[0]["position"], json!({ "x": 100, "y": 100 }));
    assert_eq!(
        result.nodes[1]["position"],
        json!({ "x": (100.0 + w) as i64, "y": 100 })
    );
    assert_eq!(
        result.nodes[2]["position"],
        json!({ "x": (100.0 + w * 2.0) as i64, "y": 100 })
    );
    for node in &result.nodes {
        assert_eq!(&node["measured"], fc::default_node_measured());
    }
}

#[test]
fn normalizeauthoredgraph_keeps_a_caller_supplied_position_instead_of_laying_it_out() {
    let result = fc::normalize_authored_graph(
        &[json!({ "id": "a", "position": { "x": 42, "y": 7 } })],
        &[],
    )
    .unwrap();

    assert_eq!(result.nodes[0]["position"], json!({ "x": 42, "y": 7 }));
}

#[test]
fn normalizeauthoredgraph_remaps_every_authored_node_id_to_a_fresh_internal_id() {
    let result =
        fc::normalize_authored_graph(&[json!({ "id": "a" }), json!({ "id": "b" })], &[]).unwrap();

    let ids: Vec<&str> = result.nodes.iter().map(|node| s(node, "id")).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 2);
    for id in ids {
        assert_ne!(id, "a");
        assert_ne!(id, "b");
    }
}

#[test]
fn normalizeauthoredgraph_returns_the_authored_id_persisted_id_map_for_every_node() {
    let result =
        fc::normalize_authored_graph(&[json!({ "id": "a" }), json!({ "id": "b" })], &[]).unwrap();

    let mut keys: Vec<&String> = result.node_ids.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["a", "b"]);
    assert_eq!(result.node_ids["a"], s(&result.nodes[0], "id"));
    assert_eq!(result.node_ids["b"], s(&result.nodes[1], "id"));
}

#[test]
fn normalizeauthoredgraph_generates_edge_ids_and_node_id_handles_rewriting_them_to_the_remapped_node_ids()
 {
    let result = fc::normalize_authored_graph(
        &[json!({ "id": "a" }), json!({ "id": "b" }), json!({ "id": "c" })],
        &[
            json!({ "source": "a", "target": "b" }),
            json!({ "id": "custom-edge", "source": "b", "sourceHandle": "yes", "target": "c", "targetHandle": "in" }),
        ],
    )
    .unwrap();
    let (a, b, c) = (
        s(&result.nodes[0], "id"),
        s(&result.nodes[1], "id"),
        s(&result.nodes[2], "id"),
    );

    let generated = result
        .edges
        .iter()
        .find(|edge| edge["source"] == a)
        .unwrap();
    assert!(truthy(&generated["id"]));
    assert_eq!(generated["sourceHandle"], a);
    assert_eq!(generated["target"], b);
    assert_eq!(generated["targetHandle"], b);

    // A caller-supplied handle is an arbitrary handle name, not a node id —
    // it passes through untouched even though source/target are remapped.
    let supplied = result
        .edges
        .iter()
        .find(|edge| edge["id"] == "custom-edge")
        .unwrap();
    assert_eq!(
        supplied,
        &json!({ "id": "custom-edge", "source": b, "sourceHandle": "yes", "target": c, "targetHandle": "in" })
    );
}

#[test]
fn normalizeauthoredgraph_start_node_falls_back_to_the_first_node_when_none_is_flagged() {
    let result =
        fc::normalize_authored_graph(&[json!({ "id": "a" }), json!({ "id": "b" })], &[]).unwrap();

    assert_eq!(result.start_node_id, s(&result.nodes[0], "id"));
}

#[test]
fn normalizeauthoredgraph_start_node_uses_the_flagged_node_and_exactly_one_node_stays_flagged() {
    let result = fc::normalize_authored_graph(
        &[
            json!({ "id": "a", "data": { "isStartNode": true } }),
            json!({ "id": "b", "data": { "isStartNode": true } }),
        ],
        &[],
    )
    .unwrap();

    assert_eq!(result.start_node_id, s(&result.nodes[0], "id"));
    let flagged: Vec<&Value> = result
        .nodes
        .iter()
        .filter(|node| node["data"]["isStartNode"] == true)
        .collect();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0]["id"], result.nodes[0]["id"]);
}

#[test]
fn normalizeauthoredgraph_collects_a_duplicate_node_id_and_an_unknown_edge_target_in_one_exception()
{
    let error = fc::normalize_authored_graph(
        &[json!({ "id": "a" }), json!({ "id": "a" })],
        &[json!({ "source": "a", "target": "missing" })],
    )
    .expect_err("FlowAuthoringException");
    let paths: Vec<&str> = error
        .errors
        .iter()
        .map(|issue| issue.path.as_str())
        .collect();
    assert!(paths.contains(&"nodes[1].id"));
    assert!(paths.contains(&"edges[0].target"));
}
