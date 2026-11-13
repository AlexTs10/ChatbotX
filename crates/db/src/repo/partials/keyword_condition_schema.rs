//! Port of `partials/keyword-condition-schema.ts`. RE2 (`re2js`) is the Rust `regex`
//! crate: linear time, no backreferences or look-around.

use super::automated_response::KeywordCondition;
use super::shared::ZodError;

/// `compileKeywordRegex`: a case-insensitive regex, `None` for an over-long or invalid
/// pattern.
pub fn compile_keyword_regex(pattern: &str) -> Option<regex::Regex> {
    let _ = pattern;
    todo!("P3.2b")
}

/// `keywordConditionSchema.safeParse`.
pub fn parse_keyword_condition(value: &serde_json::Value) -> Result<KeywordCondition, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `keywordConditionsSchema.safeParse` (1 to `MAX_KEYWORD_CONDITIONS` conditions).
pub fn parse_keyword_conditions(
    value: &serde_json::Value,
) -> Result<Vec<KeywordCondition>, ZodError> {
    let _ = value;
    todo!("P3.2b")
}
