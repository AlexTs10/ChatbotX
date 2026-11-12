//! Port of `packages/flow-config/__tests__/authoring/compile.test.ts`.

mod flow_config_authoring_support;
mod flow_config_support;
use flow_config_authoring_support::*;
use flow_config_support::{Value, assert_match_object, json, spread};

fn empty_ctx() -> FlowAuthoringContext {
    FlowAuthoringContext::default()
}

fn ctx() -> FlowAuthoringContext {
    FlowAuthoringContext {
        templates_by_name: map_of(vec![("welcome_promo", template("1001", "en", "APPROVED"))]),
        custom_fields_by_name: map_of(vec![("Plan", custom_field("1002", "text"))]),
        flows_by_name: map_of(vec![("Nurture", flow_ref("1003"))]),
        pipelines_by_reference_name: None,
    }
}

fn spec(steps: Value) -> Value {
    json!({ "formatVersion": 1, "name": "Test flow", "steps": steps })
}

fn spec_with(steps: Value, overrides: Value) -> Value {
    spread(&spec(steps), overrides)
}

fn first_step(node: &Value) -> &Value {
    &node["data"]["details"]["steps"][0]
}

fn edge_target<'a>(
    compiled: &'a CompiledFlow,
    source: &Value,
    source_handle: &Value,
) -> Option<&'a Value> {
    find_edge(&compiled.edges, |e| {
        &e["source"] == source && &e["sourceHandle"] == source_handle
    })
    .map(|e| &e["target"])
}

// --- compileFlowSpec — one node per step type ---

