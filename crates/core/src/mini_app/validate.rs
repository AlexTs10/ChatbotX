//! `validate/*`, `display-text.ts` (the parts validation needs) and
//! `issue-location.ts`: Meta's Flow JSON rules on a definition.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::catalog::{
    MAX_COMPONENTS_PER_SCREEN, MAX_DROPDOWN_OPTIONS_WITH_IMAGES, MAX_IF_DEPTH, can_place_in_slot,
};
use super::expression::{extract_references, is_valid_expression};
use super::runtime::{MAX_UPLOADS_PER_INPUT, WHATSAPP_MAX_FILES_PER_RESPONSE};
use super::serialize::{clean_object, js_trim};
use super::{
    ACTION_KEY, Action, Definition, LIST_ITEMS_KEY, Node, SCREEN_TITLE_MAX_LENGTH, Screen, Visit,
    collect_input_names, collect_path_screens, component, find_screen, input_name,
    is_terminal_screen, needs_name, node_name, parse_definition, read_action, screen_actions,
    slot_entries, walk,
};
use crate::flow_config::z::{Issue, IssueCode};

/// `MiniAppValidationIssue`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationIssue {
    pub code: &'static str,
    pub screen_key: Option<String>,
    pub node_id: Option<String>,
    pub property: Option<String>,
    pub params: Option<Map<String, Value>>,
    /// `error` or `warning`.
    pub severity: &'static str,
}

/// `MiniAppValidationResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationResult {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Default)]
struct Report {
    code: &'static str,
    property: Option<String>,
    params: Option<Map<String, Value>>,
    warning: bool,
}

fn report(code: &'static str) -> Report {
    Report {
        code,
        ..Report::default()
    }
}

impl Report {
    fn prop(mut self, property: impl Into<String>) -> Self {
        self.property = Some(property.into());
        self
    }
    fn param(mut self, key: &str, value: Value) -> Self {
        self.params
            .get_or_insert_with(Map::new)
            .insert(key.into(), value);
        self
    }
    fn warning(mut self) -> Self {
        self.warning = true;
        self
    }
}

#[derive(Default)]
struct Collector {
    issues: Vec<ValidationIssue>,
}

impl Collector {
    fn add(&mut self, screen: Option<&Screen>, node: Option<&Node>, r: Report) {
        self.issues.push(ValidationIssue {
            code: r.code,
            screen_key: screen.map(|s| s.key.clone()),
            node_id: node.map(|n| n.id.clone()),
            property: r.property,
            params: r.params,
            severity: if r.warning { "warning" } else { "error" },
        });
    }
}

fn num(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

// ------------------------------------------------------- display text

fn contact_variable() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{[^{}\n]+\}\}").expect("regex"))
}

/// `hasContactVariable`.
pub fn has_contact_variable(text: &str) -> bool {
    contact_variable().is_match(text)
}

/// `text.replace(CONTACT_VARIABLE_PATTERN, "")`.
pub fn strip_contact_variables(text: &str) -> String {
    contact_variable().replace_all(text, "").into_owned()
}

fn collapse_contact_variables(text: &str) -> String {
    contact_variable().replace_all(text, "x").into_owned()
}

fn display_text_props(kind: &str) -> &'static [&'static str] {
    match kind {
        "TextHeading" | "TextSubheading" | "TextBody" | "TextCaption" | "RichText"
        | "EmbeddedLink" => &["text"],
        "TextInput" | "TextArea" | "DatePicker" => &["label", "helper-text", "error-message"],
        "Dropdown" => &["label", "error-message"],
        "RadioButtonsGroup" | "CheckboxGroup" | "ChipsSelector" | "PhotoPicker"
        | "DocumentPicker" => &["label", "description", "error-message"],
        "CalendarPicker" => &[
            "label",
            "title",
            "description",
            "helper-text",
            "error-message",
        ],
        "OptIn" => &["label"],
        "Footer" => &["label", "left-caption", "center-caption", "right-caption"],
        _ => &[],
    }
}

fn map_strings(holder: Option<&mut Value>, keys: &[&str], f: &mut dyn FnMut(&mut String, bool)) {
    if let Some(Value::Object(holder)) = holder {
        for key in keys {
            if let Some(Value::String(text)) = holder.get_mut(*key) {
                f(text, false);
            }
        }
    }
}

