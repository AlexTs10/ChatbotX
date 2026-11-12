//! Port of `packages/flow-config/__tests__/routable-handle.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::FlowRouteUpdate;

fn make_message_node(id: &str, detail_props: Value) -> Value {
    fc::send_message_node_default_fn(
        &json!({ "nodeProps": { "id": id }, "detailProps": detail_props }),
    )
}

fn make_button(id: &str, label: Option<&str>) -> Value {
    spread(
        &fc::button_step_default_fn(&json!({ "label": label.unwrap_or(id) })),
        json!({ "id": id }),
    )
}

fn steps(node: &Value) -> &Vec<Value> {
    node["data"]["details"]["steps"]
        .as_array()
        .expect("Expected a node with steps")
}

/// Mirrors a flow version persisted before `quickReplies` joined the schema.
fn strip_quick_replies(node: &Value) -> Value {
    let mut out = node.clone();
    out["data"]["details"] = without(&node["data"]["details"], &["quickReplies"]);
    out
}

/// Mirrors a card persisted before #381, with no `buttons` key at all.
fn strip_card_buttons(card: Value) -> Value {
    without(&card, &["buttons"])
}

fn mail_elements(node: &Value) -> &Vec<Value> {
    node["data"]["details"]["steps"][0]["elements"]
        .as_array()
        .expect("Expected a send-mail node")
}

fn get_node_from_button(nodes: &[Value], id: &str) -> Value {
    let (button, node_id) = fc::get_node_from_button(nodes, id);
    json!({ "button": button, "nodeId": node_id })
}

fn target(id: &str) -> Option<&str> {
    Some(id)
}

fn card(id: Option<&str>, title: &str, buttons: Value) -> Value {
    let mut card = spread(
        &fc::send_card_step_default_fn(),
        json!({ "title": title, "buttons": buttons }),
    );
    if let Some(id) = id {
        card["id"] = json!(id);
    }
    card
}

fn carousel(cards: Value) -> Value {
    spread(
        &fc::send_carousel_step_default_fn(),
        json!({ "cards": cards }),
    )
}

fn text(props: Value) -> Value {
    fc::send_text_step_default_fn(&props)
}

#[test]
fn getnodefrombutton_finds_top_level_and_carousel_buttons_without_changing_the_result_shape() {
    let top_level_button = make_button("top-level", Some("Top level"));
    let card_buttons = [
        make_button("card-first", Some("First")),
        make_button("card-middle", Some("Middle")),
        make_button("card-last", Some("Last")),
    ];
    let cards: Vec<Value> = card_buttons
        .iter()
        .enumerate()
        .map(|(index, button)| {
            card(
                Some(&format!("card-{index}")),
                &format!("Card {index}"),
                json!([button]),
            )
        })
        .collect();
    let node = make_message_node(
        "owner-node",
        json!({ "steps": [text(json!({ "buttons": [top_level_button], "text": "Hello" })), carousel(json!(cards))] }),
    );

    assert_eq!(
        get_node_from_button(std::slice::from_ref(&node), s(&top_level_button, "id")),
        json!({ "button": top_level_button, "nodeId": "owner-node" })
    );
    for button in &card_buttons {
        assert_eq!(
            get_node_from_button(std::slice::from_ref(&node), s(button, "id")),
            json!({ "button": button, "nodeId": "owner-node" })
        );
    }
}

#[test]
fn getnodefrombutton_prefers_a_step_nodeid_and_preserves_whatsapp_option_synthesis() {
    let button = make_button("button", None);
    let text_step = text(json!({ "buttons": [button], "nodeId": "runtime-node", "text": "Hello" }));
    let option_step = fc::whatsapp_option_list_step_default_fn(&json!({
        "nodeId": "option-runtime-node",
        "options": [{ "id": "option", "title": "Choice", "description": "Description" }],
    }));
    let node = make_message_node("owner-node", json!({ "steps": [text_step, option_step] }));

    assert_eq!(
        get_node_from_button(std::slice::from_ref(&node), s(&button, "id"))["nodeId"],
        "runtime-node"
    );
    assert_eq!(
        get_node_from_button(&[node], "option"),
        json!({
            "button": { "id": "option", "label": "Choice", "buttonType": null, "beforeStep": null, "steps": [] },
            "nodeId": "option-runtime-node",
        })
    );
}