#[test]
fn compileflowspec_one_node_per_step_type_send_text() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "send", "text": "Hello!" }])),
        &empty_ctx(),
    );
    assert_eq!(compiled.nodes.len(), 1);
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "sendMessage");
    assert_eq!(node["id"], compiled.start_node_id.as_str());
    assert_eq!(node["data"]["isStartNode"], true);
    assert_match_object(
        first_step(node),
        &json!({ "stepType": "sendText", "text": "Hello!" }),
    );
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_send_image() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "send", "imageUrl": "https://example.com/a.png" }])),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    if node["type"] == "sendMessage" {
        assert_match_object(
            first_step(node),
            &json!({ "stepType": "sendImage", "url": "https://example.com/a.png" }),
        );
    }
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_send_file() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "send", "fileUrl": "https://example.com/a.pdf" }])),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    if node["type"] == "sendMessage" {
        assert_match_object(
            first_step(node),
            &json!({ "stepType": "sendFile", "url": "https://example.com/a.pdf" }),
        );
    }
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendviewcatalog_builds_the_catalog_step_on_the_flow_channel()
 {
    let compiled = compile_ok(
        &spec_with(
            json!([{
                "type": "sendViewCatalog",
                "text": "Browse our catalog",
                "footer": "Free shipping",
                "thumbnailProductId": "sku-1",
            }]),
            json!({ "channel": "whatsapp" }),
        ),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "sendMessage");
    assert_eq!(node["data"]["details"]["beforeStep"]["channel"], "whatsapp");
    assert_match_object(
        first_step(node),
        &json!({
            "stepType": "sendViewCatalog",
            "text": "Browse our catalog",
            "footer": "Free shipping",
            "thumbnailProductId": "sku-1",
        }),
    );
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendviewcatalog_defaults_footer_and_thumbnail_to_empty_and_can_be_followed_by_more_steps()
 {
    let compiled = compile_ok(
        &spec_with(
            json!([
                { "type": "sendViewCatalog", "text": "Browse our catalog" },
                { "type": "send", "text": "Anything else?" },
            ]),
            json!({ "channel": "whatsapp" }),
        ),
        &empty_ctx(),
    );
    assert_eq!(compiled.nodes.len(), 2);
    let node = &compiled.nodes[0];
    if node["type"] == "sendMessage" {
        assert_match_object(
            first_step(node),
            &json!({ "footer": "", "thumbnailProductId": "" }),
        );
    }
    assert_eq!(compiled.edges.len(), 1);
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendviewcatalog_is_rejected_at_publish_on_a_channel_that_cannot_deliver_it()
 {
    let compiled = compile_ok(
        &spec_with(
            json!([{ "type": "sendViewCatalog", "text": "Browse our catalog" }]),
            json!({ "channel": "messenger" }),
        ),
        &empty_ctx(),
    );

    assert!(!parse_publish_nodes(&compiled.nodes).success);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendtemplate_resolves_the_template_by_name() {
    let compiled = compile_ok(
        &spec_with(
            json!([{ "type": "sendTemplate", "templateName": "welcome_promo" }]),
            json!({ "channel": "messenger" }),
        ),
        &ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "sendMessage");
    assert_eq!(node["data"]["details"]["beforeStep"]["channel"], "whatsapp");
    assert_match_object(
        first_step(node),
        &json!({
            "stepType": "sendWaTemplateMessage",
            "template": { "id": "1001", "name": "welcome_promo", "language": "en" },
        }),
    );
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendproducts_compiles_to_a_publishable_whatsapp_product_list()
 {
    let compiled = compile_ok(
        &spec_with(
            json!([{
                "type": "sendProducts",
                "display": "products",
                "header": "Our picks",
                "body": "Have a look",
                "sections": [
                    { "title": "Shoes", "productIds": ["11", "12"] },
                    { "title": "Bags", "productIds": ["13"] },
                ],
            }]),
            json!({ "channel": "whatsapp" }),
        ),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "sendMessage");
    assert_eq!(node["data"]["details"]["beforeStep"]["channel"], "whatsapp");
    assert_match_object(
        first_step(node),
        &json!({
            "stepType": "sendProducts",
            "display": "products",
            "sections": [
                { "title": "Shoes", "productIds": ["11", "12"] },
                { "title": "Bags", "productIds": ["13"] },
            ],
        }),
    );
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendproducts_carousel_compiles_for_whatsapp() {
    let compiled = compile_ok(
        &spec_with(
            json!([{ "type": "sendProducts", "display": "carousel", "body": "Look", "sections": [{ "productIds": ["11", "12"] }] }]),
            json!({ "channel": "whatsapp" }),
        ),
        &empty_ctx(),
    );
    assert_eq!(compiled.nodes.len(), 1);
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_sendproducts_is_rejected_at_publish_on_a_channel_without_product_messages()
 {
    let compiled = compile_ok(
        &spec_with(
            json!([{ "type": "sendProducts", "display": "carousel", "body": "Look", "sections": [{ "productIds": ["11", "12"] }] }]),
            json!({ "channel": "messenger" }),
        ),
        &empty_ctx(),
    );
    assert!(!parse_publish_nodes(&compiled.nodes).success);
}

#[test]
fn compileflowspec_one_node_per_step_type_send_uses_the_flow_channel_or_defaults_to_omnichannel() {
    let whatsapp = compile_ok(
        &spec_with(
            json!([{ "type": "send", "text": "Hello!" }]),
            json!({ "channel": "whatsapp" }),
        ),
        &empty_ctx(),
    );
    let omnichannel = compile_ok(
        &spec(json!([{ "type": "send", "text": "Hello!" }])),
        &empty_ctx(),
    );

    assert_eq!(
        whatsapp.nodes[0]["data"]["details"]["beforeStep"]["channel"],
        "whatsapp"
    );
    assert_eq!(
        omnichannel.nodes[0]["data"]["details"]["beforeStep"]["channel"],
        "omnichannel"
    );
}

#[test]
fn compileflowspec_one_node_per_step_type_sendtemplate_reports_an_unknown_template_with_candidates()
{
    let errors = compile_errors(
        &spec(json!([{ "type": "sendTemplate", "templateName": "welcom_promo" }])),
        &ctx(),
    );
    let error = &errors[0];
    assert_eq!(error.path, "steps[0].templateName");
    assert_eq!(error.code, "unknownTemplate");
    assert!(
        error
            .candidates
            .as_ref()
            .unwrap()
            .contains(&"welcome_promo".to_string())
    );
}

#[test]
fn compileflowspec_one_node_per_step_type_wait() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "wait", "duration": 2, "unit": "hours" }])),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "wait");
    assert_match_object(first_step(node), &json!({ "duration": 2, "unit": "hours" }));
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_action_s_compiles_to_a_performaction_node() {
    for (action, extra, expected_step_type) in [
        ("addTags", json!({ "tagNames": ["VIP"] }), "addContactTag"),
        (
            "removeTags",
            json!({ "tagNames": ["VIP"] }),
            "removeContactTag",
        ),
        (
            "setCustomField",
            json!({ "customFieldName": "Plan", "value": "premium" }),
            "setCustomField",
        ),
        (
            "assignConversation",
            json!({ "assigneeId": "user-1" }),
            "assignConversation",
        ),
        ("archiveConversation", json!({}), "archiveConversation"),
    ] {
        let compiled = compile_ok(
            &spec(json!([spread(
                &json!({ "type": "action", "action": action }),
                extra
            )])),
            &ctx(),
        );
        let node = &compiled.nodes[0];
        assert_eq!(node["type"], "performAction", "{action}");
        let compiled_step = first_step(node);
        assert_eq!(compiled_step["stepType"], expected_step_type, "{action}");
        if action == "setCustomField" {
            assert_eq!(compiled_step["inputFieldId"], "1002");
        }
        expect_publishable(&compiled.nodes, &compiled.edges);
    }
}

#[test]
fn compileflowspec_one_node_per_step_type_action_setcustomfield_reports_an_unknown_custom_field_with_candidates()
 {
    let errors = compile_errors(
        &spec(
            json!([{ "type": "action", "action": "setCustomField", "customFieldName": "Pln", "value": "premium" }]),
        ),
        &ctx(),
    );
    let error = &errors[0];
    assert_eq!(error.path, "steps[0].customFieldName");
    assert_eq!(error.code, "unknownCustomField");
    assert!(
        error
            .candidates
            .as_ref()
            .unwrap()
            .contains(&"Plan".to_string())
    );
}

#[test]
fn compileflowspec_one_node_per_step_type_startflow_resolves_the_target_flow_by_name() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "startFlow", "flowName": "Nurture" }])),
        &ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "startFlow");
    assert_eq!(node["data"]["details"]["beforeStep"]["flowId"], "1003");
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_startflow_reports_an_unknown_flow_with_candidates() {
    let errors = compile_errors(
        &spec(json!([{ "type": "startFlow", "flowName": "Nurtur" }])),
        &ctx(),
    );
    let error = &errors[0];
    assert_eq!(error.path, "steps[0].flowName");
    assert_eq!(error.code, "unknownFlow");
    assert!(
        error
            .candidates
            .as_ref()
            .unwrap()
            .contains(&"Nurture".to_string())
    );
}

