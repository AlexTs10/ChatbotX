//! Port of `packages/database/__tests__/keyword-condition-schema.test.ts` (P3.2b,
//! ported: `safeParse` is `parse_..` on a JSON value; RE2 is the Rust `regex` crate;
//! `matcher(..).find()` is `is_match`).

use chatbotx_db::repo::partials::automated_response::{
    KeywordCondition, KeywordConditionOperator, MAX_KEYWORD_CONDITIONS, MAX_KEYWORD_REGEX_LENGTH,
    flatten_keyword_condition_values,
};
use chatbotx_db::repo::partials::keyword_condition_schema::{
    compile_keyword_regex, parse_keyword_condition, parse_keyword_conditions,
};
use serde_json::{Value, json};

fn issue_messages(value: Value) -> Vec<String> {
    match parse_keyword_condition(&value) {
        Ok(_) => Vec::new(),
        Err(error) => error.messages().into_iter().map(str::to_owned).collect(),
    }
}

fn condition(operator: KeywordConditionOperator, values: &[&str]) -> KeywordCondition {
    KeywordCondition {
        operator,
        values: values.iter().map(|v| (*v).to_owned()).collect(),
    }
}

#[test]
fn keyword_condition_schema_requires_values_for_operators_that_match_keywords() {
    assert_eq!(
        issue_messages(json!({ "operator": "contains", "values": [] })),
        vec!["keywordConditionValuesRequired"]
    );
    assert!(issue_messages(json!({ "operator": "contains", "values": ["giá"] })).is_empty());
}

#[test]
fn keyword_condition_schema_rejects_values_on_value_less_operators() {
    assert!(issue_messages(json!({ "operator": "containsImage", "values": [] })).is_empty());
    assert_eq!(
        issue_messages(json!({ "operator": "isThumbsUp", "values": ["x"] })),
        vec!["keywordConditionTakesNoValues"]
    );
}

#[test]
fn keyword_condition_schema_trims_values_and_rejects_blank_ones() {
    let parsed = parse_keyword_condition(&json!({ "operator": "is", "values": ["  chào  "] }))
        .expect("parse");
    assert_eq!(parsed.values, vec!["chào"]);
    assert!(parse_keyword_condition(&json!({ "operator": "is", "values": ["  "] })).is_err());
}

#[test]
fn keyword_condition_schema_validates_regex_values_with_re2_syntax() {
    assert!(
        issue_messages(json!({ "operator": "matchesRegex", "values": ["^order-\\d+$"] }))
            .is_empty()
    );
    assert_eq!(
        issue_messages(json!({ "operator": "matchesRegex", "values": ["(unclosed"] })),
        vec!["invalidKeywordRegex"]
    );
    // Backreferences are not RE2 — rejected instead of failing at runtime.
    assert_eq!(
        issue_messages(json!({ "operator": "matchesRegex", "values": ["(a)\\1"] })),
        vec!["invalidKeywordRegex"]
    );
}

#[test]
fn keyword_condition_schema_rejects_unknown_operators() {
    assert!(
        parse_keyword_condition(&json!({ "operator": "startsWith", "values": ["a"] })).is_err()
    );
}

#[test]
fn keyword_conditions_schema_needs_at_least_one_and_at_most_the_maximum_number_of_conditions() {
    let condition = json!({ "operator": "containsLink", "values": [] });
    assert!(parse_keyword_conditions(&json!([])).is_err());
    assert!(parse_keyword_conditions(&json!([condition.clone()])).is_ok());
    let too_many = Value::Array(vec![condition; MAX_KEYWORD_CONDITIONS + 1]);
    assert!(parse_keyword_conditions(&too_many).is_err());
}

#[test]
fn compile_keyword_regex_is_case_insensitive_and_refuses_over_long_patterns() {
    assert!(compile_keyword_regex("abc").is_some_and(|re| re.is_match("xABCx")));
    assert!(compile_keyword_regex(&"a".repeat(MAX_KEYWORD_REGEX_LENGTH + 1)).is_none());
}

#[test]
fn flatten_keyword_condition_values_lists_each_trigger_keyword_once_in_condition_order() {
    use KeywordConditionOperator::*;
    assert_eq!(
        flatten_keyword_condition_values(&[
            condition(Contains, &["a", "b"]),
            condition(ContainsVideo, &[]),
            condition(BeginsWith, &["b", "c"]),
        ]),
        vec!["a", "b", "c"]
    );
}

#[test]
fn flatten_keyword_condition_values_leaves_out_excluded_words_and_regex_patterns() {
    use KeywordConditionOperator::*;
    assert_eq!(
        flatten_keyword_condition_values(&[
            condition(Contains, &["mua"]),
            condition(NotContains, &["spam"]),
            condition(MatchesRegex, &["^a"]),
        ]),
        vec!["mua"]
    );
}
