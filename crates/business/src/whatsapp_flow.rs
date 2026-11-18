//! `parseFlowScreens` of `integrations/whatsapp/src/api/flow.ts`: the screens of a
//! WhatsApp Flow JSON and the answers each screen (and every screen it reaches)
//! completes with.

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde_json::{Map, Value, json};

type Obj = Map<String, Value>;

/// `humanizeKey`.
pub fn humanize_key(key: &str) -> String {
    static CAMEL: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"([a-z])([A-Z])").unwrap());
    let spaced = CAMEL
        .replace_all(&key.replace('_', " "), "$1 $2")
        .into_owned();
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

/// `forEachJsonNode`: every object, depth first, parent before children.
fn for_each_node(node: &Value, visit: &mut dyn FnMut(&Obj)) {
    match node {
        Value::Array(items) => items.iter().for_each(|i| for_each_node(i, visit)),
        Value::Object(obj) => {
            visit(obj);
            obj.values().for_each(|v| for_each_node(v, visit));
        }
        _ => {}
    }
}

fn on_click(obj: &Obj) -> Option<&Obj> {
    obj.get("on-click-action").and_then(Value::as_object)
}

fn screen_id(screen: &Value) -> &str {
    screen.get("id").and_then(Value::as_str).unwrap_or("")
}

/// `FORM_REFERENCE_REGEX`.
fn form_reference(value: &Value) -> Option<(Option<String>, String)> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^\$\{(?:screen\.([^.}]+)\.)?form\.([^}]+)\}$").unwrap()
    });
    let caps = RE.captures(value.as_str()?)?;
    Some((
        caps.get(1).map(|m| m.as_str().to_owned()),
        caps[2].to_owned(),
    ))
}

/// The component index of `indexFlowComponents` (only the type matters here).
struct Components {
    by_screen: HashMap<String, HashMap<String, String>>,
    by_name: HashMap<String, (String, bool)>,
}

fn static_label(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty() && !s.contains("${"))
}

fn index_components(screens: &[Value]) -> Components {
    let mut by_screen: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut by_name: HashMap<String, (String, bool)> = HashMap::new();
    for screen in screens {
        let on_screen = by_screen.entry(screen_id(screen).to_owned()).or_default();
        if let Some(layout) = screen.get("layout") {
            for_each_node(layout, &mut |obj| {
                let (Some(name), Some(kind)) = (
                    obj.get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty()),
                    obj.get("type").and_then(Value::as_str),
                ) else {
                    return;
                };
                if kind == "screen" {
                    return;
                }
                on_screen
                    .entry(name.to_owned())
                    .or_insert_with(|| kind.to_owned());
                let labelled = static_label(obj.get("label"));
                match by_name.get(name) {
                    Some((_, known)) if *known || !labelled => {}
                    _ => {
                        by_name.insert(name.to_owned(), (kind.to_owned(), labelled));
                    }
                }
            });
        }
    }
    Components { by_screen, by_name }
}

/// `readAnswerKind`: `file` for a PhotoPicker / DocumentPicker form answer.
fn answer_kind(screen: &str, value: &Value, components: &Components) -> &'static str {
    let Some((referenced, name)) = form_reference(value) else {
        return "value";
    };
    let on_screen = components
        .by_screen
        .get(referenced.as_deref().unwrap_or(screen))
        .and_then(|c| c.get(&name));
    let kind = on_screen.or_else(|| components.by_name.get(&name).map(|(k, _)| k));
    if matches!(
        kind.map(String::as_str),
        Some("PhotoPicker" | "DocumentPicker")
    ) {
        "file"
    } else {
        "value"
    }
}

