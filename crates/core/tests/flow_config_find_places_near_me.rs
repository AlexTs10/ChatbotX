//! Port of `packages/flow-config/__tests__/find-places-near-me.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn reply_format(key: &str) -> Value {
    fc::reply_format()[key].clone()
}

fn node(id: &str, steps: Value) -> Value {
    json!({ "id": id, "data": { "details": { "steps": steps } } })
}

fn edge(source: &str, target: &str) -> Value {
    json!({ "source": source, "target": target })
}

fn location_step(id: &str, output_field_id: &str) -> Value {
    json!({ "id": id, "stepType": "getUserData", "replyFormat": reply_format("location"), "outputFieldId": output_field_id })
}

fn action(id: &str) -> Value {
    json!({ "id": id, "stepType": "findPlacesNearMe" })
}

fn keys(nodes: &[Value], edges: &[Value], step_id: &str) -> Vec<String> {
    fc::find_upstream_location_field_keys(nodes, edges, step_id)
}

#[test]
fn findplacesnearmestepschema_schema_accepts_null_and_a_numeric_category_id_rejects_abc() {
    let base = json!({ "id": "1", "stepType": "findPlacesNearMe" });
    let schema = fc::find_places_near_me_step_schema();
    assert!(
        schema
            .safe_parse(&spread(&base, json!({ "placeCategoryId": null })))
            .success
    );
    assert!(
        schema
            .safe_parse(&spread(&base, json!({ "placeCategoryId": "7" })))
            .success
    );
    assert!(
        !schema
            .safe_parse(&spread(&base, json!({ "placeCategoryId": "abc" })))
            .success
    );
}

#[test]
fn findplacesnearmestepschema_defaultfn_has_placecategoryid_null() {
    let step = fc::find_places_near_me_step_default_fn();
    assert_eq!(step["stepType"], "findPlacesNearMe");
    assert!(step["placeCategoryId"].is_null());
    assert!(
        fc::find_places_near_me_step_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn findupstreamlocationfieldkeys_finds_a_location_field_two_nodes_upstream() {
    let nodes = [
        node("A", json!([location_step("a1", "f1")])),
        node("B", json!([{ "id": "b1", "stepType": "sendText" }])),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "B"), edge("B", "C")];
    assert_eq!(keys(&nodes, &edges, "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_collects_every_branch() {
    let nodes = [
        node("A", json!([location_step("a1", "f1")])),
        node("B", json!([location_step("b1", "f2")])),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "C"), edge("B", "C")];
    let found = keys(&nodes, &edges, "s1");
    assert_eq!(found.len(), 2);
    assert!(found.contains(&"f1".to_owned()));
    assert!(found.contains(&"f2".to_owned()));
}

#[test]
fn findupstreamlocationfieldkeys_ignores_non_upstream_steps() {
    let nodes = [
        node("C", json!([action("s1")])),
        node("D", json!([location_step("d1", "f3")])),
        node("E", json!([location_step("e1", "f4")])),
    ];
    let edges = [edge("C", "D")];
    assert!(keys(&nodes, &edges, "s1").is_empty());
}

#[test]
fn findupstreamlocationfieldkeys_ignores_non_location_reply_formats_and_empty_outputfieldid() {
    let nodes = [
        node(
            "A",
            json!([
                spread(&location_step("a1", "f1"), json!({ "replyFormat": reply_format("text") })),
                location_step("a2", ""),
                { "id": "a3", "stepType": "getUserData", "replyFormat": reply_format("location") },
            ]),
        ),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "C")];
    assert!(keys(&nodes, &edges, "s1").is_empty());
}

#[test]
fn findupstreamlocationfieldkeys_ignores_bot_fields() {
    let nodes = [
        node("A", json!([location_step("a1", "bot_field:12")])),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "C")];
    assert!(keys(&nodes, &edges, "s1").is_empty());
}

#[test]
fn findupstreamlocationfieldkeys_handles_cycles() {
    let nodes = [
        node("A", json!([location_step("a1", "f1")])),
        node("B", json!([])),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "B"), edge("B", "C"), edge("C", "A")];
    assert_eq!(keys(&nodes, &edges, "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_unknown_step_id_returns() {
    let nodes = [node("A", json!([location_step("a1", "f1")]))];
    assert!(keys(&nodes, &[], "missing").is_empty());
}

#[test]
fn findupstreamlocationfieldkeys_dedupes() {
    let nodes = [
        node("A", json!([location_step("a1", "f1")])),
        node("B", json!([location_step("b1", "f1")])),
        node("C", json!([action("s1")])),
    ];
    let edges = [edge("A", "B"), edge("B", "C")];
    assert_eq!(keys(&nodes, &edges, "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_counts_an_earlier_step_in_the_same_node() {
    let nodes = [node("A", json!([location_step("a1", "f1"), action("s1")]))];
    assert_eq!(keys(&nodes, &[], "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_ignores_a_later_step_in_the_same_node() {
    let nodes = [node("A", json!([action("s1"), location_step("a1", "f1")]))];
    assert!(keys(&nodes, &[], "s1").is_empty());
}

#[test]
fn findupstreamlocationfieldkeys_scans_the_action_node_fully_when_a_cycle_returns_to_it() {
    let nodes = [
        node("A", json!([action("s1"), location_step("a1", "f1")])),
        node("B", json!([])),
    ];
    let edges = [edge("A", "B"), edge("B", "A")];
    assert_eq!(keys(&nodes, &edges, "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_finds_the_source_for_an_action_in_a_button_s_steps() {
    let nodes = [
        node("A", json!([location_step("a1", "f1")])),
        json!({
            "id": "B",
            "data": { "details": { "steps": [{
                "id": "t1",
                "stepType": "sendText",
                "buttons": [{ "id": "btn", "steps": [action("s1")] }],
            }] } },
        }),
    ];
    let edges = [edge("A", "B")];
    assert_eq!(keys(&nodes, &edges, "s1"), vec!["f1"]);
}

#[test]
fn findupstreamlocationfieldkeys_finds_the_source_for_an_action_in_a_quick_reply_s_steps() {
    let nodes = [json!({
        "id": "B",
        "data": { "details": {
            "steps": [location_step("a1", "f1")],
            "quickReplies": [{ "id": "qr", "steps": [action("s1")] }],
        } },
    })];
    assert_eq!(keys(&nodes, &[], "s1"), vec!["f1"]);
}