#[test]
fn compileflowspec_one_node_per_step_type_addnote() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "addNote", "note": "Called back" }])),
        &empty_ctx(),
    );
    let node = &compiled.nodes[0];
    assert_eq!(node["type"], "addNotes");
    assert_eq!(node["data"]["details"]["beforeStep"]["text"], "Called back");
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_branch_compiles_to_a_condition_node_with_cases_and_otherwise()
 {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "branch",
            "cases": [{
                "when": [{ "field": "email", "operator": "isNotEmpty" }],
                "then": [{ "type": "addNote", "note": "has email" }],
            }],
            "otherwise": [{ "type": "addNote", "note": "no email" }],
        }])),
        &empty_ctx(),
    );
    assert_eq!(compiled.nodes.len(), 3);
    let branch_node = &compiled.nodes[0];
    assert_eq!(branch_node["type"], "condition");
    let condition_step = first_step(branch_node);
    assert_eq!(condition_step["cases"].as_array().unwrap().len(), 1);
    assert_match_object(
        &condition_step["cases"][0]["conditions"][0],
        &json!({ "field": "email", "operator": "isNotEmpty" }),
    );
    assert!(
        find_edge(&compiled.edges, |e| e["sourceHandle"]
            == condition_step["cases"][0]["id"])
        .is_some()
    );
    assert!(
        find_edge(&compiled.edges, |e| e["sourceHandle"]
            == condition_step["otherwiseId"])
        .is_some()
    );
    expect_publishable(&compiled.nodes, &compiled.edges);
}

