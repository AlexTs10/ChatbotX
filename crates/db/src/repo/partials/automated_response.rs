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
    use KeywordConditionOperator::*;
    matches!(
        operator,
        ContainsPhone | ContainsEmail | ContainsImage | ContainsVideo | ContainsLink | IsThumbsUp
    )
}

/// `flattenKeywordConditionValues`: the trigger words, each once, in condition order.
pub fn flatten_keyword_condition_values(conditions: &[KeywordCondition]) -> Vec<String> {
    use KeywordConditionOperator::*;
    let mut seen = std::collections::HashSet::new();
    conditions
        .iter()
        .filter(|condition| {
            matches!(
                condition.operator,
                Is | SimilarTo | Contains | ContainsWholeWord | BeginsWith | EndsWith
            )
        })
        .flat_map(|condition| condition.values.iter())
        .filter(|value| seen.insert(value.as_str()))
        .cloned()
        .collect()
}
