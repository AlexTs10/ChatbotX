//! `serialize/*`: the definition ⇄ Meta Flow JSON conversion.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::catalog::text_input_date_format;
use super::expression::is_nested_expression;
use super::{
    ACTION_KEY, Action, Definition, FLOW_JSON_VERSION, LIST_ITEMS_KEY, Node, Screen, collect_input_names,
    collect_path_screens, create_id, is_terminal_screen, read_action, slot_entries,
};

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => js_trim(s).is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

pub use crate::js::trim as js_trim;

/// `cleanObject`: drops empty strings, empty arrays and null values.
pub fn clean_object(value: &Map<String, Value>) -> Map<String, Value> {
    let mut result = Map::new();
    for (key, entry) in value {
        if is_empty_value(entry) {
            continue;
        }
        match entry {
            Value::Object(nested) => {
                let nested = clean_object(nested);
                if !nested.is_empty() {
                    result.insert(key.clone(), Value::Object(nested));
                }
            }
            Value::Array(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        Value::Object(o) => Value::Object(clean_object(o)),
                        other => other.clone(),
                    })
                    .collect();
                result.insert(key.clone(), Value::Array(items));
            }
            other => {
                result.insert(key.clone(), other.clone());
            }
        }
    }
    result
}

/// `buildCompletePayload`.
pub fn build_complete_payload(definition: &Definition, screen_key: &str) -> Map<String, Value> {
    let on_path = collect_path_screens(definition, screen_key);
    let mut payload = Map::new();
    for screen in definition.screens.iter().filter(|s| on_path.contains(&s.key)) {
        for name in collect_input_names(screen) {
            let value = if screen.key == screen_key {
                format!("${{form.{name}}}")
            } else {
                format!("${{screen.{}.form.{name}}}", screen.id)
            };
            payload.insert(name, Value::String(value));
        }
    }
    payload
}

struct Context<'a> {
    definition: &'a Definition,
    screen: &'a Screen,
    screen_ids: HashMap<&'a str, &'a str>,
}

fn serialize_action(value: Option<&Value>, context: &Context) -> Option<Value> {
    match read_action(value)? {
        Action::Navigate(next) => {
            let target = context.screen_ids.get(next.as_str())?;
            Some(json!({ "name": "navigate", "next": { "type": "screen", "name": target }, "payload": {} }))
        }
        Action::Complete => Some(json!({
            "name": "complete",
            "payload": build_complete_payload(context.definition, &context.screen.key),
        })),
        Action::OpenUrl(url) => Some(json!({ "name": "open_url", "url": url })),
    }
}

fn whole_reference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\$\{[^}]*\}$").expect("regex"))
}

fn reference_split() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$\{[^}]*\}").expect("regex"))
}

fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'").replace('`', "'"))
}

/// `toNestedExpression`: "Hello ${form.name}" → "`'Hello ' ${form.name}`".
pub fn to_nested_expression(text: &str) -> String {
    let trimmed = js_trim(text);
    if !text.contains("${") || whole_reference().is_match(trimmed) || is_nested_expression(trimmed) {
        return text.to_owned();
    }
    // `text.split(/(\$\{[^}]*\})/)` keeping the separators, without empty parts.
    let mut parts = Vec::new();
    let mut last = 0;
    for found in reference_split().find_iter(text) {
        if found.start() > last {
            parts.push(quote_literal(&text[last..found.start()]));
        }
        parts.push(found.as_str().to_owned());
        last = found.end();
    }
    if last < text.len() {
        parts.push(quote_literal(&text[last..]));
    }
    format!("`{}`", parts.join(" "))
}

fn to_nested_expressions(value: &Value, key: Option<&str>) -> Value {
    match value {
        Value::String(s) if matches!(key, Some("condition" | "value")) => Value::String(s.clone()),
        Value::String(s) => Value::String(to_nested_expression(s)),
        Value::Array(items) => Value::Array(items.iter().map(|i| to_nested_expressions(i, None)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), to_nested_expressions(v, Some(k))))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn nested_props(map: &Map<String, Value>) -> Map<String, Value> {
    match to_nested_expressions(&Value::Object(clean_object(map)), None) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn serialize_node(node: &Node, context: &Context) -> Value {
    let mut rest = node.props.clone();
    let action = rest.remove(ACTION_KEY);
    let list_items = rest.remove(LIST_ITEMS_KEY);
    let mut output = Map::new();
    output.insert("type".into(), Value::String(node.kind.clone()));
    output.extend(nested_props(&rest));
    if node.kind == "TextInput"
        && let Some((pattern, helper)) = rest.get("input-type").and_then(Value::as_str).and_then(text_input_date_format)
    {
        output.insert("input-type".into(), json!("text"));
        output.entry("pattern").or_insert_with(|| json!(pattern));
        output.entry("helper-text").or_insert_with(|| json!(helper));
    }
    if let Some(Value::Array(items)) = list_items {
        let items = items
            .iter()
            .map(|item| {
                let mut item = item.as_object().cloned().unwrap_or_default();
                let item_action = item.remove(ACTION_KEY);
                let mut out = nested_props(&item);
                if let Some(action) = serialize_action(item_action.as_ref(), context) {
                    out.insert(ACTION_KEY.into(), action);
                }
                Value::Object(out)
            })
            .collect();
        output.insert(LIST_ITEMS_KEY.into(), Value::Array(items));
    }
    if let Some(action) = serialize_action(action.as_ref(), context) {
        output.insert(ACTION_KEY.into(), action);
    }
    let list = |nodes: Option<&Vec<Node>>| -> Value {
        Value::Array(nodes.into_iter().flatten().map(|n| serialize_node(n, context)).collect())
    };
    let slot = |key: &str| node.slots.as_ref().and_then(|s| s.get(key));
    match node.kind.as_str() {
        "Form" => {
            output.insert("children".into(), list(slot("children")));
        }
        "If" => {
            output.insert("then".into(), list(slot("then")));
            let else_branch = list(slot("else"));
            if else_branch.as_array().is_some_and(|a| !a.is_empty()) {
                output.insert("else".into(), else_branch);
            }
        }
        "Switch" => {
            let cases = slot_entries(node)
                .into_iter()
                .map(|(key, nodes)| (key.to_owned(), list(Some(nodes))))
                .collect();
            output.insert("cases".into(), Value::Object(cases));
        }
        _ => {}
    }
    Value::Object(output)
}

/// `toFlowJson`: the editor definition as Meta's Flow JSON 7.3.
pub fn to_flow_json(definition: &Definition) -> Value {
    let screen_ids = definition.screens.iter().map(|s| (s.key.as_str(), s.id.as_str())).collect();
    let mut context = Context { definition, screen: &definition.screens[0], screen_ids };
    let screens: Vec<Value> = definition
        .screens
        .iter()
        .map(|screen| {
            context.screen = screen;
            let mut out = Map::new();
            out.insert("id".into(), json!(screen.id));
            let title = js_trim(&screen.title);
            if !title.is_empty() {
                out.insert("title".into(), json!(title));
            }
            if is_terminal_screen(screen) {
                out.insert("terminal".into(), json!(true));
                out.insert("success".into(), json!(true));
            }
            let children: Vec<Value> = screen.children.iter().map(|n| serialize_node(n, &context)).collect();
            out.insert("layout".into(), json!({ "type": "SingleColumnLayout", "children": children }));
            Value::Object(out)
        })
        .collect();
    json!({ "version": FLOW_JSON_VERSION, "screens": screens })
}

/// `MiniAppImportError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    pub code: &'static str,
    pub path: String,
}

fn import_error(code: &'static str, path: impl Into<String>) -> ImportError {
    ImportError { code, path: path.into() }
}

fn import_action(value: &Value, path: &str, keys: &HashMap<String, String>) -> Result<Value, ImportError> {
    let name = value.get("name").and_then(Value::as_str);
    match name {
        Some("navigate") => {
            let next = value.get("next").and_then(|n| n.get("name"));
            let next = match next {
                Some(Value::String(s)) => s.clone(),
                None | Some(Value::Null) => String::new(),
                Some(other) => crate::js::string_of(other),
            };
            let key = keys.get(&next).ok_or_else(|| import_error("unknown_screen", format!("{path}.next.name")))?;
            Ok(json!({ "name": "navigate", "next": key }))
        }
        Some("complete") => Ok(json!({ "name": "complete" })),
        Some("open_url") => match value.get("url") {
            Some(Value::String(url)) => Ok(json!({ "name": "open_url", "url": url })),
            _ => Err(import_error("unsupported_action", path)),
        },
        _ => Err(import_error("unsupported_action", path)),
    }
}

fn import_list(value: Option<&Value>, path: &str, keys: &HashMap<String, String>) -> Result<Vec<Node>, ImportError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, child)| import_node(child, &format!("{path}[{i}]"), keys))
            .collect(),
        Some(_) => Err(import_error("invalid_component", path)),
    }
}