#[test]
fn compileflowspec_one_node_per_step_type_rejects_bot_fields_in_branch_conditions() {
    let errors = compile_errors(
        &spec(json!([{
            "type": "branch",
            "cases": [{
                "when": [{ "field": "botField:email", "operator": "isNotEmpty" }],
                "then": [{ "type": "addNote", "note": "has email" }],
            }],
        }])),
        &empty_ctx(),
    );
    assert_eq!(errors[0].code, "invalidSpec");
    assert_eq!(errors[0].path, "steps[0].cases[0].when[0].field");
}

// --- compileFlowSpec — reference resolution ---

#[test]
fn compileflowspec_reference_resolution_accumulates_invalid_names_in_spec_order() {
    let errors = compile_errors(
        &spec(json!([
            { "type": "sendTemplate", "templateName": "missing template" },
            { "type": "action", "action": "setCustomField", "customFieldName": "missing field", "value": "value" },
            { "type": "startFlow", "flowName": "missing flow" },
        ])),
        &empty_ctx(),
    );
    let pairs: Vec<(String, String)> = errors.into_iter().map(|e| (e.code, e.path)).collect();
    assert_eq!(
        pairs,
        vec![
            (
                "unknownTemplate".to_string(),
                "steps[0].templateName".to_string()
            ),
            (
                "unknownCustomField".to_string(),
                "steps[1].customFieldName".to_string()
            ),
            ("unknownFlow".to_string(), "steps[2].flowName".to_string()),
        ]
    );
}

#[test]
fn compileflowspec_reference_resolution_rejects_a_resolved_template_that_is_not_approved() {
    let pending_template_ctx = FlowAuthoringContext {
        templates_by_name: map_of(vec![("pending", template("1004", "en", "PENDING"))]),
        ..empty_ctx()
    };

    let errors = compile_errors(
        &spec(json!([{ "type": "sendTemplate", "templateName": "pending" }])),
        &pending_template_ctx,
    );
    let error = &errors[0];
    assert_eq!(error.code, "templateNotApproved");
    assert_eq!(error.path, "steps[0].templateName");
    assert_eq!(
        error.hint.as_deref(),
        Some("Pick a template whose status is APPROVED in capabilities.get's templates list.")
    );
}

// --- compileFlowSpec — a realistic multi-step flow ---

#[test]
fn compileflowspec_a_realistic_multi_step_flow_send_wait_1h_branch_sendtemplate() {
    let compiled = compile_ok(
        &spec_with(
            json!([
                { "type": "send", "text": "Hi! Thanks for reaching out." },
                { "type": "wait", "duration": 1, "unit": "hours" },
                {
                    "type": "branch",
                    "cases": [{
                        "when": [{ "field": "country", "operator": "eq", "value": "US" }],
                        "then": [{ "type": "sendTemplate", "templateName": "welcome_promo" }],
                    }],
                },
            ]),
            json!({ "channel": "whatsapp" }),
        ),
        &ctx(),
    );

    let types: Vec<&str> = compiled
        .nodes
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        vec!["sendMessage", "wait", "condition", "sendMessage"]
    );
    // send -> wait -> branch chained by Continue edges (source === sourceHandle === node id).
    let (send_node, wait_node, branch_node) =
        (&compiled.nodes[0], &compiled.nodes[1], &compiled.nodes[2]);
    assert_eq!(
        edge_target(&compiled, &send_node["id"], &send_node["id"]),
        Some(&wait_node["id"])
    );
    assert_eq!(
        edge_target(&compiled, &wait_node["id"], &wait_node["id"]),
        Some(&branch_node["id"])
    );

    expect_publishable(&compiled.nodes, &compiled.edges);
}

// --- compileFlowSpec — button routing ---

#[test]
fn compileflowspec_button_routing_a_call_button_compiles_to_a_callphonenumber_button_with_no_route_or_edge()
 {
    let compiled = compile_ok(
        &spec(
            json!([{ "type": "send", "text": "Need help?", "buttons": [{ "text": "Call us", "call": "+14155552671" }] }]),
        ),
        &empty_ctx(),
    );

    let send_node = &compiled.nodes[0];
    assert_eq!(
        send_node["type"], "sendMessage",
        "expected sendMessage node"
    );
    let button = &first_step(send_node)["buttons"][0];
    assert_match_object(
        button,
        &json!({
            "label": "Call us",
            "buttonType": "callPhoneNumber",
            "beforeStep": { "stepType": "callPhoneNumber", "phoneNumber": "+14155552671" },
            "steps": [],
        }),
    );
    assert!(compiled.edges.is_empty());
}

