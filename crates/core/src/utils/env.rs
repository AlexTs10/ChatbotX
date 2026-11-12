//! `packages/utils/src/env.ts`.

use serde_json::Value;

/// zod `z.stringbool()` default truthy values.
const TRUTHY_ENV_VALUES: &[&str] = &["true", "1", "yes", "on", "y", "enabled"];

/// `parseEnvBool`: a boolean passes through; a string is matched
/// (trimmed, case-insensitive) against zod's truthy set; anything else
/// (`null`, numbers, …) is `false`.
pub fn parse_env_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::String(s) => parse_env_bool_str(Some(s)),
        _ => false,
    }
}

/// [`parse_env_bool`] for an environment variable.
pub fn parse_env_bool_str(value: Option<&str>) -> bool {
    value.is_some_and(|v| TRUTHY_ENV_VALUES.contains(&crate::js::trim(v).to_lowercase().as_str()))
}
