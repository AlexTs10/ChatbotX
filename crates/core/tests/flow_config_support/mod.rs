//! Shared helpers for the `packages/flow-config` test ports.
#![allow(dead_code, unused_imports)]

pub use chatbotx_core::flow_config::z::{self, Issue, PathSegment, SafeParse, Schema};
pub use chatbotx_core::flow_config::{self as fc};
pub use serde_json::{Value, json};

/// `{ ...base, ...patch }` (shallow).
pub fn spread(base: &Value, patch: Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Value::Object(patch) = patch {
        for (k, v) in patch {
            out.insert(k, v);
        }
    }
    Value::Object(out)
}

/// `{ ...base }` without `keys` (`const { a, ...rest } = base`).
pub fn without(base: &Value, keys: &[&str]) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    for k in keys {
        out.remove(*k);
    }
    Value::Object(out)
}

/// `expect(actual).toMatchObject(expected)`.
pub fn assert_match_object(actual: &Value, expected: &Value) {
    if let Err(path) = match_object(actual, expected, String::new()) {
        panic!("toMatchObject failed at `{path}`\nactual: {actual:#}\nexpected: {expected:#}");
    }
}

/// Recursive subset match (vitest `toMatchObject`).
pub fn matches_object(actual: &Value, expected: &Value) -> bool {
    match_object(actual, expected, String::new()).is_ok()
}

fn match_object(actual: &Value, expected: &Value, path: String) -> Result<(), String> {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            for (k, ev) in e {
                let p = format!("{path}.{k}");
                match a.get(k) {
                    Some(av) => match_object(av, ev, p)?,
                    None => return Err(p),
                }
            }
            Ok(())
        }
        (Value::Array(a), Value::Array(e)) => {
            if a.len() != e.len() {
                return Err(format!("{path}[len]"));
            }
            for (i, (av, ev)) in a.iter().zip(e).enumerate() {
                match_object(av, ev, format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Number(a), Value::Number(e)) if a.as_f64() == e.as_f64() => Ok(()),
        (a, e) if a == e => Ok(()),
        _ => Err(path),
    }
}

/// `schema.safeParse(value).success`.
pub fn ok(schema: &Schema, value: &Value) -> bool {
    schema.safe_parse(value).success
}

/// `schemas.some((schema) => schema.safeParse(value).success)`.
pub fn any_ok(schemas: &[Schema], value: &Value) -> bool {
    schemas.iter().any(|s| s.safe_parse(value).success)
}

/// `"x".repeat(n)`.
pub fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

/// `result.error?.issues.map((i) => i.message) ?? []`.
pub fn messages(result: &SafeParse) -> Vec<String> {
    result.messages()
}

/// `result.error?.issues.map((i) => i.path.join("."))`.
pub fn paths(result: &SafeParse) -> Vec<String> {
    result.paths()
}

/// A string field of a JSON object.
pub fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {value}"))
}

/// `Boolean(x)` for JSON.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