#[test]
fn compileflowspec_button_routing_a_copy_button_compiles_to_a_copytext_button_with_no_route_or_edge()
 {
    let compiled = compile_ok(
        &spec(
            json!([{ "type": "send", "text": "Pague com PIX", "buttons": [{ "text": "Copiar PIX", "copy": "{{pix_code}}" }] }]),
        ),
        &empty_ctx(),
    );

    let send_node = &compiled.nodes[0];
    assert_eq!(
        send_node["type"], "sendMessage",
        "expected sendMessage node"
    );
    let button = &first_step(send_node)["buttons"][0];
    assert_match_object(
        button,
        &json!({
            "label": "Copiar PIX",
            "buttonType": "copyText",
            "beforeStep": { "stepType": "copyText", "text": "{{pix_code}}" },
            "steps": [],
        }),
    );
    assert!(compiled.edges.is_empty());
}

#[test]
fn compileflowspec_button_routing_a_routed_button_writes_both_the_node_s_beforestep_and_a_matching_edge()
 {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "send",
            "text": "Want a discount?",
            "buttons": [
                { "text": "Yes", "then": [{ "type": "addNote", "note": "said yes" }] },
                { "text": "No" },
            ],
        }])),
        &empty_ctx(),
    );

    let send_node = &compiled.nodes[0];
    assert_eq!(send_node["type"], "sendMessage");
    let buttons = &first_step(send_node)["buttons"];
    let (yes_button, no_button) = (&buttons[0], &buttons[1]);
    let note_node = &compiled.nodes[1];

    // Routed button: beforeStep points at the new node...
    assert_eq!(yes_button["buttonType"], "startAnotherNode");
    assert_eq!(yes_button["beforeStep"]["nodeId"], note_node["id"]);
    // ...and an edge exists for the same handle, to the same target.
    let edge = find_edge(&compiled.edges, |e| e["sourceHandle"] == yes_button["id"]).expect("edge");
    assert_match_object(
        edge,
        &json!({ "source": send_node["id"], "target": note_node["id"], "targetHandle": note_node["id"] }),
    );

    // Unrouted button stays inert — no beforeStep, no edge.
    assert!(no_button["buttonType"].is_null());
    assert!(find_edge(&compiled.edges, |e| e["sourceHandle"] == no_button["id"]).is_none());

    expect_publishable(&compiled.nodes, &compiled.edges);
}

// --- compileFlowSpec — goto ---

#[test]
fn compileflowspec_goto_jumps_to_an_earlier_step_s_node_instead_of_creating_a_new_one() {
    let compiled = compile_ok(
        &spec(json!([
            { "type": "addNote", "id": "greet", "note": "greeted" },
            { "type": "wait", "duration": 1, "unit": "days" },
            { "type": "goto", "targetId": "greet" },
        ])),
        &empty_ctx(),
    );

    // "goto" creates no node of its own.
    assert_eq!(compiled.nodes.len(), 2);
    let (greet_node, wait_node) = (&compiled.nodes[0], &compiled.nodes[1]);
    assert_eq!(
        edge_target(&compiled, &wait_node["id"], &wait_node["id"]),
        Some(&greet_node["id"])
    );
    for node in &compiled.nodes {
        assert!(node["position"]["x"].is_number());
        assert!(node["position"]["y"].is_number());
    }
}

#[test]
fn compileflowspec_goto_wires_a_nested_chain_opening_with_goto_directly_to_its_target() {
    let compiled = compile_ok(
        &spec(json!([
            { "type": "addNote", "id": "target", "note": "target" },
            {
                "type": "branch",
                "cases": [{
                    "when": [{ "field": "email", "operator": "isNotEmpty" }],
                    "then": [{ "type": "goto", "targetId": "target" }],
                }],
            },
        ])),
        &empty_ctx(),
    );

    assert_eq!(compiled.nodes.len(), 2);
    let (target_node, branch_node) = (&compiled.nodes[0], &compiled.nodes[1]);
    assert_eq!(branch_node["type"], "condition", "expected condition node");
    let case_id = &first_step(branch_node)["cases"][0]["id"];
    assert_eq!(
        find_edge(&compiled.edges, |e| &e["sourceHandle"] == case_id).map(|e| &e["target"]),
        Some(&target_node["id"])
    );
}

