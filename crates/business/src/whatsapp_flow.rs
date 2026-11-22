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

/// `FlowComponentInfo`.
struct Info {
    kind: String,
    label: Option<String>,
    options: Obj,
}

/// `indexFlowComponents`: screen id → input name → component, and input
/// name → the first component with a static label, else the first.
struct Components {
    by_screen: HashMap<String, HashMap<String, Info>>,
    by_name: HashMap<String, Info>,
}

/// `readStaticLabel`.
fn static_label(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && !s.contains("${"))
        .map(str::to_owned)
}

/// `readStaticOptions`.
fn static_options(data_source: Option<&Value>) -> Obj {
    data_source
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            Some((
                o.get("id")?.as_str()?.to_owned(),
                o.get("title")?.as_str()?.into(),
            ))
        })
        .collect()
}

fn index_components(screens: &[Value]) -> Components {
    let mut by_screen: HashMap<String, HashMap<String, Info>> = HashMap::new();
    let mut by_name: HashMap<String, Info> = HashMap::new();
    for screen in screens {
        let on_screen = by_screen.entry(screen_id(screen).to_owned()).or_default();
        let Some(layout) = screen.get("layout") else {
            continue;
        };
        for_each_node(layout, &mut |obj| {
            let (Some(name), Some(kind)) = (
                obj.get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty()),
                obj.get("type").and_then(Value::as_str),
            ) else {
                return;
            };
            // A navigate target (`next: { type: "screen", name }`) is no input.
            if kind == "screen" {
                return;
            }
            let info = || Info {
                kind: kind.to_owned(),
                label: static_label(obj.get("label")),
                options: static_options(obj.get("data-source")),
            };
            on_screen.entry(name.to_owned()).or_insert_with(info);
            let replace = by_name.get(name).is_none_or(|known| {
                known.label.is_none() && static_label(obj.get("label")).is_some()
            });
            if replace {
                by_name.insert(name.to_owned(), info());
            }
        });
    }
    Components { by_screen, by_name }
}

/// `findFormReferencedComponent`: the input a `${form.X}` /
/// `${screen.S.form.X}` payload value reads, on the named screen first.
fn form_component<'a>(screen: &str, value: &Value, components: &'a Components) -> Option<&'a Info> {
    let (referenced, name) = form_reference(value)?;
    components
        .by_screen
        .get(referenced.as_deref().unwrap_or(screen))
        .and_then(|c| c.get(&name))
        .or_else(|| components.by_name.get(&name))
}

/// `readAnswerKind`: `file` for a PhotoPicker / DocumentPicker form answer.
fn answer_kind(screen: &str, value: &Value, components: &Components) -> &'static str {
    match form_component(screen, value, components).map(|i| i.kind.as_str()) {
        Some("PhotoPicker" | "DocumentPicker") => "file",
        _ => "value",
    }
}

/// `WhatsappFlowResponseField`: a response key as the Flow JSON labels it.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowResponseLabel {
    pub key: String,
    pub label: String,
    pub options: Obj,
    pub labelled: bool,
}

/// Every `(key, value)` one screen's `complete` actions return.
fn complete_payload_entries(screen: &Value) -> Vec<(String, Value)> {
    let mut entries = Vec::new();
    if let Some(layout) = screen.get("layout") {
        for_each_node(layout, &mut |obj| {
            if let Some(action) = on_click(obj)
                && action.get("name").and_then(Value::as_str) == Some("complete")
                && let Some(payload) = action.get("payload").and_then(Value::as_object)
            {
                entries.extend(payload.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        });
    }
    entries
}

/// `parseFlowResponseFields`: each key the Flow's `complete` payloads return
/// (the first screen that names it decides), traced through its form
/// reference to the input that labels it. Endpoint Flows list none.
///
/// ponytail: serde_json maps are sorted (no `preserve_order`), so keys come in
/// key order per payload, not the Flow JSON's question order.
pub fn parse_flow_response_fields(json: &Value) -> Vec<FlowResponseLabel> {
    let Some(screens) = json.get("screens").and_then(Value::as_array) else {
        return Vec::new();
    };
    let components = index_components(screens);
    let mut seen = HashSet::new();
    let mut fields = Vec::new();
    for screen in screens {
        for (key, value) in complete_payload_entries(screen) {
            if !seen.insert(key.clone()) {
                continue;
            }
            let component = if form_reference(&value).is_some() {
                form_component(screen_id(screen), &value, &components)
            } else {
                components.by_name.get(&key)
            };
            let label = component.and_then(|c| c.label.clone());
            fields.push(FlowResponseLabel {
                labelled: label.is_some(),
                label: label.unwrap_or_else(|| humanize_key(&key)),
                options: component.map(|c| c.options.clone()).unwrap_or_default(),
                key,
            });
        }
    }
    fields
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