/// The display texts of a node's props (`nodeDisplayTextSlots`), mutably;
/// `f` also gets whether the text is markdown (`isMarkdownText`).
fn for_each_display_text(
    kind: &str,
    props: &mut Map<String, Value>,
    f: &mut dyn FnMut(&mut String, bool),
) {
    let markdown = kind == "RichText"
        || (kind == "TextBody" && props.get("markdown") == Some(&Value::Bool(true)));
    for key in display_text_props(kind) {
        if let Some(Value::String(text)) = props.get_mut(*key) {
            f(text, markdown && *key == "text");
        }
    }
    if let Some(Value::Array(options)) = props.get_mut("data-source") {
        for option in options {
            map_strings(Some(option), &["title", "description", "metadata"], f);
        }
    }
    if kind == "NavigationList"
        && let Some(Value::Array(items)) = props.get_mut(LIST_ITEMS_KEY)
    {
        const CONTENT: &[&str] = &["title", "description", "metadata"];
        for item in items {
            let Value::Object(item) = item else { continue };
            map_strings(item.get_mut("main-content"), CONTENT, f);
            map_strings(item.get_mut("end"), CONTENT, f);
            if let Some(Value::String(badge)) = item.get_mut("badge") {
                f(badge, false);
            }
            if let Some(Value::Array(tags)) = item.get_mut("tags") {
                for tag in tags {
                    if let Value::String(tag) = tag {
                        f(tag, false);
                    }
                }
            }
        }
    }
}

fn node_has_contact_variables(node: &Node) -> bool {
    let mut props = node.props.clone();
    let mut found = false;
    for_each_display_text(&node.kind, &mut props, &mut |text, _| {
        found |= has_contact_variable(text)
    });
    found
}

/// `mapDisplayText` in place: `f` gets every screen title and display text
/// and whether it is markdown.
pub fn map_display_text(definition: &mut Definition, f: &mut dyn FnMut(&mut String, bool)) {
    for screen in &mut definition.screens {
        f(&mut screen.title, false);
        super::walk_mut(&mut screen.children, &mut |node| {
            for_each_display_text(&node.kind, &mut node.props, f)
        });
    }
}

const WORD_JOINER: &str = "\u{2060}";

/// `escapeContactValue`: a value put into `text` always shows as itself, never
/// as a `${...}` reference, a `{{variable}}`, markdown, or the end of a quoted
/// string inside a nested expression.
pub fn escape_contact_value(value: &str, text: &str, markdown: bool) -> String {
    let mut escaped = value
        .replace("${", &format!("${WORD_JOINER}{{"))
        .replace("{{", &format!("{{{WORD_JOINER}{{"));
    let escape = |s: &str, specials: &str| {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if specials.contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    if markdown {
        escaped = escape(&escaped, "\\*_~[]()#-.`");
    }
    if super::expression::is_nested_expression(text) {
        escaped = escape(&escaped, "\\'\"");
    }
    escaped
}

// ----------------------------------------------------------- screens

fn screen_id_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z][A-Za-z_]*$").expect("regex"))
}

fn name_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("regex"))
}

fn screen_picker_files(screen: &Screen) -> usize {
    let mut total = 0;
    walk(&screen.children, &mut |v| {
        let prop = match v.node.kind.as_str() {
            "PhotoPicker" => "max-uploaded-photos",
            "DocumentPicker" => "max-uploaded-documents",
            _ => return,
        };
        total += match v.node.props.get(prop).and_then(Value::as_f64) {
            Some(max) => max as usize,
            None => MAX_UPLOADS_PER_INPUT,
        };
    });
    total
}

fn reachable_screens(definition: &Definition) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue: std::collections::VecDeque<String> = definition
        .screens
        .first()
        .map(|s| s.key.clone())
        .into_iter()
        .collect();
    while let Some(key) = queue.pop_front() {
        if !reachable.insert(key.clone()) {
            continue;
        }
        for action in find_screen(definition, &key)
            .map(screen_actions)
            .unwrap_or_default()
        {
            if let Action::Navigate(next) = action {
                queue.push_back(next);
            }
        }
    }
    reachable
}

