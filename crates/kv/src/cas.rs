//! CAS semantics of `casStore` (spec 2.10; K11, K13, K14). Implemented by
//! [`crate::MemKv`] and [`crate::DurableKv`]; dyn-compatible so callers and
//! tests can inject either.
//!
//! Match rule (Lua `cjson`): numbers compare by value (`1 == 1.0`), strings by
//! bytes, booleans, `null == null`; a missing field never matches (also not
//! `null`); an object or array in `expected` never matches.

use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::error::KvError;

pub trait CasStore: Send + Sync {
    /// `SET key json PX ttl_ms NX`; an expired key is absent; a second write
    /// can neither overwrite nor extend. `true` when written.
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>>;

    /// The raw stored text; `None` when absent.
    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>>;

    /// Parsed JSON; `None` when absent or not valid JSON.
    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>>;

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>>;

    /// `expected: None` = the key must be absent (create-only); `Some(e)` = the
    /// key must exist and every field of `e` must match. On success writes
    /// `next` with a new TTL `ttl_ms`.
    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>>;

    /// Same match as [`CasStore::compare_and_swap`]; deletes on success.
    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>>;
}

/// The match loop of `COMPARE_AND_SWAP_JSON_LUA` / `COMPARE_AND_DELETE_JSON_LUA`
/// on the stored text `current`:
///
/// - `cjson.decode(current)` runs first: malformed JSON is a script error.
/// - an object is indexed by field;
/// - an array (integer keys) or a string (string methods) never yields an
///   equal value, so any expected field fails;
/// - a number, boolean or `null` cannot be indexed: a script error when
///   `expected` has a field (no index happens for an empty `expected`).
pub(crate) fn cjson_matches(
    key: &str,
    current: &str,
    expected: &Map<String, Value>,
) -> Result<bool, KvError> {
    let decoded: Value = serde_json::from_str(current)?;
    if expected.is_empty() {
        return Ok(true);
    }
    match &decoded {
        Value::Object(fields) => Ok(expected
            .iter()
            .all(|(field, value)| cjson_field_eq(fields.get(field), value))),
        Value::Array(_) | Value::String(_) => Ok(false),
        Value::Null | Value::Bool(_) | Value::Number(_) => Err(KvError::WrongType {
            key: key.to_owned(),
        }),
    }
}

/// Lua `decodedCurrent[field] == value` after `cjson.decode` of both sides.
fn cjson_field_eq(current: Option<&Value>, expected: &Value) -> bool {
    match (expected, current) {
        // `cjson.null` is one sentinel: null equals null, never a missing field.
        (Value::Null, Some(Value::Null)) => true,
        (Value::Bool(a), Some(Value::Bool(b))) => a == b,
        // Lua numbers are doubles: `1 == 1.0`.
        (Value::Number(a), Some(Value::Number(b))) => a.as_f64() == b.as_f64(),
        (Value::String(a), Some(Value::String(b))) => a == b,
        // Tables compare by identity: an object or array never matches.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obj(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn matches(current: &str, expected: Value) -> Result<bool, KvError> {
        cjson_matches("k", current, &obj(expected))
    }

    #[test]
    fn objects_and_arrays_in_expected_never_match() {
        let current = r#"{"a":{"x":1},"b":[1,2],"c":{}}"#;
        assert!(!matches(current, json!({"a": {"x": 1}})).unwrap());
        assert!(!matches(current, json!({"b": [1, 2]})).unwrap());
        assert!(!matches(current, json!({"c": {}})).unwrap());
    }

    #[test]
    fn null_matches_null_but_never_a_missing_field() {
        let current = r#"{"a":null}"#;
        assert!(matches(current, json!({"a": null})).unwrap());
        assert!(!matches(current, json!({"b": null})).unwrap());
        assert!(!matches(r#"{"a":false}"#, json!({"a": null})).unwrap());
        assert!(!matches(r#"{"a":0}"#, json!({"a": null})).unwrap());
    }

    #[test]
    fn numbers_compare_by_value_and_types_do_not_mix() {
        assert!(matches(r#"{"n":1}"#, json!({"n": 1.0})).unwrap());
        assert!(matches(r#"{"n":1e2}"#, json!({"n": 100})).unwrap());
        assert!(matches(r#"{"n":-0}"#, json!({"n": 0})).unwrap());
        assert!(!matches(r#"{"n":1}"#, json!({"n": 2})).unwrap());
        assert!(!matches(r#"{"n":"1"}"#, json!({"n": 1})).unwrap());
        assert!(!matches(r#"{"b":true}"#, json!({"b": 1})).unwrap());
        assert!(matches(r#"{"s":"x","b":true}"#, json!({"s": "x", "b": true})).unwrap());
    }

    #[test]
    fn every_expected_field_must_match_and_extra_current_fields_are_ignored() {
        let current = r#"{"phase":"reserved","fenceToken":"f1","extra":1}"#;
        assert!(matches(current, json!({"phase": "reserved", "fenceToken": "f1"})).unwrap());
        assert!(!matches(current, json!({"phase": "reserved", "fenceToken": "f2"})).unwrap());
        assert!(matches(current, json!({})).unwrap());
    }

    #[test]
    fn non_object_records_follow_the_lua_indexing_rules() {
        assert!(!matches("[1,2]", json!({"0": 1})).unwrap());
        assert!(!matches("\"text\"", json!({"len": 4})).unwrap());
        assert!(matches!(
            matches("5", json!({"a": 1})),
            Err(KvError::WrongType { .. })
        ));
        assert!(matches("5", json!({})).unwrap());
        assert!(matches!(
            matches("{not-json", json!({})),
            Err(KvError::Json(_))
        ));
    }
}
