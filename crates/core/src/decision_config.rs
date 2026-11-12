//! Port of `packages/decision-config` (`profile.ts`): the Decision profile
//! form and the Global Router configuration, validated from JSON with the
//! same rules (and issue messages) as the zod schemas.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

/// One validation issue: where, and what (the zod message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub path: Vec<PathSegment>,
    pub message: String,
}

/// A step of an issue path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Default)]
struct Ctx {
    path: Vec<PathSegment>,
    issues: Vec<Issue>,
}

impl Ctx {
    fn issue(&mut self, extra: &[PathSegment], message: impl Into<String>) {
        let mut path = self.path.clone();
        path.extend_from_slice(extra);
        self.issues.push(Issue {
            path,
            message: message.into(),
        });
    }

    fn at<T>(&mut self, segment: PathSegment, f: impl FnOnce(&mut Ctx) -> T) -> T {
        self.path.push(segment);
        let out = f(self);
        self.path.pop();
        out
    }

    fn key<T>(&mut self, key: &str, f: impl FnOnce(&mut Ctx) -> T) -> T {
        self.at(PathSegment::Key(key.to_owned()), f)
    }
}

fn key(name: &str) -> PathSegment {
    PathSegment::Key(name.to_owned())
}

fn strict_object<'a>(
    ctx: &mut Ctx,
    value: &'a Value,
    keys: &[&str],
) -> Option<&'a Map<String, Value>> {
    let Some(map) = value.as_object() else {
        ctx.issue(&[], "Invalid input: expected object");
        return None;
    };
    let unknown: Vec<&String> = map.keys().filter(|k| !keys.contains(&k.as_str())).collect();
    if !unknown.is_empty() {
        let names: Vec<String> = unknown.iter().map(|k| format!("\"{k}\"")).collect();
        ctx.issue(
            &[],
            format!(
                "Unrecognized key{}: {}",
                if names.len() > 1 { "s" } else { "" },
                names.join(", ")
            ),
        );
    }
    Some(map)
}

fn js_len(text: &str) -> usize {
    crate::js::utf16_len(text)
}

/// `z.string().trim().min(min).max(max)` (optionally `.regex`).
fn trimmed_string(
    ctx: &mut Ctx,
    value: Option<&Value>,
    min: usize,
    max: usize,
    pattern: Option<&Regex>,
) -> Option<String> {
    let Some(Value::String(raw)) = value else {
        ctx.issue(&[], "Invalid input: expected string");
        return None;
    };
    let trimmed = crate::js::trim(raw).to_owned();
    let len = js_len(&trimmed);
    let mut ok = true;
    if len < min {
        ctx.issue(
            &[],
            format!("Too small: expected string to have >={min} characters"),
        );
        ok = false;
    }
    if len > max {
        ctx.issue(
            &[],
            format!("Too big: expected string to have <={max} characters"),
        );
        ok = false;
    }
    if let Some(re) = pattern {
        if !re.is_match(&trimmed) {
            ctx.issue(&[], "Invalid string: must match pattern");
            ok = false;
        }
    }
    ok.then_some(trimmed)
}

fn stable_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[a-z][a-z0-9_]*$").expect("valid regex"))
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d+$").expect("valid regex"))
}

/// `decisionStableKeySchema`.
fn stable_key(ctx: &mut Ctx, value: Option<&Value>) -> Option<String> {
    trimmed_string(ctx, value, 1, 64, Some(stable_key_re()))
}

/// `idSchema`: a decimal string (not trimmed).
fn id(ctx: &mut Ctx, value: Option<&Value>) -> Option<String> {
    let Some(Value::String(raw)) = value else {
        ctx.issue(&[], "Invalid input: expected string");
        return None;
    };
    if id_re().is_match(raw) {
        Some(raw.clone())
    } else {
        ctx.issue(&[], "Invalid string: must match pattern /^\\d+$/");
        None
    }
}

fn number_in(ctx: &mut Ctx, value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    let Some(n) = value.and_then(Value::as_f64) else {
        ctx.issue(&[], "Invalid input: expected number");
        return None;
    };
    if n < min {
        ctx.issue(
            &[],
            format!(
                "Too small: expected number to be >={}",
                crate::js::number_to_string(min)
            ),
        );
        return None;
    }
    if n > max {
        ctx.issue(
            &[],
            format!(
                "Too big: expected number to be <={}",
                crate::js::number_to_string(max)
            ),
        );
        return None;
    }
    Some(n)
}