fn make_mail_node(id: &str, button_id: &str) -> Value {
    let button = json!({
        "id": button_id,
        "type": "button",
        "label": "Email button",
        "buttonType": "startAnotherNode",
        "beforeStep": spread(
            &fc::start_another_node_step_default_fn(&json!({ "nodeId": "old-target", "viewOnly": true })),
            json!({ "id": "email-before-step" }),
        ),
        "steps": [],
    });
    let mut node = fc::send_mail_node_default_fn(&json!({ "nodeProps": { "id": id } }));
    node["data"]["details"]["steps"] = json!([fc::email_step_default_fn(
        &json!({ "id": "email-step", "elements": [button] })
    )]);
    node
}

#[test]
fn getnodefrombutton_does_not_expose_page_elements_and_returns_nulls_for_an_unknown_id() {
    let handle_id = "email-handle";
    let mail_node = make_mail_node("mail-node", handle_id);

    assert_eq!(
        get_node_from_button(std::slice::from_ref(&mail_node), handle_id),
        json!({ "button": null, "nodeId": null })
    );
    assert_eq!(
        get_node_from_button(&[mail_node], "missing"),
        json!({ "button": null, "nodeId": null })
    );
}

fn duplicate_node() -> (Value, Value, Value) {
    let carousel_button = make_button("duplicate", Some("Carousel first"));
    let top_level_button = make_button("duplicate", Some("Top level later"));
    let node = make_message_node(
        "owner-node",
        json!({
            "steps": [
                carousel(json!([card(None, "First step", json!([carousel_button]))])),
                text(json!({ "buttons": [top_level_button], "text": "Second step" })),
            ],
        }),
    );
    (node, carousel_button, top_level_button)
}

#[test]
fn getnodefrombutton_preserves_step_order_when_legacy_containers_reuse_a_button_id() {
    let (node, carousel_button, _) = duplicate_node();

    assert_eq!(
        get_node_from_button(&[node], "duplicate")["button"],
        carousel_button
    );
}

#[test]
fn getnodefrombutton_finds_buttons_in_legacy_flow_nodes_without_a_react_flow_type() {
    let button = make_button("legacy-button", None);
    let mut node = make_message_node(
        "legacy-node",
        json!({ "steps": [text(json!({ "buttons": [button], "text": "Legacy" }))] }),
    );
    node.as_object_mut().unwrap().remove("type");

    assert_eq!(
        get_node_from_button(&[node], s(&button, "id")),
        json!({ "button": button, "nodeId": "legacy-node" })
    );
}

