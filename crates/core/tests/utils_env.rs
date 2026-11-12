//! Port of `packages/utils/__tests__/env.test.ts`. `unknown` input is a
//! `serde_json::Value` (`undefined` and `null` are both `Null`).

use chatbotx_core::utils::env::parse_env_bool;
use serde_json::{Value, json};

/// Port of: packages/utils/__tests__/env.test.ts:5 "parseEnvBool > passes validated booleans through unchanged"
#[test]
fn parseenvbool_passes_validated_booleans_through_unchanged() {
    assert!(parse_env_bool(&json!(true)));
    assert!(!parse_env_bool(&json!(false)));
}

/// Port of: packages/utils/__tests__/env.test.ts:10 "parseEnvBool > parses zod stringbool truthy strings when validation was skipped"
#[test]
fn parseenvbool_parses_zod_stringbool_truthy_strings_when_validation_was_skipped() {
    assert!(parse_env_bool(&json!("true")));
    assert!(parse_env_bool(&json!("TRUE")));
    assert!(parse_env_bool(&json!("1")));
    assert!(parse_env_bool(&json!("yes")));
    assert!(parse_env_bool(&json!("on")));
    assert!(parse_env_bool(&json!(" true ")));
}

/// Port of: packages/utils/__tests__/env.test.ts:19 "parseEnvBool > treats falsy strings as false instead of truthy non-empty strings"
#[test]
fn parseenvbool_treats_falsy_strings_as_false_instead_of_truthy_non_empty_strings() {
    assert!(!parse_env_bool(&json!("false")));
    assert!(!parse_env_bool(&json!("0")));
    assert!(!parse_env_bool(&json!("no")));
    assert!(!parse_env_bool(&json!("off")));
}

/// Port of: packages/utils/__tests__/env.test.ts:26 "parseEnvBool > returns false for undefined, null, and unrecognized values"
#[test]
fn parseenvbool_returns_false_for_undefined_null_and_unrecognized_values() {
    assert!(!parse_env_bool(&Value::Null));
    assert!(!parse_env_bool(&Value::Null));
    assert!(!parse_env_bool(&json!("")));
    assert!(!parse_env_bool(&json!("banana")));
    assert!(!parse_env_bool(&json!(1)));
}
