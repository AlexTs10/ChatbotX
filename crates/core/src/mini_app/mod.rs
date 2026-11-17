//! Port of the server part of `packages/mini-app`: the definition schema,
//! the component catalog, Flow JSON import / export, validation, custom field
//! mappings, answers and upload rules. The editor tree mutations, the display
//! text rendering and the runtime evaluation stay in TypeScript (the web
//! editor and runner import the package directly).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub mod catalog;
pub mod expression;
pub mod runtime;
pub mod serialize;
pub mod validate;

pub use catalog::{Component, component, needs_name};
pub use runtime::*;
pub use serialize::{ImportError, from_flow_json, to_flow_json, to_nested_expression};
pub use validate::{LocatedIssue, ValidationIssue, ValidationResult, locate_issues, validate};

/// `MINI_APP_FLOW_JSON_VERSION`.
pub const FLOW_JSON_VERSION: &str = "7.3";
/// `MINI_APP_MAX_SCREENS`.
pub const MAX_SCREENS: usize = 100;
/// `MINI_APP_SCREEN_TITLE_MAX_LENGTH`.
pub const SCREEN_TITLE_MAX_LENGTH: usize = 80;
/// `MINI_APP_NAME_MAX_LENGTH`.
pub const NAME_MAX_LENGTH: usize = 100;
const MAX_ID_LENGTH: usize = 64;
const MAX_STORED_TITLE_LENGTH: usize = 1000;

/// `MINI_APP_ACTION_KEY`.
pub const ACTION_KEY: &str = "on-click-action";
/// `MINI_APP_LIST_ITEMS_KEY`.
pub const LIST_ITEMS_KEY: &str = "list-items";

/// `MiniAppNode`. Slots are a sorted map (as a jsonb column); `slot_order`
/// keeps the Switch case order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub props: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<BTreeMap<String, Vec<Node>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_order: Option<Vec<String>>,
    // ponytail: `null` and absent are one state here (zod kept `null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_field_id: Option<String>,
}

/// `MiniAppScreen`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub key: String,
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub children: Vec<Node>,
}

/// `MiniAppDefinition`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub screens: Vec<Screen>,
}

/// `MiniAppAction`.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Navigate(String),
    Complete,
    OpenUrl(String),
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Navigate(_) => "navigate",
            Action::Complete => "complete",
            Action::OpenUrl(_) => "open_url",
        }
    }
}

use crate::js::utf16_len;

fn id_ok(text: &str) -> bool {
    (1..=MAX_ID_LENGTH).contains(&utf16_len(text))
}

fn node_ok(node: &Node) -> bool {
    id_ok(&node.id)
        && catalog::component(&node.kind).is_some()
        && node.custom_field_id.as_deref().is_none_or(|id| utf16_len(id) <= MAX_ID_LENGTH)
        && node
            .slots
            .iter()
            .flat_map(|slots| slots.values().flatten())
            .all(node_ok)
}

/// `miniAppDefinitionSchema.safeParse`: the definition, or `None` when it is
/// not one.
pub fn parse_definition(value: &Value) -> Option<Definition> {
    let definition: Definition = serde_json::from_value(value.clone()).ok()?;
    let ok = (1..=MAX_SCREENS).contains(&definition.screens.len())
        && definition.screens.iter().all(|screen| {
            id_ok(&screen.key)
                && id_ok(&screen.id)
                && utf16_len(&screen.title) <= MAX_STORED_TITLE_LENGTH
                && screen.children.iter().all(node_ok)
        });
    ok.then_some(definition)
}

impl Definition {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

// ---------------------------------------------------------------- tree

/// `readMiniAppAction`.
pub fn read_action(value: Option<&Value>) -> Option<Action> {
    let object = value?.as_object()?;
    match object.get("name")?.as_str()? {
        "navigate" => {
            let next = object.get("next")?.as_str()?;
            (!next.is_empty()).then(|| Action::Navigate(next.to_owned()))
        }
        "complete" => Some(Action::Complete),
        "open_url" => {
            let url = object.get("url")?.as_str()?;
            url::Url::parse(url).ok()?;
            Some(Action::OpenUrl(url.to_owned()))
        }
        _ => None,
    }
}

/// One node visit of `walkNodes`.
pub struct Visit<'a> {
    pub node: &'a Node,
    pub parent: Option<&'a Node>,
    pub slot: Option<&'a str>,
    pub index: usize,
    pub if_depth: usize,
    pub in_conditional: bool,
}

/// `slotEntries`: slot order, then the If default order, then the map order.
pub fn slot_entries(node: &Node) -> Vec<(&str, &Vec<Node>)> {
    let Some(slots) = &node.slots else {
        return Vec::new();
    };
    let defaults: &[&str] = if node.kind == "If" { &["then", "else"] } else { &[] };
    let mut seen = Vec::<&str>::new();
    let order = node.slot_order.iter().flatten().map(String::as_str);
    for key in order.chain(defaults.iter().copied()).chain(slots.keys().map(String::as_str)) {
        if let Some((k, _)) = slots.get_key_value(key)
            && !seen.contains(&k.as_str())
        {
            seen.push(k.as_str());
        }
    }
    seen.into_iter().map(|key| (key, &slots[key])).collect()
}