fn int_in(ctx: &mut Ctx, value: Option<&Value>, min: f64, max: f64) -> Option<i64> {
    let n = number_in(ctx, value, min, max)?;
    if n.fract() != 0.0 {
        ctx.issue(&[], "Invalid input: expected int, received number");
        return None;
    }
    Some(n as i64)
}

fn literal_tag<'a>(
    ctx: &mut Ctx,
    map: &'a Map<String, Value>,
    tag: &str,
    options: &[&str],
) -> Option<&'a str> {
    match map.get(tag).and_then(Value::as_str) {
        Some(found) if options.contains(&found) => Some(found),
        _ => {
            ctx.issue(&[key(tag)], "Invalid input");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// `globalRouterDestinationSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GlobalRouterDestination {
    Flow { id: String },
    AiAgent { id: String },
    User { id: String },
    Team { id: String },
}

/// `globalRouterRouteSchema`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GlobalRouterRoute {
    Choice {
        destination: GlobalRouterDestination,
        value: String,
    },
    Score {
        destination: GlobalRouterDestination,
        from: i64,
        to: i64,
    },
    Noul {
        destination: GlobalRouterDestination,
        value: bool,
    },
}

/// `globalRouterMetadataMappingSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalRouterMetadataMapping {
    pub custom_field_id: String,
    /// `shortText` / `longText` for `choice`; `number` otherwise.
    pub custom_field_type: String,
    /// `choice`, `confidence`, `selectedProbability`, `score` or
    /// `probabilityTrue`.
    pub value: String,
}

/// The Noul probability bands.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoulBands {
    pub false_maximum: f64,
    pub true_minimum: f64,
}

/// `globalRouterConfigSchema` (defaults applied).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalRouterConfig {
    pub fallback: GlobalRouterDestination,
    pub metadata_mappings: Vec<GlobalRouterMetadataMapping>,
    pub noul: NoulBands,
    pub routes: Vec<GlobalRouterRoute>,
    pub version: i64,
}

/// `decisionProfileThresholdConfigSchema`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionProfileThresholdConfig {
    ChoiceConfidence { minimum: f64 },
    Score { operator: String, value: f64 },
    NoulTrueProbability { minimum: f64 },
}

/// One option of a `choice` decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceOption {
    pub description: String,
    pub value: String,
}

/// The decision of a profile form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DecisionForm {
    Choice {
        instructions: String,
        options: Vec<ChoiceOption>,
    },
    Score {
        instructions: String,
        levels: Vec<String>,
    },
    Noul {
        #[serde(rename = "falseCriteria")]
        false_criteria: String,
        instructions: String,
        #[serde(rename = "trueCriteria")]
        true_criteria: String,
    },
}

/// `decisionProfileFormSchema` (defaults applied).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionProfileForm {
    pub connection_id: String,
    pub decision: DecisionForm,
    pub description: Option<String>,
    pub global_router_config: Option<GlobalRouterConfig>,
    pub global_router_enabled: bool,
    pub model: String,
    pub name: String,
    pub status: String,
    pub threshold_config: Option<DecisionProfileThresholdConfig>,
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn destination(ctx: &mut Ctx, value: Option<&Value>) -> Option<GlobalRouterDestination> {
    let value = value.unwrap_or(&Value::Null);
    let map = strict_object(ctx, value, &["id", "type"])?;
    let tag = literal_tag(ctx, map, "type", &["flow", "aiAgent", "user", "team"])?;
    let id = ctx.key("id", |c| id(c, map.get("id")))?;
    Some(match tag {
        "flow" => GlobalRouterDestination::Flow { id },
        "aiAgent" => GlobalRouterDestination::AiAgent { id },
        "user" => GlobalRouterDestination::User { id },
        _ => GlobalRouterDestination::Team { id },
    })
}