#[test]
fn compileflowspec_goto_rejects_goto_as_the_first_step() {
    let errors = compile_errors(
        &spec(json!([{ "type": "goto", "targetId": "x" }])),
        &empty_ctx(),
    );
    assert_eq!(errors[0].code, "invalidFirstStep");
    assert_eq!(errors[0].path, "steps[0]");
}

#[test]
fn compileflowspec_goto_rejects_a_goto_to_an_unknown_step_id() {
    let errors = compile_errors(
        &spec(
            json!([{ "type": "addNote", "note": "hi" }, { "type": "goto", "targetId": "does-not-exist" }]),
        ),
        &empty_ctx(),
    );
    assert_eq!(errors[0].code, "invalidGotoTarget");
    assert_eq!(errors[0].path, "steps[1].targetId");
}

#[test]
fn compileflowspec_goto_rejects_a_goto_that_targets_the_immediately_preceding_step_self_loop() {
    let errors = compile_errors(
        &spec(
            json!([{ "type": "addNote", "id": "note", "note": "hi" }, { "type": "goto", "targetId": "note" }]),
        ),
        &empty_ctx(),
    );
    assert_eq!(errors[0].code, "selfLoopGoto");
    assert_eq!(errors[0].path, "steps[1]");
}

#[test]
fn compileflowspec_goto_a_goto_jumping_back_over_an_earlier_step_not_the_immediately_preceding_one_still_compiles()
 {
    // Distinct from the self-loop case above: previousNodeId !== target here.
    let compiled = compile_ok(
        &spec(json!([
            { "type": "addNote", "id": "greet", "note": "greeted" },
            { "type": "wait", "duration": 1, "unit": "days" },
            { "type": "goto", "targetId": "greet" },
        ])),
        &empty_ctx(),
    );
    assert_eq!(compiled.nodes.len(), 2);
}

#[test]
fn compileflowspec_goto_a_button_s_goto_back_to_its_own_send_step_compiles_id_registered_before_children()
 {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "send",
            "id": "menu",
            "text": "Pick an option",
            "buttons": [{ "text": "Back to menu", "then": [{ "type": "goto", "targetId": "menu" }] }],
        }])),
        &empty_ctx(),
    );

    assert_eq!(compiled.nodes.len(), 1);
    let menu_node = &compiled.nodes[0];
    assert!(
        find_edge(&compiled.edges, |e| e["target"] == menu_node["id"]
            && e["source"] == menu_node["id"])
        .is_some()
    );
}

#[test]
fn compileflowspec_goto_a_branch_case_s_goto_back_to_its_own_branch_step_compiles_id_registered_before_children()
 {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "branch",
            "id": "router",
            "cases": [{
                "when": [{ "field": "email", "operator": "isNotEmpty" }],
                "then": [{ "type": "goto", "targetId": "router" }],
            }],
        }])),
        &empty_ctx(),
    );

    assert_eq!(compiled.nodes.len(), 1);
    let branch_node = &compiled.nodes[0];
    assert_eq!(branch_node["type"], "condition", "expected condition node");
    let case_id = &first_step(branch_node)["cases"][0]["id"];
    assert_eq!(
        find_edge(&compiled.edges, |e| &e["sourceHandle"] == case_id).map(|e| &e["target"]),
        Some(&branch_node["id"])
    );
}

// --- compileFlowSpec — structural validation ---

#[test]
fn compileflowspec_structural_validation_reports_an_unreachable_step_after_a_terminal_branch() {
    let errors = compile_errors(
        &spec(json!([
            {
                "type": "branch",
                "cases": [{
                    "when": [{ "field": "email", "operator": "isNotEmpty" }],
                    "then": [{ "type": "addNote", "note": "x" }],
                }],
            },
            { "type": "addNote", "note": "unreachable" },
        ])),
        &empty_ctx(),
    );
    assert_eq!(errors[0].code, "unreachableStep");
    assert_eq!(errors[0].path, "steps[1]");
}