fn check_screens(definition: &Definition, c: &mut Collector) {
    let mut seen = HashSet::new();
    for screen in &definition.screens {
        if has_contact_variable(&screen.title) {
            c.add(
                Some(screen),
                None,
                report("contact_variable_whatsapp").prop("title").warning(),
            );
        }
        if crate::js::utf16_len(&collapse_contact_variables(&screen.title))
            > SCREEN_TITLE_MAX_LENGTH
        {
            c.add(
                Some(screen),
                None,
                report("property_too_long")
                    .prop("title")
                    .param("max", json!(SCREEN_TITLE_MAX_LENGTH)),
            );
        }
        if screen.id == "SUCCESS" {
            c.add(Some(screen), None, report("screen_id_reserved").prop("id"));
        } else if !screen_id_pattern().is_match(&screen.id) {
            c.add(Some(screen), None, report("screen_id_invalid").prop("id"));
        }
        if !seen.insert(screen.id.as_str()) {
            c.add(Some(screen), None, report("screen_id_duplicate").prop("id"));
        }
    }
    if !definition.screens.iter().any(is_terminal_screen) {
        c.add(None, None, report("no_terminal_screen"));
    }
    for screen in definition.screens.iter().filter(|s| is_terminal_screen(s)) {
        let path = collect_path_screens(definition, &screen.key);
        let files: usize = definition
            .screens
            .iter()
            .filter(|s| path.contains(&s.key))
            .map(screen_picker_files)
            .sum();
        if files > WHATSAPP_MAX_FILES_PER_RESPONSE {
            c.add(
                Some(screen),
                None,
                report("picker_multiple_in_complete")
                    .param("max", json!(WHATSAPP_MAX_FILES_PER_RESPONSE)),
            );
        }
    }
    let reachable = reachable_screens(definition);
    for screen in &definition.screens {
        if screen_actions(screen)
            .iter()
            .all(|a| matches!(a, Action::OpenUrl(_)))
        {
            c.add(Some(screen), None, report("screen_without_exit"));
        }
        if !reachable.contains(&screen.key) {
            c.add(Some(screen), None, report("screen_unreachable").warning());
        }
    }
}

// --------------------------------------------------------- node props

/// `issueFromZod`.
fn issue_from_zod(issue: &Issue) -> Report {
    match issue.code {
        IssueCode::TooBig => {
            report("property_too_long").param("max", num(issue.maximum.unwrap_or(0.0)))
        }
        IssueCode::TooSmall => {
            let min = issue.minimum.unwrap_or(0.0);
            if min <= 1.0 && issue.origin.as_deref() == Some("string") {
                report("property_required")
            } else {
                report("property_too_short").param("min", num(min))
            }
        }
        IssueCode::Custom if issue.message == "duplicate_option_id" => {
            report("option_duplicate_id")
        }
        IssueCode::UnrecognizedKeys => {
            report("property_unknown").param("name", json!(issue.keys.join(", ")))
        }
        IssueCode::InvalidType if issue.message.ends_with("received undefined") => {
            report("property_required")
        }
        _ => report("property_invalid"),
    }
}

struct ScreenContext<'a> {
    screen: &'a Screen,
    screen_keys: HashSet<&'a str>,
    local_names: HashSet<String>,
    names_by_screen: HashMap<&'a str, HashSet<String>>,
}

fn check_action(
    value: Option<&Value>,
    property: String,
    allowed: &[&str],
    required: bool,
    ctx: &ScreenContext,
    out: &mut Vec<Report>,
) {
    if value.is_none_or(Value::is_null) {
        if required {
            out.push(report("action_required").prop(property));
        }
        return;
    }
    let action = read_action(value);
    let Some(action) = action.filter(|a| allowed.contains(&a.name())) else {
        out.push(report("action_not_allowed").prop(property));
        return;
    };
    if let Action::Navigate(next) = action {
        if !ctx.screen_keys.contains(next.as_str()) {
            out.push(report("navigate_target_missing").prop(property));
        } else if next == ctx.screen.key {
            out.push(report("navigate_self").prop(property));
        }
    }
}

fn for_each_string(value: &Value, path: &str, visit: &mut dyn FnMut(&str, &str)) {
    match value {
        Value::String(text) => visit(text, path),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                for_each_string(item, &format!("{path}.{i}"), visit);
            }
        }
        Value::Object(map) => {
            for (key, entry) in map {
                if ![ACTION_KEY, "src", "image"].contains(&key.as_str()) {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    for_each_string(entry, &child, visit);
                }
            }
        }
        _ => {}
    }
}

fn footer_text(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| !js_trim(s).is_empty())
}

fn count_with_text(items: Option<&Value>, key: &str) -> usize {
    items
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|i| footer_text(i.get(key))).count())
        .unwrap_or(0)
}