fn route(ctx: &mut Ctx, value: &Value) -> Option<GlobalRouterRoute> {
    let map = value.as_object();
    let Some(map) = map else {
        ctx.issue(&[], "Invalid input: expected object");
        return None;
    };
    let tag = literal_tag(ctx, map, "type", &["choice", "score", "noul"])?;
    match tag {
        "choice" => {
            strict_object(ctx, value, &["destination", "type", "value"])?;
            let destination = ctx.key("destination", |c| destination(c, map.get("destination")));
            let value = ctx.key("value", |c| stable_key(c, map.get("value")));
            Some(GlobalRouterRoute::Choice {
                destination: destination?,
                value: value?,
            })
        }
        "score" => {
            strict_object(ctx, value, &["destination", "from", "to", "type"])?;
            let destination = ctx.key("destination", |c| destination(c, map.get("destination")));
            let from = ctx.key("from", |c| int_in(c, map.get("from"), 1.0, 10.0));
            let to = ctx.key("to", |c| int_in(c, map.get("to"), 1.0, 10.0));
            let (destination, from, to) = (destination?, from?, to?);
            if from > to {
                ctx.issue(&[key("from")], "Score route start must not exceed its end");
                return None;
            }
            Some(GlobalRouterRoute::Score {
                destination,
                from,
                to,
            })
        }
        _ => {
            strict_object(ctx, value, &["destination", "type", "value"])?;
            let destination = ctx.key("destination", |c| destination(c, map.get("destination")));
            let flag = match map.get("value") {
                Some(Value::Bool(b)) => Some(*b),
                _ => {
                    ctx.issue(&[key("value")], "Invalid input: expected boolean");
                    None
                }
            };
            Some(GlobalRouterRoute::Noul {
                destination: destination?,
                value: flag?,
            })
        }
    }
}

fn metadata_mapping(ctx: &mut Ctx, value: &Value) -> Option<GlobalRouterMetadataMapping> {
    let map = strict_object(ctx, value, &["customFieldId", "customFieldType", "value"])?;
    let tag = literal_tag(
        ctx,
        map,
        "value",
        &[
            "choice",
            "confidence",
            "selectedProbability",
            "score",
            "probabilityTrue",
        ],
    )?;
    let custom_field_id = ctx.key("customFieldId", |c| id(c, map.get("customFieldId")));
    let allowed: &[&str] = if tag == "choice" {
        &["shortText", "longText"]
    } else {
        &["number"]
    };
    let custom_field_type = match map.get("customFieldType").and_then(Value::as_str) {
        Some(t) if allowed.contains(&t) => Some(t.to_owned()),
        _ => {
            ctx.issue(&[key("customFieldType")], "Invalid option");
            None
        }
    };
    Some(GlobalRouterMetadataMapping {
        custom_field_id: custom_field_id?,
        custom_field_type: custom_field_type?,
        value: tag.to_owned(),
    })
}

fn array_of<T>(
    ctx: &mut Ctx,
    value: Option<&Value>,
    max: usize,
    min: usize,
    mut item: impl FnMut(&mut Ctx, &Value) -> Option<T>,
) -> Option<Vec<T>> {
    let Some(Value::Array(items)) = value else {
        ctx.issue(&[], "Invalid input: expected array");
        return None;
    };
    let mut ok = true;
    if items.len() > max {
        ctx.issue(
            &[],
            format!("Too big: expected array to have <={max} items"),
        );
        ok = false;
    }
    if items.len() < min {
        ctx.issue(
            &[],
            format!("Too small: expected array to have >={min} items"),
        );
        ok = false;
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, entry) in items.iter().enumerate() {
        match ctx.at(PathSegment::Index(index), |c| item(c, entry)) {
            Some(v) => out.push(v),
            None => ok = false,
        }
    }
    ok.then_some(out)
}

