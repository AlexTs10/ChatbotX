//! Port of `packages/mini-app/__tests__/validate-issue-codes.test.ts`.

mod mini_app_fixtures;

use chatbotx_core::mini_app::validate;
use mini_app_fixtures::{children, two_screen_definition};
use serde_json::{Value, json};

type Break = fn(&mut Value);

fn add_to_welcome(d: &mut Value, nodes: Vec<Value>) {
    let list = children(d, 0);
    for (i, node) in nodes.into_iter().enumerate() {
        list.insert(1 + i, node);
    }
}
fn name_input(d: &mut Value) -> &mut Value {
    &mut d["screens"][0]["children"][1]["props"]
}
fn welcome_footer(d: &mut Value) -> &mut Value {
    &mut d["screens"][0]["children"][2]["props"]
}
fn text_body(id: &str, text: &str) -> Value {
    json!({ "id": id, "type": "TextBody", "props": { "text": text } })
}
fn navigation_list(id: &str, items: Vec<Value>) -> Value {
    json!({ "id": id, "type": "NavigationList", "props": { "name": id, "list-items": items } })
}
fn navigation_item(id: &str, extra: Value) -> Value {
    let mut item = json!({ "id": id, "main-content": { "title": id },
        "on-click-action": { "name": "navigate", "next": "s_confirm" } });
    item.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    item
}
fn options(count: usize, extra: Value) -> Vec<Value> {
    (0..count)
        .map(|i| {
            let mut o = json!({ "id": format!("option_{i}"), "title": format!("Option {i}") });
            o.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
            o
        })
        .collect()
}
fn nested_if(depth: usize) -> Value {
    let then = if depth > 1 { nested_if(depth - 1) } else { text_body(&format!("n_leaf_{depth}"), "x") };
    json!({ "id": format!("n_if_{depth}"), "type": "If", "props": { "condition": "true" },
        "slots": { "then": [then], "else": [] } })
}
fn go_footer(id: &str) -> Value {
    json!({ "id": id, "type": "Footer", "props": { "label": "Go",
        "on-click-action": { "name": "navigate", "next": "s_confirm" } } })
}
fn photo() -> Value {
    json!({ "id": "n_photo", "type": "PhotoPicker", "props": { "name": "photo", "label": "Photo" } })
}

