//! `packages/mini-app/__tests__/fixtures.ts`.
#![allow(dead_code)]

use serde_json::{Value, json};

/// Two screens: collect a name, then confirm and complete.
pub fn two_screen_definition() -> Value {
    json!({ "screens": [
        { "key": "s_welcome", "id": "WELCOME", "title": "Welcome", "children": [
            { "id": "n_heading", "type": "TextHeading", "props": { "text": "Book a visit" } },
            { "id": "n_name", "type": "TextInput", "props": {
                "name": "full_name", "label": "Full name", "input-type": "text", "required": true, "helper-text": "" } },
            { "id": "n_next", "type": "Footer", "props": {
                "label": "Continue", "on-click-action": { "name": "navigate", "next": "s_confirm" } } }
        ] },
        { "key": "s_confirm", "id": "CONFIRM", "title": "Confirm", "children": [
            { "id": "n_body", "type": "TextBody", "props": { "text": "Thanks ${screen.WELCOME.form.full_name}" } },
            { "id": "n_terms", "type": "OptIn", "props": { "name": "terms", "label": "I agree", "required": true } },
            { "id": "n_done", "type": "Footer", "props": { "label": "Submit", "on-click-action": { "name": "complete" } } }
        ] }
    ] })
}

/// The Flow JSON `two_screen_definition` serializes to.
pub fn two_screen_flow_json() -> Value {
    json!({ "version": "7.3", "screens": [
        { "id": "WELCOME", "title": "Welcome", "layout": { "type": "SingleColumnLayout", "children": [
            { "type": "TextHeading", "text": "Book a visit" },
            { "type": "TextInput", "name": "full_name", "label": "Full name", "input-type": "text", "required": true },
            { "type": "Footer", "label": "Continue", "on-click-action": {
                "name": "navigate", "next": { "type": "screen", "name": "CONFIRM" }, "payload": {} } }
        ] } },
        { "id": "CONFIRM", "title": "Confirm", "terminal": true, "success": true, "layout": { "type": "SingleColumnLayout", "children": [
            { "type": "TextBody", "text": "`'Thanks ' ${screen.WELCOME.form.full_name}`" },
            { "type": "OptIn", "name": "terms", "label": "I agree", "required": true },
            { "type": "Footer", "label": "Submit", "on-click-action": { "name": "complete", "payload": {
                "full_name": "${screen.WELCOME.form.full_name}", "terms": "${form.terms}" } } }
        ] } }
    ] })
}

pub fn children(definition: &mut Value, screen: usize) -> &mut Vec<Value> {
    definition["screens"][screen]["children"].as_array_mut().expect("children")
}
