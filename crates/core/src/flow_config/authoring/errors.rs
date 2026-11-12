//! Port of `packages/flow-config/src/authoring/errors.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn flow_capability_schema = z::discriminated_union("code", vec![z::object(vec![("block", z::string()),
("channel", z::string()),
("code", z::literal(json!("unsupportedBlock"))),
("policyVersion", z::number())]),
z::object(vec![("actual", z::number().optional()),
("allowed", z::number().optional()),
("block", z::string()),
("channel", z::string()),
("code", z::literal(json!("constraintExceeded"))),
("constraintId", z::string().optional()),
("policyVersion", z::number()),
("unit", z::string().optional())])]));

/// `FlowAuthoringError`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowAuthoringError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Value>,
    pub path: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<String>>,
}

/// `FlowAuthoringException`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowAuthoringException {
    pub errors: Vec<FlowAuthoringError>,
}

impl FlowAuthoringException {
    pub fn new(errors: Vec<FlowAuthoringError>) -> FlowAuthoringException {
        FlowAuthoringException { errors }
    }
}

fn format_flow_authoring_error_message(error: &FlowAuthoringError) -> String {
    let Some(capability) = &error.capability else {
        return error.message.clone();
    };
    let (block, channel) = (
        str_field(capability, "block"),
        str_field(capability, "channel"),
    );
    if capability["code"] == "unsupportedBlock" {
        return format!("The {channel} channel does not support {block}.");
    }
    let number = |v: &Value| crate::js::number_to_string(v.as_f64().unwrap_or(0.0));
    if let (Some(actual), Some(allowed)) = (capability.get("actual"), capability.get("allowed"))
        && truthy(capability.get("unit"))
    {
        return format!(
            "{block} exceeds the {channel} maximum of {} {} (received {}).",
            number(allowed),
            str_field(capability, "unit"),
            number(actual)
        );
    }
    format!("{block} violates a {channel} channel constraint.")
}

impl FlowAuthoringException {
    /// `error.message`: every error, `path: message`, joined by `; `.
    pub fn message(&self) -> String {
        if self.errors.is_empty() {
            return "Flow authoring failed".to_owned();
        }
        self.errors
            .iter()
            .map(|e| format!("{}: {}", e.path, format_flow_authoring_error_message(e)))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl std::fmt::Display for FlowAuthoringException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for FlowAuthoringException {}

/// Maps an issue path to an authoring path (`mapPath`).
pub type PathMapper<'a> = &'a dyn Fn(&[PathSegment]) -> Option<String>;

/// `formatZodPathSegment(acc, segment)`.
pub fn format_zod_path_segment(acc: &str, segment: &PathSegment) -> String {
    match segment {
        PathSegment::Index(i) => format!("{acc}[{i}]"),
        PathSegment::Key(key) if acc.is_empty() => key.clone(),
        PathSegment::Key(key) => format!("{acc}.{key}"),
    }
}

/// `zodErrorToFlowAuthoringErrors(error, code, mapPath)`.
pub fn zod_error_to_flow_authoring_errors(
    error: &z::ZodError,
    code: &str,
    map_path: Option<PathMapper<'_>>,
) -> Vec<FlowAuthoringError> {
    error
        .issues
        .iter()
        .map(|issue| {
            let capability = issue
                .params
                .as_ref()
                .and_then(|p| p.get("capability"))
                .filter(|c| flow_capability_schema().safe_parse(c).success)
                .cloned();
            let path = map_path.and_then(|m| m(&issue.path)).unwrap_or_else(|| {
                issue
                    .path
                    .iter()
                    .fold(String::new(), |acc, s| format_zod_path_segment(&acc, s))
            });
            FlowAuthoringError {
                code: capability
                    .as_ref()
                    .map_or_else(|| code.to_owned(), |c| str_field(c, "code").to_owned()),
                capability,
                path,
                message: issue.message.clone(),
                ..FlowAuthoringError::default()
            }
        })
        .collect()
}

fn name_similarity(target: &str, candidate: &str) -> f64 {
    if target == candidate {
        return f64::INFINITY;
    }
    let (tl, cl) = (
        crate::js::utf16_len(target) as f64,
        crate::js::utf16_len(candidate) as f64,
    );
    if candidate.starts_with(target) || target.starts_with(candidate) {
        return 1000.0 - (cl - tl).abs();
    }
    if candidate.contains(target) || target.contains(candidate) {
        return 500.0 - (cl - tl).abs();
    }
    let distance = crate::utils::levenshtein::levenshtein_distance(target, candidate) as f64;
    let max_length = tl.max(cl);
    let allowed = 2f64.max((max_length * 0.34).ceil());
    if distance <= allowed {
        max_length - distance
    } else {
        0.0
    }
}

/// `closestNames(target, available)`: up to three similar names, best first.
pub fn closest_names(target: &str, available: &[&str]) -> Vec<String> {
    let target_lower = crate::js::to_lower(target);
    let mut scored: Vec<(&str, f64)> = available
        .iter()
        .map(|name| {
            (
                *name,
                name_similarity(&target_lower, &crate::js::to_lower(name)),
            )
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(3)
        .map(|(n, _)| n.to_owned())
        .collect()
}