fn import_node(component: &Value, path: &str, keys: &HashMap<String, String>) -> Result<Node, ImportError> {
    let empty = Map::new();
    let object = component.as_object().unwrap_or(&empty);
    let kind = object.get("type").and_then(Value::as_str).filter(|k| super::component(k).is_some());
    let kind = kind.ok_or_else(|| import_error("unknown_component", format!("{path}.type")))?.to_owned();
    for key in ["on-select-action", "on-unselect-action"] {
        if object.contains_key(key) {
            return Err(import_error("unsupported_property", format!("{path}.{key}")));
        }
    }
    let mut props = object.clone();
    for key in ["type", "children", "then", "else", "cases"] {
        props.remove(key);
    }
    if let Some(action) = props.remove(ACTION_KEY) {
        props.insert(ACTION_KEY.into(), import_action(&action, &format!("{path}.{ACTION_KEY}"), keys)?);
    }
    if let Some(Value::Array(items)) = props.remove(LIST_ITEMS_KEY) {
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let mut item = item.as_object().cloned().unwrap_or_default();
            if let Some(action) = item.remove(ACTION_KEY) {
                let action_path = format!("{path}.{LIST_ITEMS_KEY}[{i}].{ACTION_KEY}");
                item.insert(ACTION_KEY.into(), import_action(&action, &action_path, keys)?);
            }
            out.push(Value::Object(item));
        }
        props.insert(LIST_ITEMS_KEY.into(), Value::Array(out));
    }
    let mut node = Node { id: create_id("n"), kind, props, slots: None, slot_order: None, custom_field_id: None };
    match node.kind.as_str() {
        "Form" => {
            let children = import_list(object.get("children"), &format!("{path}.children"), keys)?;
            node.slots = Some([("children".to_owned(), children)].into());
        }
        "If" => {
            let then = import_list(object.get("then"), &format!("{path}.then"), keys)?;
            let else_ = import_list(object.get("else"), &format!("{path}.else"), keys)?;
            node.slots = Some([("then".to_owned(), then), ("else".to_owned(), else_)].into());
        }
        "Switch" => {
            let Some(Value::Object(cases)) = object.get("cases") else {
                return Err(import_error("invalid_component", format!("{path}.cases")));
            };
            let mut slots = std::collections::BTreeMap::new();
            for (key, nodes) in cases {
                slots.insert(key.clone(), import_list(Some(nodes), &format!("{path}.cases.{key}"), keys)?);
            }
            node.slot_order = Some(cases.keys().cloned().collect());
            node.slots = Some(slots);
        }
        _ => {}
    }
    Ok(node)
}

/// `flowJsonSchema.safeParse`: a loose Flow JSON object with 1–100 screens.
pub fn is_flow_json(value: &Value) -> bool {
    let Some(object) = value.as_object() else { return false };
    let Some(Value::Array(screens)) = object.get("screens") else { return false };
    object.get("version").is_some_and(Value::is_string)
        && (1..=super::MAX_SCREENS).contains(&screens.len())
        && screens.iter().all(|screen| {
            let layout = screen.get("layout");
            screen.get("id").is_some_and(Value::is_string)
                && screen.get("title").is_none_or(Value::is_string)
                && screen.get("terminal").is_none_or(Value::is_boolean)
                && layout.and_then(|l| l.get("type")).and_then(Value::as_str) == Some("SingleColumnLayout")
                && layout
                    .and_then(|l| l.get("children"))
                    .and_then(Value::as_array)
                    .is_some_and(|c| c.iter().all(|c| c.get("type").is_some_and(Value::is_string)))
        })
}

/// `fromFlowJson`: Meta Flow JSON (endpoint-less) as an editor definition,
/// as JSON (`parseDefinition` checks it next). `flow_json` passed
/// [`is_flow_json`].
pub fn from_flow_json(flow_json: &Value) -> Result<Value, ImportError> {
    if flow_json.get("data_api_version").is_some() || flow_json.get("data_channel_uri").is_some() {
        return Err(import_error("unsupported_endpoint", "data_api_version"));
    }
    let screens = flow_json.get("screens").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut keys = HashMap::new();
    for screen in &screens {
        let id = screen.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
        keys.insert(id, create_id("s"));
    }
    let mut out = Vec::new();
    for (screen_index, screen) in screens.iter().enumerate() {
        let id = screen.get("id").and_then(Value::as_str).unwrap_or_default();
        let children = screen
            .get("layout")
            .and_then(|l| l.get("children"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut nodes = Vec::new();
        for (index, component) in children.iter().enumerate() {
            let path = format!("screens[{screen_index}].layout.children[{index}]");
            nodes.push(serde_json::to_value(import_node(component, &path, &keys)?).unwrap_or(Value::Null));
        }
        out.push(json!({
            "key": keys[id],
            "id": id,
            "title": screen.get("title").and_then(Value::as_str).unwrap_or_default(),
            "children": nodes,
        }));
    }
    Ok(json!({ "screens": out }))
}
