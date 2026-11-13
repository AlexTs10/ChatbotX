//! Port of `partials/keyword-condition-schema.ts`. RE2 (`re2js`) is the Rust `regex`
//! crate: linear time, no backreferences or look-around.

use super::automated_response::{
    KeywordCondition, KeywordConditionOperator, MAX_KEYWORD_CONDITION_VALUES,
    MAX_KEYWORD_CONDITIONS, MAX_KEYWORD_REGEX_LENGTH, is_valueless_keyword_condition_operator,
};
use super::shared::{Issues, ZodError, ZodIssue, deserialize, js_length};

/// The longest keyword value (`z.string().max(255)`).
const MAX_KEYWORD_VALUE_LENGTH: usize = 255;

/// `compileKeywordRegex`: a case-insensitive regex, `None` for an over-long or invalid
/// pattern.
pub fn compile_keyword_regex(pattern: &str) -> Option<regex::Regex> {
    if js_length(pattern) > MAX_KEYWORD_REGEX_LENGTH {
        return None;
    }
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

/// `keywordConditionSchema.safeParse`.
pub fn parse_keyword_condition(value: &serde_json::Value) -> Result<KeywordCondition, ZodError> {
    let mut condition: KeywordCondition = deserialize(value)?;
    let mut issues = Issues::default();
    for (index, text) in condition.values.iter_mut().enumerate() {
        *text = text.trim().to_owned();
        let length = js_length(text);
        if length < 1 {
            issues.add(
                format!("values.{index}"),
                "Too small: expected string to have >=1 characters",
            );
        } else if length > MAX_KEYWORD_VALUE_LENGTH {
            issues.add(
                format!("values.{index}"),
                "Too big: expected string to have <=255 characters",
            );
        }
    }
    if condition.values.len() > MAX_KEYWORD_CONDITION_VALUES {
        issues.add(
            "values",
            format!("Too big: expected array to have <={MAX_KEYWORD_CONDITION_VALUES} items"),
        );
    }
    // zod runs the refinement only on an input without issues.
    if !issues.0.is_empty() {
        return issues.finish(condition);
    }
    if is_valueless_keyword_condition_operator(condition.operator) {
        if !condition.values.is_empty() {
            issues.add("values", "keywordConditionTakesNoValues");
        }
        return issues.finish(condition);
    }
    if condition.values.is_empty() {
        issues.add("values", "keywordConditionValuesRequired");
        return issues.finish(condition);
    }
    if condition.operator == KeywordConditionOperator::MatchesRegex {
        for (index, pattern) in condition.values.iter().enumerate() {
            if compile_keyword_regex(pattern).is_none() {
                issues.add(format!("values.{index}"), "invalidKeywordRegex");
            }
        }
    }
    issues.finish(condition)
}

/// `keywordConditionsSchema.safeParse` (1 to `MAX_KEYWORD_CONDITIONS` conditions).
pub fn parse_keyword_conditions(
    value: &serde_json::Value,
) -> Result<Vec<KeywordCondition>, ZodError> {
    let items: Vec<serde_json::Value> = deserialize(value)?;
    let mut issues = Issues::default();
    let mut conditions = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match parse_keyword_condition(item) {
            Ok(condition) => conditions.push(condition),
            Err(error) => issues
                .0
                .extend(error.issues.into_iter().map(|issue| ZodIssue {
                    path: prefixed(index, &issue.path),
                    message: issue.message,
                })),
        }
    }
    if items.is_empty() {
        issues.add("", "Too small: expected array to have >=1 items");
    }
    if items.len() > MAX_KEYWORD_CONDITIONS {
        issues.add(
            "",
            format!("Too big: expected array to have <={MAX_KEYWORD_CONDITIONS} items"),
        );
    }
    issues.finish(conditions)
}

fn prefixed(index: usize, path: &str) -> String {
    if path.is_empty() {
        index.to_string()
    } else {
        format!("{index}.{path}")
    }
}