/// `checkNodeProps`.
fn check_node_props(node: &Node, ctx: &ScreenContext) -> Vec<Report> {
    let mut out = Vec::new();
    let Some(definition) = component(&node.kind) else {
        return out;
    };
    // Schema, with each `{{variable}}` counted as one character.
    let mut measured = node.props.clone();
    for_each_display_text(&node.kind, &mut measured, &mut |text, _| {
        *text = collapse_contact_variables(text)
    });
    measured.remove(ACTION_KEY);
    let parsed = definition
        .props
        .safe_parse(&Value::Object(clean_object(&measured)));
    for issue in parsed.issues() {
        out.push(issue_from_zod(issue).prop(issue.path_string()));
    }
    // Name.
    let name = node.props.get("name").and_then(Value::as_str);
    if needs_name(&node.kind) && !name.is_some_and(|n| name_pattern().is_match(n)) {
        out.push(report("name_invalid").prop("name"));
    }
    // Min / max pairs (numbers as numbers, ISO dates as strings).
    for (min_key, max_key) in [
        ("min-chars", "max-chars"),
        ("min-selected-items", "max-selected-items"),
        ("min-uploaded-photos", "max-uploaded-photos"),
        ("min-uploaded-documents", "max-uploaded-documents"),
        ("min-days", "max-days"),
        ("min-date", "max-date"),
    ] {
        let greater = match (node.props.get(min_key), node.props.get(max_key)) {
            (Some(Value::Number(a)), Some(Value::Number(b))) => a.as_f64() > b.as_f64(),
            (Some(Value::String(a)), Some(Value::String(b))) => {
                a.encode_utf16().collect::<Vec<_>>() > b.encode_utf16().collect::<Vec<_>>()
            }
            _ => false,
        };
        if greater {
            out.push(report("min_greater_than_max").prop(min_key));
        }
    }
    check_action(
        node.props.get(ACTION_KEY),
        ACTION_KEY.into(),
        definition.actions,
        definition.action_required,
        ctx,
        &mut out,
    );
    // Component rules.
    match node.kind.as_str() {
        "Footer" => {
            let left = footer_text(node.props.get("left-caption"));
            let right = footer_text(node.props.get("right-caption"));
            let center = footer_text(node.props.get("center-caption"));
            if (center && (left || right)) || left != right {
                out.push(report("footer_captions_invalid").prop("left-caption"));
            }
        }
        "Dropdown" => {
            let options = node.props.get("data-source");
            if options
                .and_then(Value::as_array)
                .is_some_and(|o| o.len() > MAX_DROPDOWN_OPTIONS_WITH_IMAGES)
                && count_with_text(options, "image") > 0
            {
                out.push(
                    report("options_with_images_limit")
                        .prop("data-source")
                        .param("max", json!(MAX_DROPDOWN_OPTIONS_WITH_IMAGES)),
                );
            }
        }
        "NavigationList" if count_with_text(node.props.get(LIST_ITEMS_KEY), "badge") > 1 => {
            out.push(report("navigation_badge_multiple").prop(LIST_ITEMS_KEY));
        }
        _ => {}
    }
    if node_has_contact_variables(node) {
        out.push(report("contact_variable_whatsapp").warning());
    }
    if node.kind == "NavigationList"
        && let Some(Value::Array(items)) = node.props.get(LIST_ITEMS_KEY)
    {
        for (i, item) in items.iter().enumerate() {
            let property = format!("{LIST_ITEMS_KEY}.{i}.{ACTION_KEY}");
            check_action(
                item.get(ACTION_KEY),
                property,
                &["navigate"],
                true,
                ctx,
                &mut out,
            );
        }
    }
    let expression_prop = match node.kind.as_str() {
        "If" => Some("condition"),
        "Switch" => Some("value"),
        _ => None,
    };
    if let Some(prop) = expression_prop
        && let Some(Value::String(expression)) = node.props.get(prop)
        && !js_trim(expression).is_empty()
        && !is_valid_expression(expression)
    {
        out.push(report("expression_invalid").prop(prop));
    }
    for_each_string(
        &Value::Object(node.props.clone()),
        "",
        &mut |text, property| {
            for (raw, reference) in extract_references(text) {
                let base = |code| report(code).prop(property).param("reference", json!(raw));
                match reference {
                    None => out.push(base("reference_invalid")),
                    Some(r) if r.source == "data" => out.push(base("data_reference_unsupported")),
                    Some(r) => {
                        let names = match &r.screen_id {
                            Some(id) => ctx.names_by_screen.get(id.as_str()),
                            None => Some(&ctx.local_names),
                        };
                        if !names.is_some_and(|n| n.contains(&r.name)) {
                            out.push(base("reference_unknown"));
                        }
                    }
                }
            }
        },
    );
    out
}