#[test]
fn compileflowspec_structural_validation_reports_duplicate_step_ids_anywhere_in_the_spec() {
    let errors = compile_errors(
        &spec(json!([
            { "type": "addNote", "id": "dup", "note": "a" },
            { "type": "addNote", "id": "dup", "note": "b" },
        ])),
        &empty_ctx(),
    );
    let codes: Vec<&str> = errors.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, vec!["duplicateStepId", "duplicateStepId"]);
}

#[test]
fn compileflowspec_structural_validation_reports_duplicate_ids_across_nesting_levels() {
    let errors = compile_errors(
        &spec(json!([{
            "type": "branch",
            "id": "dup",
            "cases": [{
                "when": [{ "field": "email", "operator": "isNotEmpty" }],
                "then": [{ "type": "addNote", "id": "dup", "note": "nested" }],
            }],
        }])),
        &empty_ctx(),
    );
    let pairs: Vec<(String, String)> = errors.into_iter().map(|e| (e.code, e.path)).collect();
    assert_eq!(
        pairs,
        vec![
            ("duplicateStepId".to_string(), "steps[0]".to_string()),
            (
                "duplicateStepId".to_string(),
                "steps[0].cases[0].then[0]".to_string()
            ),
        ]
    );
}

#[test]
fn compileflowspec_structural_validation_guards_the_compiler_invariant_when_an_unparsed_spec_has_no_steps()
 {
    // `FlowSpec` cannot encode flowSpecSchema's runtime `.min(1)` constraint.
    let errors = compile_errors(&spec(json!([])), &empty_ctx());
    assert_eq!(errors[0].code, "compileFailed");
    assert_eq!(errors[0].path, "steps");
}

// --- compileFlowSpec — layout determinism ---

#[test]
fn compileflowspec_layout_determinism_compiling_the_same_spec_twice_produces_identical_positions() {
    let build_spec = || {
        spec(json!([
            { "type": "send", "text": "Hi" },
            {
                "type": "branch",
                "cases": [{
                    "when": [{ "field": "email", "operator": "isNotEmpty" }],
                    "then": [{ "type": "addNote", "note": "a" }],
                }],
                "otherwise": [{ "type": "addNote", "note": "b" }],
            },
        ]))
    };

    let first = compile_ok(&build_spec(), &empty_ctx());
    let second = compile_ok(&build_spec(), &empty_ctx());

    let positions = |nodes: &[Value]| {
        nodes
            .iter()
            .map(|n| n["position"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&first.nodes), positions(&second.nodes));
}

// --- compileFlowSpec — round-trip through the export schema ---

#[test]
fn compileflowspec_round_trip_through_the_export_schema_a_compiled_graph_parses_as_a_valid_flow_export()
 {
    let compiled = compile_ok(
        &spec(
            json!([{ "type": "send", "text": "Hi!" }, { "type": "wait", "duration": 1, "unit": "hours" }]),
        ),
        &empty_ctx(),
    );

    let result = fc::parse_flow_export(&json!({
        "formatVersion": 2,
        "exportedAt": chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "source": { "workspaceId": "1", "flowId": "1" },
        "flows": [{
            "name": "Test flow",
            "active": true,
            "enableInInbox": false,
            "startNodeId": compiled.start_node_id,
            "nodes": compiled.nodes,
            "edges": compiled.edges,
        }],
        "customFields": {},
        "botFields": {},
    }));

    assert!(result.is_ok(), "{:?}", result.err().map(|e| e.reason));
}

// --- compileFlowSpec — specPathByNodeId ---

#[test]
fn compileflowspec_specpathbynodeid_maps_a_top_level_step_s_node_back_to_its_spec_path() {
    let compiled = compile_ok(
        &spec(json!([{ "type": "addNote", "note": "Called back" }])),
        &empty_ctx(),
    );
    let node_id = compiled.nodes[0]["id"].as_str().unwrap();
    assert_eq!(
        compiled
            .spec_path_by_node_id
            .get(node_id)
            .map(String::as_str),
        Some("steps[0]")
    );
}

#[test]
fn compileflowspec_specpathbynodeid_maps_a_button_s_nested_chain_to_its_buttons_then_path() {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "send",
            "text": "Pick one",
            "buttons": [{ "text": "Yes", "then": [{ "type": "addNote", "note": "said yes" }] }],
        }])),
        &empty_ctx(),
    );
    let nested_node = compiled
        .nodes
        .iter()
        .find(|n| n["type"] == "addNotes")
        .expect("addNotes node");
    assert_eq!(
        compiled
            .spec_path_by_node_id
            .get(nested_node["id"].as_str().unwrap())
            .map(String::as_str),
        Some("steps[0].buttons[0].then[0]")
    );
}