const CASES: &[(&str, Break)] = &[
    ("screen_id_invalid", |d| d["screens"][1]["id"] = json!("1_BAD")),
    ("screen_id_reserved", |d| d["screens"][1]["id"] = json!("SUCCESS")),
    ("screen_id_duplicate", |d| d["screens"][1]["id"] = json!("WELCOME")),
    ("no_terminal_screen", |d| {
        children(d, 1).last_mut().unwrap()["props"].as_object_mut().unwrap().remove("on-click-action");
    }),
    ("screen_without_exit", |d| {
        children(d, 0).pop();
    }),
    ("screen_unreachable", |d| {
        children(d, 0).pop();
    }),
    ("too_many_components", |d| {
        add_to_welcome(d, (0..50).map(|i| text_body(&format!("n_body_{i}"), "x")).collect())
    }),
    ("component_limit", |d| {
        add_to_welcome(
            d,
            (1..=4).map(|i| json!({ "id": format!("n_image_{i}"), "type": "Image", "props": { "src": "AAAA" } })).collect(),
        )
    }),
    ("picker_conflict", |d| {
        add_to_welcome(
            d,
            vec![
                photo(),
                json!({ "id": "n_document", "type": "DocumentPicker", "props": { "name": "document", "label": "Document" } }),
            ],
        )
    }),
    ("picker_multiple_in_complete", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_photo", "type": "PhotoPicker",
            "props": { "name": "photo", "label": "Photo", "max-uploaded-photos": 11 } })])
    }),
    ("footer_not_last", |d| children(d, 0).push(text_body("n_late", "x"))),
    ("footer_in_nested_branch", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_switch", "type": "Switch", "props": { "value": "${form.full_name}" },
            "slots": { "case_1": [go_footer("n_switch_footer")] } })])
    }),
    ("footer_missing_in_branch", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_if", "type": "If", "props": { "condition": "true" },
            "slots": { "then": [go_footer("n_if_footer")], "else": [text_body("n_else", "x")] } })])
    }),
    ("navigation_list_with_footer", |d| {
        add_to_welcome(d, vec![navigation_list("menu", vec![navigation_item("a", json!({}))])])
    }),
    ("navigation_list_on_terminal", |d| {
        let item = navigation_item("a", json!({ "on-click-action": { "name": "navigate", "next": "s_welcome" } }));
        children(d, 1).insert(0, navigation_list("menu", vec![item]));
    }),
    ("if_too_deep", |d| add_to_welcome(d, vec![nested_if(4)])),
    ("if_then_empty", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_if", "type": "If", "props": { "condition": "true" },
            "slots": { "then": [], "else": [] } })])
    }),
    ("switch_no_cases", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_switch", "type": "Switch",
            "props": { "value": "${form.full_name}" }, "slots": {} })])
    }),
    ("child_not_allowed", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_if", "type": "If", "props": { "condition": "true" },
            "slots": { "then": [photo()], "else": [] } })])
    }),
    ("name_invalid", |d| name_input(d)["name"] = json!("1_bad")),
    ("name_duplicate", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_name_copy", "type": "TextInput",
            "props": { "name": "full_name", "label": "Again" } })])
    }),
    ("property_required", |d| name_input(d)["label"] = json!("")),
    ("property_too_long", |d| name_input(d)["label"] = json!("This label is far too long for Meta")),
    ("property_too_short", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_chips", "type": "ChipsSelector",
            "props": { "name": "chips", "label": "Chips", "data-source": options(1, json!({})) } })])
    }),
    ("property_invalid", |d| name_input(d)["input-type"] = json!("bogus")),
    ("option_duplicate_id", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_dropdown", "type": "Dropdown", "props": {
            "name": "pick", "label": "Pick", "data-source": [{ "id": "same", "title": "A" }, { "id": "same", "title": "B" }] } })])
    }),
    ("min_greater_than_max", |d| {
        name_input(d)["min-chars"] = json!(10);
        name_input(d)["max-chars"] = json!(5);
    }),
    ("action_required", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_link", "type": "EmbeddedLink", "props": { "text": "Link" } })])
    }),
    ("action_not_allowed", |d| {
        welcome_footer(d)["on-click-action"] = json!({ "name": "open_url", "url": "https://example.com" })
    }),
    ("navigate_target_missing", |d| {
        welcome_footer(d)["on-click-action"] = json!({ "name": "navigate", "next": "s_missing" })
    }),
    ("navigate_self", |d| welcome_footer(d)["on-click-action"] = json!({ "name": "navigate", "next": "s_welcome" })),
    ("expression_invalid", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_if", "type": "If", "props": { "condition": "((" },
            "slots": { "then": [text_body("n_then", "x")], "else": [] } })])
    }),
    ("reference_invalid", |d| add_to_welcome(d, vec![text_body("n_ref", "Hi ${nope}")])),
    ("reference_unknown", |d| add_to_welcome(d, vec![text_body("n_ref", "Hi ${form.nobody}")])),
    ("data_reference_unsupported", |d| add_to_welcome(d, vec![text_body("n_ref", "Hi ${data.x}")])),
    ("property_unknown", |d| name_input(d)["enabled"] = json!(false)),
    ("footer_captions_invalid", |d| welcome_footer(d)["left-caption"] = json!("Left")),
    ("options_with_images_limit", |d| {
        add_to_welcome(d, vec![json!({ "id": "n_dropdown", "type": "Dropdown", "props": {
            "name": "pick", "label": "Pick", "data-source": options(101, json!({ "image": "AAAA" })) } })])
    }),
    ("navigation_badge_multiple", |d| {
        let back = json!({ "name": "navigate", "next": "s_welcome" });
        let items = vec![
            navigation_item("a", json!({ "badge": "New", "on-click-action": back })),
            navigation_item("b", json!({ "badge": "Hot", "on-click-action": back })),
        ];
        children(d, 1).insert(0, navigation_list("menu", items));
    }),
    ("contact_variable_whatsapp", |d| add_to_welcome(d, vec![text_body("n_var", "Hi {{first_name}}")])),
];

#[test]
fn validate_mini_app_issue_codes_reports_definition_invalid_for_a_definition_without_screens() {
    let result = validate(&json!({ "screens": [] }));
    assert!(!result.valid);
    assert_eq!(result.issues.len(), 1);
    assert_eq!((result.issues[0].code, result.issues[0].severity), ("definition_invalid", "error"));
}

#[test]
fn validate_mini_app_issue_codes_reports_each_code() {
    for (code, break_definition) in CASES {
        let mut definition = two_screen_definition();
        break_definition(&mut definition);
        let result = validate(&definition);
        let issue = result.issues.iter().find(|i| i.code == *code);
        let expected = if matches!(*code, "screen_unreachable" | "contact_variable_whatsapp") { "warning" } else { "error" };
        assert_eq!(issue.map(|i| i.severity), Some(expected), "{code}: {:?}", result.issues);
    }
}