// ---------------------------------------------------------- structure

fn contains_footer(nodes: Option<&Vec<Node>>) -> bool {
    nodes.is_some_and(|n| n.iter().any(|n| n.kind == "Footer"))
}

fn slot<'a>(node: &'a Node, key: &str) -> Option<&'a Vec<Node>> {
    node.slots.as_ref().and_then(|s| s.get(key))
}

fn check_visit(visit: &Visit, screen: &Screen, out: &mut Vec<Report>) {
    let node = visit.node;
    if let Some(parent) = visit.parent
        && !can_place_in_slot(&parent.kind, &node.kind)
    {
        out.push(report("child_not_allowed").param("parent", json!(parent.kind)));
    }
    if node.kind == "If" && visit.if_depth + 1 > MAX_IF_DEPTH {
        out.push(report("if_too_deep").param("max", json!(MAX_IF_DEPTH)));
    }
    if node.kind == "If" && slot(node, "then").is_none_or(Vec::is_empty) {
        out.push(report("if_then_empty"));
    }
    if node.kind == "Switch" && node.slots.as_ref().is_none_or(|s| s.is_empty()) {
        out.push(report("switch_no_cases"));
    }
    if node.kind == "Footer" {
        let siblings = match visit.parent {
            Some(parent) => slot(parent, visit.slot.unwrap_or_default())
                .map(Vec::as_slice)
                .unwrap_or_default(),
            None => screen.children.as_slice(),
        };
        if visit.in_conditional {
            if visit.parent.is_none_or(|p| p.kind != "If") || visit.if_depth > 1 {
                out.push(report("footer_in_nested_branch"));
            }
        } else if siblings.last().is_none_or(|last| last.id != node.id) {
            out.push(report("footer_not_last"));
        }
    }
    if node.kind == "If"
        && node.slots.is_some()
        && contains_footer(slot(node, "then")) != contains_footer(slot(node, "else"))
    {
        out.push(report("footer_missing_in_branch"));
    }
}

fn count_footers(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| match node.kind.as_str() {
            "Footer" => 1,
            "Form" => count_footers(
                slot(node, "children")
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ),
            "If" => usize::from(contains_footer(slot(node, "then"))),
            _ => 0,
        })
        .sum()
}

fn check_structure(ctx: &ScreenContext, c: &mut Collector) {
    let screen = ctx.screen;
    let mut counts: Vec<(&str, usize)> = Vec::new();
    let mut id_by_name: HashMap<&str, &str> = HashMap::new();
    let mut visits = Vec::new();
    walk(&screen.children, &mut |v| visits.push(v));
    for visit in &visits {
        let node = visit.node;
        match counts.iter_mut().find(|(k, _)| *k == node.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((&node.kind, 1)),
        }
        let mut out = Vec::new();
        check_visit(visit, screen, &mut out);
        if let Some(name) = node_name(node) {
            if id_by_name.get(name).is_some_and(|owner| *owner != node.id) {
                out.push(
                    report("name_duplicate")
                        .prop("name")
                        .param("name", json!(name)),
                );
            }
            id_by_name.insert(name, &node.id);
        }
        out.extend(check_node_props(node, ctx));
        for r in out {
            c.add(Some(screen), Some(node), r);
        }
    }
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total > MAX_COMPONENTS_PER_SCREEN {
        c.add(
            Some(screen),
            None,
            report("too_many_components").param("max", json!(MAX_COMPONENTS_PER_SCREEN)),
        );
    }
    for (kind, count) in &counts {
        if let Some(max) = component(kind).and_then(|d| d.max_per_screen)
            && *count > max
            && *kind != "Footer"
        {
            c.add(
                Some(screen),
                None,
                report("component_limit")
                    .param("type", json!(kind))
                    .param("max", json!(max)),
            );
        }
    }
    let has = |kind: &str| counts.iter().any(|(k, _)| *k == kind);
    if has("PhotoPicker") && has("DocumentPicker") {
        c.add(Some(screen), None, report("picker_conflict"));
    }
    if has("NavigationList") && has("Footer") {
        c.add(Some(screen), None, report("navigation_list_with_footer"));
    }
    if has("NavigationList") && is_terminal_screen(screen) {
        c.add(Some(screen), None, report("navigation_list_on_terminal"));
    }
    if count_footers(&screen.children) > 1 {
        c.add(
            Some(screen),
            None,
            report("component_limit")
                .param("type", json!("Footer"))
                .param("max", json!(1)),
        );
    }
}