#[test]
fn applyrouteinnode_keeps_an_open_mini_app_button_s_action_when_its_edge_is_drawn_or_dropped() {
    let button = spread(
        &make_button("mini-app-button", None),
        json!({ "buttonType": "openMiniApp", "beforeStep": fc::open_mini_app_step_default_fn(&json!({ "miniAppId": "7" })) }),
    );
    let node = make_message_node(
        "owner-node",
        json!({ "steps": [text(json!({ "buttons": [button], "text": "Target" }))] }),
    );

    let connected =
        fc::apply_route_in_node(&node, s(&button, "id"), target("target-node")).unwrap();
    assert_eq!(
        get_node_from_button(std::slice::from_ref(&connected), s(&button, "id"))["button"],
        button
    );

    let disconnected = fc::apply_route_in_node(&connected, s(&button, "id"), None).unwrap();
    assert_eq!(
        get_node_from_button(&[disconnected], s(&button, "id"))["button"],
        button
    );
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteinnode_connects_and_disconnects_a_top_level_button_immutably() {
    let button = make_button("button", None);
    let untouched_step = text(json!({ "text": "Untouched" }));
    let node = make_message_node(
        "owner-node",
        json!({ "steps": [untouched_step, text(json!({ "buttons": [button], "text": "Target" }))] }),
    );
    let original_node = node.clone();

    let connected =
        fc::apply_route_in_node(&node, s(&button, "id"), target("target-node")).expect("connected");
    assert_ne!(connected, node);
    assert_eq!(steps(&connected)[0], untouched_step);
    assert_eq!(node, original_node);

    let connected_button =
        get_node_from_button(std::slice::from_ref(&connected), s(&button, "id"))["button"].clone();
    assert_match_object(
        &connected_button,
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node", "viewOnly": true } }),
    );

    let disconnected = fc::apply_route_in_node(&connected, s(&button, "id"), None).unwrap();
    assert_match_object(
        &get_node_from_button(&[disconnected], s(&button, "id"))["button"],
        &json!({ "buttonType": null, "beforeStep": null }),
    );
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteinnode_updates_a_carousel_button_in_the_s_card_and_preserves_unrelated_references() {
    let positions = ["first", "middle", "last"];
    for position in positions {
        let cards: Vec<Value> = positions
            .iter()
            .map(|p| {
                card(
                    Some(&format!("card-{p}")),
                    p,
                    json!([make_button(&format!("button-{p}"), None)]),
                )
            })
            .collect();
        let node = make_message_node("owner-node", json!({ "steps": [carousel(json!(cards))] }));
        let handle = format!("button-{position}");

        let updated = fc::apply_route_in_node(&node, &handle, target("target-node")).unwrap();
        let updated_cards = steps(&updated)[0]["cards"]
            .as_array()
            .expect("Expected carousel step");

        assert_eq!(updated["id"], "owner-node");
        for (index, original) in cards.iter().enumerate() {
            assert_eq!(
                updated_cards[index] == *original,
                positions[index] != position,
                "{position} card {index}"
            );
        }
        assert_match_object(
            &get_node_from_button(std::slice::from_ref(&updated), &handle)["button"],
            &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
        );
        assert_eq!(
            get_node_from_button(&[node], &handle)["button"],
            make_button(&handle, None)
        );

        let disconnected = fc::apply_route_in_node(&updated, &handle, None).unwrap();
        assert_match_object(
            &get_node_from_button(&[disconnected], &handle)["button"],
            &json!({ "buttonType": null, "beforeStep": null }),
        );
    }
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteinnode_connects_and_disconnects_a_page_button_while_keeping_runtime_lookup_disabled() {
    let node = make_mail_node("mail-node", "email-handle");
    let connected = fc::apply_route_in_node(&node, "email-handle", target("new-target")).unwrap();
    let connected_button = mail_elements(&connected)[0].clone();
    assert_match_object(
        &connected_button,
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "new-target", "viewOnly": true } }),
    );

    assert_ne!(connected_button["beforeStep"]["id"], "email-handle");

    let disconnected = fc::apply_route_in_node(&connected, "email-handle", None).unwrap();
    let disconnected_button = mail_elements(&disconnected)[0].clone();
    assert_match_object(
        &disconnected_button,
        &json!({ "buttonType": null, "beforeStep": null }),
    );
    assert_ne!(mail_elements(&node)[0], disconnected_button);
    assert_eq!(
        get_node_from_button(&[disconnected], "email-handle"),
        json!({ "button": null, "nodeId": null })
    );
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteinnode_does_not_update_whatsapp_options_unknown_handles_or_another_owner_node() {
    let option_node = make_message_node(
        "option-owner",
        json!({ "steps": [fc::whatsapp_option_list_step_default_fn(&json!({ "options": [{ "id": "shared-id", "title": "Option" }] }))] }),
    );
    let button = make_button("shared-id", None);
    let button_node = make_message_node(
        "button-owner",
        json!({ "steps": [text(json!({ "buttons": [button], "text": "Hello" }))] }),
    );
    let option_before = option_node.clone();

    assert!(fc::apply_route_in_node(&option_node, "shared-id", target("target-node")).is_none());
    assert!(fc::apply_route_in_node(&button_node, "missing", None).is_none());

    let updated = fc::apply_route_in_node(&button_node, "shared-id", target("target-node"));
    assert_eq!(updated.unwrap()["id"], "button-owner");
    assert_eq!(option_node, option_before);
}

#[test]
fn applyrouteinnode_keeps_the_canvas_owner_id_when_a_step_has_a_runtime_nodeid() {
    let button = make_button("button", None);
    let node = make_message_node(
        "owner-node",
        json!({ "steps": [text(json!({ "buttons": [button], "nodeId": "runtime-node", "text": "Hello" }))] }),
    );

    let updated = fc::apply_route_in_node(&node, s(&button, "id"), target("target-node")).unwrap();

    assert_eq!(updated["id"], "owner-node");
    assert_eq!(
        get_node_from_button(&[updated], s(&button, "id"))["nodeId"],
        "runtime-node"
    );
}

#[test]
fn applyrouteinnode_updates_the_same_earliest_step_selected_by_runtime_lookup_for_a_duplicate_legacy_id()
 {
    let (node, _, top_level_button) = duplicate_node();

    let updated = fc::apply_route_in_node(&node, "duplicate", target("target-node")).unwrap();
    let steps = steps(&updated);

    assert_match_object(
        &steps[0]["cards"][0]["buttons"][0],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
    );
    assert_eq!(steps[1]["buttons"][0], top_level_button);
    assert_eq!(
        get_node_from_button(std::slice::from_ref(&updated), "duplicate")["button"],
        steps[0]["cards"][0]["buttons"][0]
    );
}

#[test]
fn applyrouteinnode_routes_buttons_in_a_legacy_node_persisted_without_quickreplies() {
    let top_level_button = make_button("legacy-top", Some("Top level"));
    let card_button = make_button("legacy-card", Some("Card"));
    let node = strip_quick_replies(&make_message_node(
        "legacy-node",
        json!({
            "steps": [
                text(json!({ "buttons": [top_level_button], "text": "Hello" })),
                carousel(json!([card(Some("legacy-card-container"), "Card", json!([card_button]))])),
            ],
        }),
    ));

    assert!(node["data"]["details"].get("quickReplies").is_none());
    assert_eq!(
        get_node_from_button(std::slice::from_ref(&node), s(&card_button, "id"))["nodeId"],
        "legacy-node"
    );

    let connected =
        fc::apply_route_in_node(&node, s(&card_button, "id"), target("target-node")).unwrap();
    assert_match_object(
        &steps(&connected)[1]["cards"][0]["buttons"][0],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
    );

    let top_level_connected =
        fc::apply_route_in_node(&node, s(&top_level_button, "id"), target("target-node")).unwrap();
    assert_match_object(
        &steps(&top_level_connected)[0]["buttons"][0],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
    );

    let disconnected = fc::apply_route_in_node(&connected, s(&card_button, "id"), None).unwrap();
    assert_match_object(
        &steps(&disconnected)[1]["cards"][0]["buttons"][0],
        &json!({ "buttonType": null, "beforeStep": null }),
    );
}

#[test]
fn applyrouteinnode_routes_past_a_legacy_card_persisted_without_a_buttons_array() {
    let card_button = make_button("second-card-button", Some("Card"));
    let mut subtitle_card = card(Some("subtitle-only-card"), "Subtitle only", json!([]));
    subtitle_card["subtitle"] = json!("No buttons key at all");
    let node = make_message_node(
        "legacy-cards-node",
        json!({
            "steps": [carousel(json!([strip_card_buttons(subtitle_card), card(Some("card-with-button"), "Card", json!([card_button]))]))],
        }),
    );

    assert!(steps(&node)[0]["cards"][0].get("buttons").is_none());
    assert_eq!(
        get_node_from_button(std::slice::from_ref(&node), s(&card_button, "id"))["nodeId"],
        "legacy-cards-node"
    );

    let connected =
        fc::apply_route_in_node(&node, s(&card_button, "id"), target("target-node")).unwrap();
    assert_match_object(
        &steps(&connected)[0]["cards"][1]["buttons"][0],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
    );
}

fn update(source: &str, handle: &str, route: Option<&str>) -> FlowRouteUpdate {
    FlowRouteUpdate {
        source_node_id: source.to_owned(),
        handle_id: handle.to_owned(),
        route: route.map(str::to_owned),
    }
}

#[test]
fn applyrouteupdatesinnodes_applies_multiple_route_changes_to_the_same_source_node_atomically() {
    let first_button = make_button("first", None);
    let second_button = make_button("second", None);
    let source_node = make_message_node(
        "source-node",
        json!({
            "steps": [
                text(json!({ "buttons": [first_button], "text": "Top-level button" })),
                carousel(json!([card(None, "Carousel button", json!([second_button]))])),
            ],
        }),
    );
    let connected_node = fc::apply_route_updates_in_nodes(
        &[source_node],
        &[
            update("source-node", "first", Some("first-target")),
            update("source-node", "second", Some("second-target")),
        ],
    )[0]
    .clone();

    let disconnected_node = fc::apply_route_updates_in_nodes(
        &[connected_node],
        &[
            update("source-node", "first", None),
            update("source-node", "second", None),
        ],
    )[0]
    .clone();

    for id in ["first", "second"] {
        assert_match_object(
            &get_node_from_button(std::slice::from_ref(&disconnected_node), id)["button"],
            &json!({ "buttonType": null, "beforeStep": null }),
        );
    }
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteupdatesinnodes_scopes_duplicate_handle_ids_to_their_declared_source_node() {
    let first_node = make_message_node(
        "first-node",
        json!({ "steps": [text(json!({ "buttons": [make_button("duplicate", None)], "text": "First" }))] }),
    );
    let second_node = make_message_node(
        "second-node",
        json!({ "steps": [text(json!({ "buttons": [make_button("duplicate", None)], "text": "Second" }))] }),
    );

    let result = fc::apply_route_updates_in_nodes(
        &[first_node.clone(), second_node],
        &[update("second-node", "duplicate", Some("target-node"))],
    );

    assert_eq!(result[0], first_node);
    assert_match_object(
        &get_node_from_button(&[result[0].clone()], "duplicate")["button"],
        &json!({ "buttonType": null, "beforeStep": null }),
    );
    assert_match_object(
        &get_node_from_button(&[result[1].clone()], "duplicate")["button"],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "target-node" } }),
    );
}