fn walk_inner<'a>(
    nodes: &'a [Node],
    parent: Option<&'a Node>,
    slot: Option<&'a str>,
    if_depth: usize,
    in_conditional: bool,
    visit: &mut dyn FnMut(Visit<'a>),
) {
    for (index, node) in nodes.iter().enumerate() {
        visit(Visit { node, parent, slot, index, if_depth, in_conditional });
        let is_if = node.kind == "If";
        let conditional = in_conditional || is_if || node.kind == "Switch";
        for (key, children) in slot_entries(node) {
            walk_inner(children, Some(node), Some(key), if_depth + usize::from(is_if), conditional, visit);
        }
    }
}

/// `walkNodes`.
pub fn walk<'a>(nodes: &'a [Node], visit: &mut dyn FnMut(Visit<'a>)) {
    walk_inner(nodes, None, None, 0, false, visit);
}

/// `walkNodes` over mutable nodes (pre-order, slot map order).
pub fn walk_mut(nodes: &mut [Node], visit: &mut dyn FnMut(&mut Node)) {
    for node in nodes {
        visit(node);
        for children in node.slots.iter_mut().flat_map(|slots| slots.values_mut()) {
            walk_mut(children, visit);
        }
    }
}

/// `actionHolders`: the node props, then each NavigationList item.
pub fn action_holders(node: &Node) -> Vec<&Map<String, Value>> {
    let mut holders = vec![&node.props];
    if let Some(Value::Array(items)) = node.props.get(LIST_ITEMS_KEY) {
        holders.extend(items.iter().filter_map(Value::as_object));
    }
    holders
}

/// `screenActions`.
pub fn screen_actions(screen: &Screen) -> Vec<Action> {
    let mut actions = Vec::new();
    walk(&screen.children, &mut |visit| {
        for holder in action_holders(visit.node) {
            actions.extend(read_action(holder.get(ACTION_KEY)));
        }
    });
    actions
}

/// `isTerminalScreen`.
pub fn is_terminal_screen(screen: &Screen) -> bool {
    screen_actions(screen).iter().any(|a| *a == Action::Complete)
}

/// `nodeName`.
pub fn node_name(node: &Node) -> Option<&str> {
    node.props.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())
}

/// `inputName`.
pub fn input_name(node: &Node) -> Option<&str> {
    if catalog::component(&node.kind)?.is_input { node_name(node) } else { None }
}

/// `collectInputNames`: input names of a screen, in document order.
pub fn collect_input_names(screen: &Screen) -> Vec<String> {
    let mut names = Vec::new();
    walk(&screen.children, &mut |visit| names.extend(input_name(visit.node).map(str::to_owned)));
    names
}

/// `findScreen`.
pub fn find_screen<'a>(definition: &'a Definition, key: &str) -> Option<&'a Screen> {
    definition.screens.iter().find(|s| s.key == key)
}

// ------------------------------------------------------------- factory

const ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// `createMiniAppId`.
pub fn create_id(prefix: &str) -> String {
    let mut bytes = [0u8; 10];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    let random: String = bytes
        .iter()
        .map(|b| ID_ALPHABET[usize::from(*b) % ID_ALPHABET.len()] as char)
        .collect();
    format!("{prefix}_{random}")
}

/// `createStarterDefinition` with the English builder texts
/// (`miniApps.editor.newScreenTitle`, `miniApps.defaults.*`).
pub fn starter_definition() -> Definition {
    let heading = Node {
        id: create_id("n"),
        kind: "TextHeading".into(),
        props: obj(json!({ "text": "Heading" })),
        slots: None,
        slot_order: None,
        custom_field_id: None,
    };
    let footer = Node {
        id: create_id("n"),
        kind: "Footer".into(),
        props: obj(json!({ "label": "Continue", ACTION_KEY: { "name": "complete" } })),
        slots: None,
        slot_order: None,
        custom_field_id: None,
    };
    Definition {
        screens: vec![Screen {
            key: create_id("s"),
            id: "WELCOME".into(),
            title: "New screen".into(),
            children: vec![heading, footer],
        }],
    }
}

fn obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

// --------------------------------------------------------- navigation

/// `buildNavigationGraph`.
fn navigation_graph(definition: &Definition) -> Vec<(&str, HashSet<String>)> {
    definition
        .screens
        .iter()
        .map(|screen| {
            let targets = screen_actions(screen)
                .into_iter()
                .filter_map(|a| match a {
                    Action::Navigate(next) => Some(next),
                    _ => None,
                })
                .collect();
            (screen.key.as_str(), targets)
        })
        .collect()
}

/// `collectPathScreens`: the screens from which `screen_key` can be reached.
pub fn collect_path_screens(definition: &Definition, screen_key: &str) -> BTreeSet<String> {
    let graph = navigation_graph(definition);
    let mut reached = BTreeSet::from([screen_key.to_owned()]);
    let mut changed = true;
    while changed {
        changed = false;
        for (source, targets) in &graph {
            if !reached.contains(*source) && targets.iter().any(|t| reached.contains(t)) {
                reached.insert((*source).to_owned());
                changed = true;
            }
        }
    }
    reached
}