fn global_router_config_in(ctx: &mut Ctx, value: &Value) -> Option<GlobalRouterConfig> {
    let map = strict_object(
        ctx,
        value,
        &["fallback", "metadataMappings", "noul", "routes", "version"],
    )?;
    let start = ctx.issues.len();
    let fallback = ctx.key("fallback", |c| destination(c, map.get("fallback")));
    let metadata_mappings = match map.get("metadataMappings") {
        None => Some(Vec::new()),
        some => ctx.key("metadataMappings", |c| {
            array_of(c, some, 60, 0, metadata_mapping)
        }),
    };
    let noul = match map.get("noul") {
        None => Some(NoulBands {
            false_maximum: 0.5,
            true_minimum: 0.5,
        }),
        Some(noul) => ctx.key("noul", |c| {
            let m = strict_object(c, noul, &["falseMaximum", "trueMinimum"])?;
            let false_maximum = match m.get("falseMaximum") {
                None => Some(0.5),
                some => c.key("falseMaximum", |c| number_in(c, some, 0.0, 1.0)),
            };
            let true_minimum = match m.get("trueMinimum") {
                None => Some(0.5),
                some => c.key("trueMinimum", |c| number_in(c, some, 0.0, 1.0)),
            };
            Some(NoulBands {
                false_maximum: false_maximum?,
                true_minimum: true_minimum?,
            })
        }),
    };
    let routes = match map.get("routes") {
        None => Some(Vec::new()),
        some => ctx.key("routes", |c| array_of(c, some, 60, 0, route)),
    };
    let version = match map.get("version") {
        None => Some(1),
        Some(v) if v.as_f64() == Some(1.0) => Some(1),
        Some(_) => {
            ctx.issue(&[key("version")], "Invalid input: expected 1");
            None
        }
    };
    if ctx.issues.len() > start {
        return None;
    }
    let config = GlobalRouterConfig {
        fallback: fallback?,
        metadata_mappings: metadata_mappings?,
        noul: noul?,
        routes: routes?,
        version: version?,
    };
    refine_global_router_config(ctx, &config);
    (ctx.issues.len() == start).then_some(config)
}