fn check_unique_input_names(definition: &Definition, c: &mut Collector) {
    let mut owner: HashMap<String, String> = HashMap::new();
    for screen in &definition.screens {
        let mut visits = Vec::new();
        walk(&screen.children, &mut |v| visits.push(v.node));
        for node in visits {
            let Some(name) = input_name(node) else {
                continue;
            };
            if owner.get(name).is_some_and(|k| *k != screen.key) {
                c.add(
                    Some(screen),
                    Some(node),
                    report("name_duplicate")
                        .prop("name")
                        .param("name", json!(name)),
                );
            }
            owner.insert(name.to_owned(), screen.key.clone());
        }
    }
}

/// `validateMiniApp`: errors block publishing; warnings do not.
pub fn validate(input: &Value) -> ValidationResult {
    let mut c = Collector::default();
    let Some(definition) = parse_definition(input) else {
        c.add(None, None, report("definition_invalid"));
        return ValidationResult {
            valid: false,
            issues: c.issues,
        };
    };
    check_screens(&definition, &mut c);
    let screen_keys: HashSet<&str> = definition.screens.iter().map(|s| s.key.as_str()).collect();
    let names_by_screen: HashMap<&str, HashSet<String>> = definition
        .screens
        .iter()
        .map(|s| (s.id.as_str(), collect_input_names(s).into_iter().collect()))
        .collect();
    for screen in &definition.screens {
        let ctx = ScreenContext {
            screen,
            screen_keys: screen_keys.clone(),
            local_names: names_by_screen
                .get(screen.id.as_str())
                .cloned()
                .unwrap_or_default(),
            names_by_screen: names_by_screen.clone(),
        };
        check_structure(&ctx, &mut c);
    }
    check_unique_input_names(&definition, &mut c);
    let valid = !c.issues.iter().any(|i| i.severity == "error");
    ValidationResult {
        valid,
        issues: c.issues,
    }
}

/// `MiniAppLocatedIssue` as JSON.
pub type LocatedIssue = Value;

fn index_paths(nodes: &[Node], base: &str, paths: &mut HashMap<String, String>) {
    for (i, node) in nodes.iter().enumerate() {
        let path = format!("{base}[{i}]");
        paths.insert(node.id.clone(), path.clone());
        for (key, children) in slot_entries(node) {
            let slot = if node.kind == "Switch" {
                format!("cases.{key}")
            } else {
                key.to_owned()
            };
            index_paths(children, &format!("{path}.{slot}"), paths);
        }
    }
}

/// `locateIssues`: issues addressed by Flow JSON screen id and path. An
/// unparseable definition has no screens to locate.
pub fn locate_issues(definition: &Value, issues: &[ValidationIssue]) -> Vec<LocatedIssue> {
    let mut screen_ids = HashMap::new();
    let mut paths = HashMap::new();
    let screens = parse_definition(definition)
        .map(|d| d.screens)
        .unwrap_or_default();
    for (i, screen) in screens.iter().enumerate() {
        screen_ids.insert(screen.key.clone(), screen.id.clone());
        index_paths(
            &screen.children,
            &format!("screens[{i}].layout.children"),
            &mut paths,
        );
    }
    issues
        .iter()
        .map(|issue| {
            let mut out = Map::new();
            out.insert("code".into(), json!(issue.code));
            out.insert("severity".into(), json!(issue.severity));
            if let Some(id) = issue.screen_key.as_ref().and_then(|k| screen_ids.get(k)) {
                out.insert("screenId".into(), json!(id));
            }
            if let Some(path) = issue.node_id.as_ref().and_then(|id| paths.get(id)) {
                out.insert("path".into(), json!(path));
            }
            if let Some(property) = issue.property.as_ref().filter(|p| !p.is_empty()) {
                out.insert("property".into(), json!(property));
            }
            if let Some(params) = &issue.params {
                out.insert("params".into(), Value::Object(params.clone()));
            }
            Value::Object(out)
        })
        .collect()
}
