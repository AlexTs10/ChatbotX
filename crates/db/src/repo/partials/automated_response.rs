//! Port of the keyword-condition types of `partials/automated-response.ts`.

use serde::{Deserialize, Serialize};

/// `keywordConditionOperators`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeywordConditionOperator {
    Is,
    SimilarTo,
    Contains,
    ContainsPhone,
    ContainsEmail,
    ContainsImage,
    ContainsVideo,
    ContainsLink,
    ContainsWholeWord,
    BeginsWith,
    EndsWith,
    IsThumbsUp,
    NotContains,
    MatchesRegex,
}

/// `MAX_KEYWORD_CONDITIONS`.
pub const MAX_KEYWORD_CONDITIONS: usize = 10;
/// `MAX_KEYWORD_CONDITION_VALUES`.
pub const MAX_KEYWORD_CONDITION_VALUES: usize = 50;
/// `MAX_KEYWORD_REGEX_LENGTH`.
pub const MAX_KEYWORD_REGEX_LENGTH: usize = 200;

/// `KeywordCondition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordCondition {
    pub operator: KeywordConditionOperator,
    pub values: Vec<String>,
}

/// `isValuelessKeywordConditionOperator`.
pub fn is_valueless_keyword_condition_operator(operator: KeywordConditionOperator) -> bool {
    let _ = operator;
    todo!("P3.2b")
}

/// `flattenKeywordConditionValues`: the trigger words, each once, in condition order.
pub fn flatten_keyword_condition_values(conditions: &[KeywordCondition]) -> Vec<String> {
    let _ = conditions;
    todo!("P3.2b")
}