// adapted: object identity (toBe / ===) is checked as value equality
#[test]
fn applyrouteupdatesinnodes_preserves_node_and_array_references_when_no_update_applies() {
    let node = make_message_node(
        "source-node",
        json!({ "steps": [text(json!({ "text": "No button" }))] }),
    );
    let nodes = vec![node.clone()];

    let result =
        fc::apply_route_updates_in_nodes(&nodes, &[update("source-node", "missing", None)]);

    assert_eq!(result, nodes);
    assert_eq!(result[0], node);
}

fn settings_node() -> (Value, Value) {
    let mut node = fc::send_message_node_default_fn(
        &json!({ "nodeProps": { "id": "node-1", "position": { "x": 0, "y": 0 } } }),
    );
    let mut settings = fc::quick_reply_settings_default_fn();
    settings["followUp"]["enabled"] = json!(true);
    node["data"]["details"]["quickReplySettings"] = settings.clone();
    (node, settings)
}

#[test]
fn quick_reply_settings_handles_connecting_an_edge_sets_a_startanothernode_target() {
    let (node, settings) = settings_node();
    let updated =
        fc::apply_route_in_node(&node, s(&settings["followUp"], "id"), target("node-9")).unwrap();
    let details = &updated["data"]["details"];
    assert_match_object(
        &details["quickReplySettings"]["followUp"]["target"],
        &json!({ "buttonType": "startAnotherNode", "beforeStep": { "nodeId": "node-9", "viewOnly": true } }),
    );
    assert_eq!(
        details["quickReplySettings"]["followUp"]["id"],
        settings["followUp"]["id"]
    );
    assert_eq!(details["quickReplySettings"]["retry"], settings["retry"]);
}

#[test]
fn quick_reply_settings_handles_deleting_the_edge_clears_the_target_but_keeps_the_handle_id() {
    let (node, settings) = settings_node();
    let connected =
        fc::apply_route_in_node(&node, s(&settings["retry"], "id"), target("node-9")).unwrap();
    let cleared = fc::apply_route_in_node(&connected, s(&settings["retry"], "id"), None).unwrap();
    let details = &cleared["data"]["details"];
    assert!(details["quickReplySettings"]["retry"]["target"].is_null());
    assert_eq!(
        details["quickReplySettings"]["retry"]["id"],
        settings["retry"]["id"]
    );
}

#[test]
fn quick_reply_settings_handles_an_unrelated_handle_id_leaves_the_node_untouched() {
    let (node, _) = settings_node();
    assert!(fc::apply_route_in_node(&node, "nope", target("x")).is_none());
}