fn refine_global_router_config(ctx: &mut Ctx, config: &GlobalRouterConfig) {
    if config.noul.false_maximum > config.noul.true_minimum {
        ctx.issue(
            &[key("noul"), key("falseMaximum")],
            "Noul false maximum must not exceed true minimum",
        );
    }
    let ids: HashSet<&str> = config
        .metadata_mappings
        .iter()
        .map(|m| m.custom_field_id.as_str())
        .collect();
    if ids.len() != config.metadata_mappings.len() {
        ctx.issue(
            &[key("metadataMappings")],
            "A metadata custom field can only be mapped once",
        );
    }
    let mut route_keys = HashSet::new();
    for (index, route) in config.routes.iter().enumerate() {
        let (route_key, kind) = match route {
            GlobalRouterRoute::Choice { value, .. } => (Some(format!("choice:{value}")), "choice"),
            GlobalRouterRoute::Noul { value, .. } => (Some(format!("noul:{value}")), "noul"),
            GlobalRouterRoute::Score { .. } => (None, "score"),
        };
        if let Some(route_key) = route_key {
            if route_keys.contains(&route_key) {
                ctx.issue(
                    &[key("routes"), PathSegment::Index(index)],
                    format!("A {kind} result can only have one route"),
                );
            }
            route_keys.insert(route_key);
        }
    }
    let mut metadata_values = HashSet::new();
    for (index, mapping) in config.metadata_mappings.iter().enumerate() {
        if metadata_values.contains(&mapping.value) {
            ctx.issue(
                &[key("metadataMappings"), PathSegment::Index(index)],
                "A result value can only be mapped once",
            );
        }
        metadata_values.insert(mapping.value.clone());
    }
    let score_routes: Vec<(i64, i64)> = config
        .routes
        .iter()
        .filter_map(|r| match r {
            GlobalRouterRoute::Score { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .collect();
    for (index, (from, to)) in score_routes.iter().enumerate() {
        let overlaps =
            score_routes
                .iter()
                .enumerate()
                .any(|(other_index, (other_from, other_to))| {
                    other_index != index && from <= other_to && other_from <= to
                });
        if overlaps {
            ctx.issue(&[key("routes")], "Score routes cannot overlap");
            break;
        }
    }
}

/// `globalRouterConfigSchema.safeParse`.
pub fn parse_global_router_config(value: &Value) -> Result<GlobalRouterConfig, Vec<Issue>> {
    let mut ctx = Ctx::default();
    match global_router_config_in(&mut ctx, value) {
        Some(config) if ctx.issues.is_empty() => Ok(config),
        _ => Err(ctx.issues),
    }
}

/// `globalRouterDestinationSchema.safeParse`.
pub fn parse_global_router_destination(
    value: &Value,
) -> Result<GlobalRouterDestination, Vec<Issue>> {
    let mut ctx = Ctx::default();
    match destination(&mut ctx, Some(value)) {
        Some(d) if ctx.issues.is_empty() => Ok(d),
        _ => Err(ctx.issues),
    }
}

fn threshold(ctx: &mut Ctx, value: &Value) -> Option<DecisionProfileThresholdConfig> {
    let Some(map) = value.as_object() else {
        ctx.issue(&[], "Invalid input: expected object");
        return None;
    };
    let tag = literal_tag(
        ctx,
        map,
        "type",
        &["choice_confidence", "score", "noul_true_probability"],
    )?;
    match tag {
        "score" => {
            strict_object(ctx, value, &["operator", "type", "value"])?;
            let operator = match map.get("operator").and_then(Value::as_str) {
                Some(op) if ["gt", "gte", "lt", "lte"].contains(&op) => Some(op.to_owned()),
                _ => {
                    ctx.issue(&[key("operator")], "Invalid option");
                    None
                }
            };
            let number = ctx.key("value", |c| {
                number_in(c, map.get("value"), f64::MIN, f64::MAX)
            });
            Some(DecisionProfileThresholdConfig::Score {
                operator: operator?,
                value: number?,
            })
        }
        other => {
            strict_object(ctx, value, &["minimum", "type"])?;
            let minimum = ctx.key("minimum", |c| number_in(c, map.get("minimum"), 0.0, 1.0))?;
            Some(if other == "choice_confidence" {
                DecisionProfileThresholdConfig::ChoiceConfidence { minimum }
            } else {
                DecisionProfileThresholdConfig::NoulTrueProbability { minimum }
            })
        }
    }
}

/// `decisionProfileThresholdConfigSchema.safeParse`.
pub fn parse_threshold_config(value: &Value) -> Result<DecisionProfileThresholdConfig, Vec<Issue>> {
    let mut ctx = Ctx::default();
    match threshold(&mut ctx, value) {
        Some(t) if ctx.issues.is_empty() => Ok(t),
        _ => Err(ctx.issues),
    }
}

/// `compileNoulInstructions`.
pub fn compile_noul_instructions(
    instructions: &str,
    true_criteria: &str,
    false_criteria: &str,
) -> String {
    format!("{instructions}\n\nTRUE when:\n{true_criteria}\n\nFALSE when:\n{false_criteria}")
}

fn decision(ctx: &mut Ctx, value: &Value) -> Option<DecisionForm> {
    let Some(map) = value.as_object() else {
        ctx.issue(&[], "Invalid input: expected object");
        return None;
    };
    let tag = literal_tag(ctx, map, "type", &["choice", "score", "noul"])?;
    match tag {
        "choice" => {
            strict_object(ctx, value, &["instructions", "options", "type"])?;
            let instructions = ctx.key("instructions", |c| {
                trimmed_string(c, map.get("instructions"), 1, 1000, None)
            });
            let options = ctx.key("options", |c| {
                array_of(c, map.get("options"), 24, 2, |c, option| {
                    let m = strict_object(c, option, &["description", "value"])?;
                    let description = c.key("description", |c| {
                        trimmed_string(c, m.get("description"), 1, 400, None)
                    });
                    let value = c.key("value", |c| stable_key(c, m.get("value")));
                    Some(ChoiceOption {
                        description: description?,
                        value: value?,
                    })
                })
            });
            let (instructions, options) = (instructions?, options?);
            let unique: HashSet<&str> = options.iter().map(|o| o.value.as_str()).collect();
            if unique.len() != options.len() {
                ctx.issue(&[key("options")], "Choice option values must be unique");
                return None;
            }
            Some(DecisionForm::Choice {
                instructions,
                options,
            })
        }
        "score" => {
            strict_object(ctx, value, &["instructions", "levels", "type"])?;
            let instructions = ctx.key("instructions", |c| {
                trimmed_string(c, map.get("instructions"), 1, 1000, None)
            });
            let levels = ctx.key("levels", |c| {
                array_of(c, map.get("levels"), 10, 2, |c, level| {
                    let m = strict_object(c, level, &["description"])?;
                    c.key("description", |c| {
                        trimmed_string(c, m.get("description"), 1, 400, None)
                    })
                })
            });
            Some(DecisionForm::Score {
                instructions: instructions?,
                levels: levels?,
            })
        }
        _ => {
            strict_object(
                ctx,
                value,
                &["falseCriteria", "instructions", "trueCriteria", "type"],
            )?;
            let false_criteria = ctx.key("falseCriteria", |c| {
                trimmed_string(c, map.get("falseCriteria"), 1, 1000, None)
            });
            let instructions = ctx.key("instructions", |c| {
                trimmed_string(c, map.get("instructions"), 1, 1000, None)
            });
            let true_criteria = ctx.key("trueCriteria", |c| {
                trimmed_string(c, map.get("trueCriteria"), 1, 1000, None)
            });
            Some(DecisionForm::Noul {
                false_criteria: false_criteria?,
                instructions: instructions?,
                true_criteria: true_criteria?,
            })
        }
    }
}

/// `decisionProfileFormSchema.safeParse`.
pub fn parse_decision_profile_form(value: &Value) -> Result<DecisionProfileForm, Vec<Issue>> {
    let mut ctx = Ctx::default();
    let form = decision_profile_form_in(&mut ctx, value);
    match form {
        Some(form) if ctx.issues.is_empty() => Ok(form),
        _ => Err(ctx.issues),
    }
}

fn decision_profile_form_in(ctx: &mut Ctx, value: &Value) -> Option<DecisionProfileForm> {
    let map = strict_object(
        ctx,
        value,
        &[
            "connectionId",
            "decision",
            "description",
            "globalRouterConfig",
            "globalRouterEnabled",
            "model",
            "name",
            "status",
            "thresholdConfig",
        ],
    )?;
    let connection_id = ctx.key("connectionId", |c| id(c, map.get("connectionId")));
    let decision = ctx.key("decision", |c| {
        decision(c, map.get("decision").unwrap_or(&Value::Null))
    });
    let description = match map.get("description") {
        None | Some(Value::Null) => Some(None),
        some => ctx
            .key("description", |c| trimmed_string(c, some, 0, 2000, None))
            .map(Some),
    };
    let global_router_config = match map.get("globalRouterConfig") {
        None | Some(Value::Null) => Some(None),
        Some(v) => ctx
            .key("globalRouterConfig", |c| global_router_config_in(c, v))
            .map(Some),
    };
    let global_router_enabled = match map.get("globalRouterEnabled") {
        None => Some(false),
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            ctx.issue(
                &[key("globalRouterEnabled")],
                "Invalid input: expected boolean",
            );
            None
        }
    };
    let model = ctx.key("model", |c| {
        trimmed_string(c, map.get("model"), 1, 256, None)
    });
    let name = ctx.key("name", |c| trimmed_string(c, map.get("name"), 1, 160, None));
    let status = match map.get("status") {
        None => Some("enabled".to_owned()),
        Some(Value::String(s)) if s == "enabled" || s == "disabled" => Some(s.clone()),
        Some(_) => {
            ctx.issue(
                &[key("status")],
                "Invalid option: expected one of \"enabled\"|\"disabled\"",
            );
            None
        }
    };
    let threshold_config = match map.get("thresholdConfig") {
        None | Some(Value::Null) => Some(None),
        Some(v) => ctx.key("thresholdConfig", |c| threshold(c, v)).map(Some),
    };
    let form = DecisionProfileForm {
        connection_id: connection_id?,
        decision: decision?,
        description: description?,
        global_router_config: global_router_config?,
        global_router_enabled: global_router_enabled?,
        model: model?,
        name: name?,
        status: status?,
        threshold_config: threshold_config?,
    };
    if !ctx.issues.is_empty() {
        return None;
    }
    if let DecisionForm::Noul {
        false_criteria,
        instructions,
        true_criteria,
    } = &form.decision
    {
        if js_len(&compile_noul_instructions(
            instructions,
            true_criteria,
            false_criteria,
        )) > 1000
        {
            ctx.issue(
                &[key("decision")],
                "Noul instructions exceed the provider limit",
            );
        }
    }
    if form.global_router_enabled && form.global_router_config.is_none() {
        ctx.issue(
            &[key("globalRouterConfig")],
            "Global Router fallback configuration is required",
        );
    }
    if let Some(threshold) = &form.threshold_config {
        let valid = match (&form.decision, threshold) {
            (
                DecisionForm::Choice { .. },
                DecisionProfileThresholdConfig::ChoiceConfidence { .. },
            ) => true,
            (
                DecisionForm::Noul { .. },
                DecisionProfileThresholdConfig::NoulTrueProbability { .. },
            ) => true,
            (
                DecisionForm::Score { levels, .. },
                DecisionProfileThresholdConfig::Score { value, .. },
            ) => *value >= 1.0 && *value <= levels.len() as f64,
            _ => false,
        };
        if !valid {
            ctx.issue(
                &[key("thresholdConfig")],
                "Threshold does not match the Decision configuration",
            );
        }
    }
    Some(form)
}