/// `addAnswerKind`: first seen wins, `file` always wins.
fn add_kind(kinds: &mut Vec<(String, &'static str)>, key: &str, kind: &'static str) {
    match kinds.iter_mut().find(|(k, _)| k == key) {
        Some(entry) if kind == "file" => entry.1 = kind,
        Some(_) => {}
        None => kinds.push((key.to_owned(), kind)),
    }
}

/// `parseFlowScreens`.
pub fn parse_flow_screens(json: &Value) -> Vec<Value> {
    let Some(screens) = json.get("screens").and_then(Value::as_array) else {
        return Vec::new();
    };
    let routing = json.get("routing_model").and_then(Value::as_object);
    let components = index_components(screens);
    // `buildScreenIndex`.
    let mut outputs: HashMap<String, Vec<(String, &'static str)>> = HashMap::new();
    let mut routes: HashMap<String, Vec<String>> = HashMap::new();
    let mut labels: HashMap<String, String> = HashMap::new();
    for screen in screens {
        let Some(id) = screen
            .get("id")
            .and_then(Value::as_str)
            .filter(|i| !i.is_empty())
        else {
            continue;
        };
        let layout = screen.get("layout").cloned().unwrap_or(Value::Null);
        let mut kinds = Vec::new();
        let mut targets: Vec<String> = Vec::new();
        for_each_node(&layout, &mut |obj| {
            if let Some(action) = on_click(obj) {
                let name = action.get("name").and_then(Value::as_str);
                if name == Some("complete")
                    && let Some(payload) = action.get("payload").and_then(Value::as_object)
                {
                    for (key, value) in payload {
                        add_kind(&mut kinds, key, answer_kind(id, value, &components));
                    }
                }
                if name == Some("navigate")
                    && let Some(next) = action.get("next").and_then(Value::as_object)
                    && next.get("type").and_then(Value::as_str) == Some("screen")
                    && let Some(target) = next.get("name").and_then(Value::as_str)
                    && !targets.iter().any(|t| t == target)
                {
                    targets.push(target.to_owned());
                }
            }
            if let (Some(name), Some(label)) = (
                obj.get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty()),
                obj.get("label").and_then(Value::as_str),
            ) {
                labels
                    .entry(name.to_owned())
                    .or_insert_with(|| label.to_owned());
            }
        });
        outputs.insert(id.to_owned(), kinds);
        let route = match routing.and_then(|r| r.get(id)).and_then(Value::as_array) {
            Some(listed) => listed
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            None => targets,
        };
        routes.insert(id.to_owned(), route);
    }
    screens
        .iter()
        .map(|screen| {
            let id = screen.get("id").and_then(Value::as_str).unwrap_or("");
            // `getReachableScreenIds` (stack order as the original).
            let mut reachable: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            let mut pending = vec![id.to_owned()];
            while let Some(next) = pending.pop() {
                if next.is_empty() || !seen.insert(next.clone()) {
                    continue;
                }
                reachable.push(next.clone());
                pending.extend(routes.get(&next).cloned().unwrap_or_default());
            }
            let mut kinds = Vec::new();
            for screen_id in &reachable {
                for (key, kind) in outputs.get(screen_id).into_iter().flatten() {
                    add_kind(&mut kinds, key, kind);
                }
            }
            let output: Vec<Value> = kinds
                .into_iter()
                .map(|(key, kind)| {
                    let label = labels
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| humanize_key(&key));
                    json!({ "value": key, "label": label, "kind": kind })
                })
                .collect();
            json!({
                "id": id,
                "title": screen.get("title").and_then(Value::as_str).unwrap_or(id),
                "terminal": screen.get("terminal").and_then(Value::as_bool).unwrap_or(false),
                "output": output,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_snake_and_camel_keys() {
        assert_eq!(humanize_key("first_name"), "First name");
        assert_eq!(humanize_key("dateOfBirth"), "Date Of Birth");
    }

    #[test]
    fn collects_the_outputs_of_every_reachable_screen_with_file_kinds() {
        let flow = json!({ "screens": [
            { "id": "A", "title": "Start", "layout": { "children": [
                { "type": "TextInput", "name": "first_name", "label": "First name" },
                { "type": "Footer", "on-click-action": { "name": "navigate",
                    "next": { "type": "screen", "name": "B" }, "payload": {} } },
            ] } },
            { "id": "B", "terminal": true, "layout": { "children": [
                { "type": "PhotoPicker", "name": "photo", "label": "Photo" },
                { "type": "Footer", "on-click-action": { "name": "complete", "payload": {
                    "first_name": "${screen.A.form.first_name}", "photo": "${form.photo}", "orderId": "42" } } },
            ] } },
        ] });
        let screens = parse_flow_screens(&flow);
        assert_eq!(screens.len(), 2);
        assert_eq!(
            screens[0],
            json!({ "id": "A", "title": "Start", "terminal": false, "output": [
                // ponytail: serde_json maps are sorted (no `preserve_order`), so a
                // payload's keys come in key order, not the Flow JSON's order.
                { "value": "first_name", "label": "First name", "kind": "value" },
                { "value": "orderId", "label": "Order Id", "kind": "value" },
                { "value": "photo", "label": "Photo", "kind": "file" },
            ] })
        );
        assert_eq!(screens[1]["title"], "B");
        assert_eq!(screens[1]["terminal"], true);
    }

    #[test]
    fn a_flow_without_screens_has_none() {
        assert!(parse_flow_screens(&json!(null)).is_empty());
        assert!(parse_flow_screens(&json!({ "screens": {} })).is_empty());
    }
}
