//! Port of `packages/mini-app/__tests__/display-text.test.ts` (the server
//! parts: `mapDisplayText`, `displayTexts`, `escapeContactValue`; the rest is
//! in the test map).

use chatbotx_core::mini_app::{
    Definition, escape_contact_value, map_display_text, parse_definition,
};
use serde_json::json;

fn with_variables() -> Definition {
    parse_definition(&json!({ "screens": [{ "key": "s1", "id": "WELCOME", "title": "Hi {{first_name}}", "children": [
        { "id": "t", "type": "TextInput", "props": { "name": "phone", "label": "SĐT {{first_name}}" } },
        { "id": "d", "type": "Dropdown", "props": { "name": "plan", "label": "Plan",
            "data-source": [{ "id": "a", "title": "Gói {{last_name}}", "description": "x" }] } },
        { "id": "n", "type": "NavigationList", "props": { "name": "nav",
            "list-items": [{ "id": "i", "main-content": { "title": "Go {{city}}" }, "badge": "{{tier}}" }] } },
        { "id": "f", "type": "Footer", "props": { "label": "Send", "on-click-action": { "name": "complete" } } }
    ] }] }))
    .expect("definition")
}

#[test]
fn map_display_text_visits_screen_titles_labels_options_and_navigation_items_but_not_keys() {
    let mut definition = with_variables();
    map_display_text(&mut definition, &mut |text, _| {
        *text = text
            .replace("{{first_name}}", "An")
            .replace("{{last_name}}", "Le")
            .replace("{{city}}", "HN")
            .replace("{{tier}}", "VIP");
    });
    let screen = &definition.screens[0];
    assert_eq!(screen.title, "Hi An");
    assert_eq!(screen.children[0].props["name"], "phone");
    assert_eq!(screen.children[0].props["label"], "SĐT An");
    assert_eq!(
        screen.children[1].props["data-source"],
        json!([{ "id": "a", "title": "Gói Le", "description": "x" }])
    );
    assert_eq!(
        screen.children[2].props["list-items"],
        json!([{ "id": "i", "main-content": { "title": "Go HN" }, "badge": "VIP" }])
    );
}

#[test]
fn display_texts_marks_only_markdown_texts_as_markdown() {
    let mut value = with_variables().to_value();
    let children = value["screens"][0]["children"].as_array_mut().unwrap();
    children.push(json!({ "id": "r", "type": "RichText", "props": { "text": "rich" } }));
    children.push(
        json!({ "id": "b", "type": "TextBody", "props": { "text": "md", "markdown": true } }),
    );
    children.push(json!({ "id": "p", "type": "TextBody", "props": { "text": "plain" } }));
    let mut definition = parse_definition(&value).unwrap();
    let mut markdown = Vec::new();
    map_display_text(&mut definition, &mut |text, is_markdown| {
        if is_markdown {
            markdown.push(text.clone());
        }
    });
    assert_eq!(markdown, ["rich", "md"]);
}

const WORD_JOINER: &str = "\u{2060}";

#[test]
fn escape_contact_value_splits_reference_and_variable_openers_so_a_value_stays_plain_text() {
    assert_eq!(
        escape_contact_value("${form.x} {{y}}", "Hi {{name}}", false),
        format!("${WORD_JOINER}{{form.x}} {{{WORD_JOINER}{{y}}}}")
    );
}

#[test]
fn escape_contact_value_escapes_markdown_characters_only_in_markdown_text() {
    assert_eq!(escape_contact_value("*a*", "{{name}}", true), "\\*a\\*");
    assert_eq!(escape_contact_value("*a*", "{{name}}", false), "*a*");
}

#[test]
fn escape_contact_value_escapes_quotes_inside_a_nested_expression() {
    assert_eq!(
        escape_contact_value("O'Neil", "`'Hi ' {{name}}`", false),
        "O\\'Neil"
    );
}
