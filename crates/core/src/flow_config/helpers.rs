//! Small JSON helpers shared by the flow-config ports (not a TS module).

use serde_json::{Map, Value};

/// `{ ...base, ...props }` (shallow; `props` must be an object or null).
pub fn spread_props(base: Value, props: &Value) -> Value {
    let mut out = match base {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Value::Object(props) = props {
        for (k, v) in props {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// `{ ...base, ...props, ...fixed }`.
pub fn spread3(base: Value, props: &Value, fixed: Value) -> Value {
    spread_props(spread_props(base, props), &fixed)
}

/// JS `typeof v === "string" && v.trim() === ""` / missing.
pub fn is_blank(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => crate::js::trim(s).is_empty(),
        _ => false,
    }
}

/// `value?.trim()` when the value is a string.
pub fn trimmed(value: &Value) -> Option<&str> {
    value.as_str().map(crate::js::trim)
}

/// A string field, `""` when absent or not a string.
pub fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// An array field, empty when absent or not an array.
pub fn arr_field<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// JS truthiness of an optional JSON value.
pub fn truthy(value: Option<&Value>) -> bool {
    crate::flow_config::z::truthy(value)
}

/// Reports every repeated key (`new Set()` + `ctx.addIssue` loops).
pub fn report_duplicates<'a>(
    ctx: &mut crate::flow_config::z::RefineCtx,
    items: impl IntoIterator<Item = &'a Value>,
    key: impl Fn(&'a Value) -> Option<String>,
    issue: impl Fn(usize, &str) -> (Vec<crate::flow_config::z::PathSegment>, String),
) {
    let mut seen = std::collections::HashSet::new();
    for (index, item) in items.into_iter().enumerate() {
        let Some(k) = key(item) else { continue };
        if seen.contains(&k) {
            let (path, message) = issue(index, &k);
            ctx.issue(path, message);
        }
        seen.insert(k);
    }
}

/// JS `String(value)` for a JSON scalar key (`Set` membership).
pub fn key_of(value: &Value) -> Option<String> {
    Some(match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// The provider/model/integration head shared by the AI step factories:
/// `provider = props.provider ?? "openai"`, `model = props.model ?? (…)`,
/// `integrationId` only for `openaiCompatible`.
pub fn ai_step_head(props: &Value, default_model: impl Fn(&str) -> Option<Value>) -> Value {
    let provider = props
        .get("provider")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| Value::String("openai".into()));
    let provider_str = provider.as_str().unwrap_or("").to_owned();
    let mut head = Map::new();
    head.insert("provider".into(), provider);
    if provider_str == "openaiCompatible" {
        let integration = match props.get("integrationId") {
            Some(v) => v.clone(),
            None => Value::String(String::new()),
        };
        head.insert("integrationId".into(), integration);
    }
    let model = match props.get("model").filter(|v| !v.is_null()) {
        Some(m) => Some(m.clone()),
        None if provider_str == "openaiCompatible" => Some(Value::String(String::new())),
        None => default_model(&provider_str),
    };
    if let Some(model) = model {
        head.insert("model".into(), model);
    }
    Value::Object(head)
}

/// `{ ...a, ...b }` over two owned objects.
pub fn merge(a: Value, b: Value) -> Value {
    spread_props(a, &b)
}

/// JS `Number(value)` for a JSON value (`undefined` is `None`).
pub fn js_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => crate::js::to_number(s),
        Some(_) => f64::NAN,
    }
}

/// A JSON number from an `f64` (integers stay integers).
pub fn num_value(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}