#[test]
fn compileflowspec_specpathbynodeid_maps_a_branch_case_s_nested_chain_to_its_cases_then_path() {
    let compiled = compile_ok(
        &spec(json!([{
            "type": "branch",
            "cases": [{
                "when": [{ "field": "email", "operator": "isNotEmpty" }],
                "then": [{ "type": "addNote", "note": "has email" }],
            }],
        }])),
        &empty_ctx(),
    );
    let branch_node = &compiled.nodes[0];
    let nested_node = compiled
        .nodes
        .iter()
        .find(|n| n["type"] == "addNotes")
        .expect("addNotes node");
    assert_eq!(
        compiled
            .spec_path_by_node_id
            .get(branch_node["id"].as_str().unwrap())
            .map(String::as_str),
        Some("steps[0]")
    );
    assert_eq!(
        compiled
            .spec_path_by_node_id
            .get(nested_node["id"].as_str().unwrap())
            .map(String::as_str),
        Some("steps[0].cases[0].then[0]")
    );
}

// --- compileFlowSpec — setCustomField operations ---

fn typed_ctx() -> FlowAuthoringContext {
    FlowAuthoringContext {
        custom_fields_by_name: map_of(vec![
            ("Points", custom_field("2001", "number")),
            ("Due", custom_field("2002", "datetime")),
            ("Note", custom_field("2003", "shortText")),
        ]),
        ..empty_ctx()
    }
}

fn action_spec(custom_field_name: &str, operation: Option<&str>, value: Option<&str>) -> Value {
    let mut step = json!({ "type": "action", "action": "setCustomField", "customFieldName": custom_field_name });
    if let Some(operation) = operation {
        step["operation"] = json!(operation);
    }
    if let Some(value) = value {
        step["value"] = json!(value);
    }
    spec(json!([step]))
}

fn compiled_operation(compiled: &CompiledFlow) -> Option<String> {
    let node = &compiled.nodes[0];
    (node["type"] == "performAction")
        .then(|| first_step(node)["operation"].as_str().map(str::to_string))
        .flatten()
}

// adapted: an `undefined` operation/value property is an absent key
#[test]
fn compileflowspec_setcustomfield_operations_an_omitted_operation_still_compiles_to_set() {
    let compiled = compile_ok(&action_spec("Points", None, Some("5")), &typed_ctx());
    assert_eq!(compiled_operation(&compiled).as_deref(), Some("O01"));
}

#[test]
fn compileflowspec_setcustomfield_operations_s_with_s_compiles_to_s() {
    for (field, operation, code) in [
        ("Points", "increase", "O04"),
        ("Points", "formula", "O07"),
        ("Due", "addDays", "O12"),
        ("Due", "dateExpression", "O09"),
        ("Note", "append", "O02"),
        ("Note", "clear", "O06"),
    ] {
        let compiled = compile_ok(
            &action_spec(field, Some(operation), Some("1")),
            &typed_ctx(),
        );
        assert_eq!(
            compiled_operation(&compiled).as_deref(),
            Some(code),
            "{field} {operation}"
        );
    }
}

#[test]
fn compileflowspec_setcustomfield_operations_an_operation_the_field_type_does_not_support_is_reported_with_the_supported_ones()
 {
    let errors = compile_errors(
        &action_spec("Points", Some("addDays"), Some("1")),
        &typed_ctx(),
    );
    let error = &errors[0];
    assert_eq!(error.path, "steps[0].operation");
    assert_eq!(error.code, "invalidFieldOperation");
    assert!(
        error
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("increase")
    );
}
