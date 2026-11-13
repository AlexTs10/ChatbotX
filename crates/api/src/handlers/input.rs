//! oRPC input of a public route and the zod checks of its schema.
//!
//! The OpenAPI handler (`inputStructure: "compact"`) merges the path params with
//! the query (GET / DELETE) or the JSON body (other methods). A failed check is a
//! zod issue `{ code, path, message }`; all issues of one request go into one
//! `422 invalidRequestData` (`data.issues`), like oRPC's `ValidationError`
//! remapped by the builder's `toKnownOrpcError`.

use chatbotx_core::js;
use serde_json::{Map, Value, json};

use crate::orpc::OrpcError;

/// A path segment of an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

impl Seg {
    fn to_json(&self) -> Value {
        match self {
            Seg::Key(key) => json!(key),
            Seg::Index(index) => json!(index),
        }
    }
}

/// A path from the input root.
pub type Path = Vec<Seg>;

/// `[key]`.
pub fn key(name: &str) -> Path {
    vec![Seg::Key(name.to_string())]
}

/// `path + [name]`.
pub fn child(path: &[Seg], name: &str) -> Path {
    let mut next = path.to_vec();
    next.push(Seg::Key(name.to_string()));
    next
}

/// String checks (`z.string()` + `.trim()` / `.min()` / `.max()` / `.regex()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Str {
    pub trim: bool,
    pub min: Option<usize>,
    pub max: Option<usize>,
    pub pattern: Option<fn(&str) -> bool>,
}

impl Str {
    pub const fn trimmed(min: usize, max: usize) -> Str {
        Str {
            trim: true,
            min: Some(min),
            max: Some(max),
            pattern: None,
        }
    }
    pub const fn max(max: usize) -> Str {
        Str {
            trim: false,
            min: None,
            max: Some(max),
            pattern: None,
        }
    }
}

/// Number checks (`z.number()` + `.int()` / `.min()` / `.max()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Num {
    pub int: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Num {
    pub const fn range(min: f64, max: f64) -> Num {
        Num {
            int: false,
            min: Some(min),
            max: Some(max),
        }
    }
    pub const fn int(min: f64, max: f64) -> Num {
        Num {
            int: true,
            min: Some(min),
            max: Some(max),
        }
    }
}

fn type_name(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// `/^\d+$/` and at most 18 digits (`zodBigintIdAsString`).
pub fn is_bigint_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 18 && text.bytes().all(|b| b.is_ascii_digit())
}

/// The merged input object and the issues found so far.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub obj: Map<String, Value>,
    issues: Vec<Value>,
}

impl Input {
    /// Input from the path params and a JSON body (`None` = no body).
    pub fn from_body(params: &[(&'static str, String)], body: &[u8]) -> Input {
        let mut input = Input::default();
        if !body.iter().all(u8::is_ascii_whitespace) {
            match serde_json::from_slice::<Value>(body) {
                Ok(Value::Object(map)) => input.obj = map,
                Ok(other) => input.issue(
                    &[],
                    "invalid_type",
                    &format!(
                        "Invalid input: expected object, received {}",
                        type_name(Some(&other))
                    ),
                ),
                Err(_) => input.issue(
                    &[],
                    "invalid_type",
                    "Invalid input: expected object, received string",
                ),
            }
        }
        for (name, value) in params {
            input
                .obj
                .insert((*name).to_string(), Value::String(value.clone()));
        }
        input
    }

    /// Input from the path params and the query string (flat keys; the last
    /// value of a repeated key wins).
    pub fn from_query(params: &[(&'static str, String)], query: Option<&str>) -> Input {
        let mut input = Input::default();
        if let Some(query) = query {
            for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
                input
                    .obj
                    .insert(name.into_owned(), Value::String(value.into_owned()));
            }
        }
        for (name, value) in params {
            input
                .obj
                .insert((*name).to_string(), Value::String(value.clone()));
        }
        input
    }

    /// Records one issue.
    pub fn issue(&mut self, path: &[Seg], code: &str, message: &str) {
        self.issues.push(json!({
            "code": code,
            "path": path.iter().map(Seg::to_json).collect::<Vec<_>>(),
            "message": message,
        }));
    }

    /// Whether any issue was recorded.
    pub fn failed(&self) -> bool {
        !self.issues.is_empty()
    }

    /// `Ok` or the `422 invalidRequestData` of the recorded issues.
    pub fn finish(self) -> Result<(), OrpcError> {
        if self.issues.is_empty() {
            return Ok(());
        }
        let mut error = OrpcError::new(
            "invalidRequestData",
            Some(422),
            Some("Input validation failed"),
        );
        error.defined = true;
        error.data = Some(json!({ "issues": self.issues }));
        Err(error)
    }

    /// The raw value of a top-level key.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.obj.get(name)
    }

    /// Whether a top-level key was sent (JSON `null` counts as sent).
    pub fn has(&self, name: &str) -> bool {
        self.obj.contains_key(name)
    }

    fn missing(&mut self, path: &[Seg], expected: &str) {
        self.issue(
            path,
            "invalid_type",
            &format!("Invalid input: expected {expected}, received undefined"),
        );
    }

    fn wrong_type(&mut self, path: &[Seg], expected: &str, value: &Value) {
        self.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected {expected}, received {}",
                type_name(Some(value))
            ),
        );
    }

    /// A string at `path` (`value` = what was sent there). `None` = invalid
    /// (issue recorded) or absent when `required` is false.
    pub fn string_at(
        &mut self,
        path: &[Seg],
        value: Option<&Value>,
        spec: Str,
        required: bool,
    ) -> Option<String> {
        let text = match value {
            None => {
                if required {
                    self.missing(path, "string");
                }
                return None;
            }
            Some(Value::String(text)) => text,
            Some(other) => {
                let other = other.clone();
                self.wrong_type(path, "string", &other);
                return None;
            }
        };
        let text = if spec.trim {
            js::trim(text).to_string()
        } else {
            text.clone()
        };
        let length = js::utf16_len(&text);
        let mut ok = true;
        if let Some(min) = spec.min.filter(|&min| length < min) {
            self.issue(
                path,
                "too_small",
                &format!("Too small: expected string to have >={min} characters"),
            );
            ok = false;
        }
        if let Some(max) = spec.max.filter(|&max| length > max) {
            self.issue(
                path,
                "too_big",
                &format!("Too big: expected string to have <={max} characters"),
            );
            ok = false;
        }
        if spec.pattern.is_some_and(|pattern| !pattern(&text)) {
            self.issue(path, "invalid_format", "Invalid string: must match pattern");
            ok = false;
        }
        ok.then_some(text)
    }

    /// A JSON number at `path` (same `None` rules as [`Input::string_at`]).
    pub fn number_at(
        &mut self,
        path: &[Seg],
        value: Option<&Value>,
        spec: Num,
        required: bool,
    ) -> Option<f64> {
        let number = match value {
            None => {
                if required {
                    self.missing(path, "number");
                }
                return None;
            }
            Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
            Some(other) => {
                let other = other.clone();
                self.wrong_type(path, "number", &other);
                return None;
            }
        };
        self.check_number(path, number, spec)
    }

    fn check_number(&mut self, path: &[Seg], number: f64, spec: Num) -> Option<f64> {
        if !number.is_finite() {
            self.issue(
                path,
                "invalid_type",
                "Invalid input: expected number, received number",
            );
            return None;
        }
        let mut ok = true;
        if spec.int && (number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0) {
            self.issue(
                path,
                "invalid_type",
                "Invalid input: expected int, received number",
            );
            return None;
        }
        if let Some(min) = spec.min.filter(|&min| number < min) {
            self.issue(
                path,
                "too_small",
                &format!(
                    "Too small: expected number to be >={}",
                    js::number_to_string(min)
                ),
            );
            ok = false;
        }
        if let Some(max) = spec.max.filter(|&max| number > max) {
            self.issue(
                path,
                "too_big",
                &format!(
                    "Too big: expected number to be <={}",
                    js::number_to_string(max)
                ),
            );
            ok = false;
        }
        ok.then_some(number)
    }

    /// `z.coerce.number()` at `path` (query values arrive as strings).
    pub fn coerce_number_at(
        &mut self,
        path: &[Seg],
        value: Option<&Value>,
        spec: Num,
        required: bool,
    ) -> Option<f64> {
        let number = match value {
            None => {
                if required {
                    self.issue(
                        path,
                        "invalid_type",
                        "Invalid input: expected number, received NaN",
                    );
                }
                return None;
            }
            Some(Value::String(text)) => js::to_number(text),
            Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
            Some(Value::Bool(flag)) => f64::from(u8::from(*flag)),
            Some(Value::Null) => 0.0,
            Some(_) => f64::NAN,
        };
        if number.is_nan() {
            self.issue(
                path,
                "invalid_type",
                "Invalid input: expected number, received NaN",
            );
            return None;
        }
        self.check_number(path, number, spec)
    }

    /// A boolean at `path`.
    pub fn bool_at(&mut self, path: &[Seg], value: Option<&Value>, required: bool) -> Option<bool> {
        match value {
            None => {
                if required {
                    self.missing(path, "boolean");
                }
                None
            }
            Some(Value::Bool(flag)) => Some(*flag),
            Some(other) => {
                let other = other.clone();
                self.wrong_type(path, "boolean", &other);
                None
            }
        }
    }

    /// One of `options` at `path` (`z.enum`).
    pub fn enum_at(
        &mut self,
        path: &[Seg],
        value: Option<&Value>,
        options: &[&str],
        required: bool,
    ) -> Option<String> {
        match value {
            None => {
                if required {
                    self.issue(path, "invalid_value", "Invalid option");
                }
                None
            }
            Some(Value::String(text)) if options.contains(&text.as_str()) => Some(text.clone()),
            Some(_) => {
                let expected = options
                    .iter()
                    .map(|option| format!("\"{option}\""))
                    .collect::<Vec<_>>()
                    .join("|");
                self.issue(
                    path,
                    "invalid_value",
                    &format!("Invalid option: expected one of {expected}"),
                );
                None
            }
        }
    }

    /// A bigint id at `path` (`zodBigintIdAsString`).
    pub fn id_at(&mut self, path: &[Seg], value: Option<&Value>, required: bool) -> Option<i64> {
        let text = self.string_at(path, value, Str::default(), required)?;
        if !text.bytes().all(|b| b.is_ascii_digit()) || text.is_empty() {
            self.issue(
                path,
                "invalid_format",
                "Invalid string: must match pattern /^\\d+$/",
            );
            return None;
        }
        if text.len() > 18 {
            self.issue(
                path,
                "too_big",
                "Too big: expected string to have <=18 characters",
            );
            return None;
        }
        text.parse().ok()
    }

    // --- top-level keys -------------------------------------------------

    /// A required string key.
    pub fn string(&mut self, name: &str, spec: Str) -> Option<String> {
        let value = self.obj.get(name).cloned();
        self.string_at(&key(name), value.as_ref(), spec, true)
    }

    /// An optional string key (absent = `None`; `null` is an issue).
    pub fn opt_string(&mut self, name: &str, spec: Str) -> Option<String> {
        let value = self.obj.get(name).cloned();
        self.string_at(&key(name), value.as_ref(), spec, false)
    }

    /// A required number key.
    pub fn number(&mut self, name: &str, spec: Num) -> Option<f64> {
        let value = self.obj.get(name).cloned();
        self.number_at(&key(name), value.as_ref(), spec, true)
    }

    /// An optional number key.
    pub fn opt_number(&mut self, name: &str, spec: Num) -> Option<f64> {
        let value = self.obj.get(name).cloned();
        self.number_at(&key(name), value.as_ref(), spec, false)
    }

    /// An optional `z.coerce.number().int()` key (query).
    pub fn opt_coerce_int(&mut self, name: &str, spec: Num) -> Option<i64> {
        let value = self.obj.get(name).cloned();
        self.coerce_number_at(&key(name), value.as_ref(), Num { int: true, ..spec }, false)
            .map(|n| n as i64)
    }

    /// A required bigint id key (path param or body).
    pub fn id(&mut self, name: &str) -> Option<i64> {
        let value = self.obj.get(name).cloned();
        self.id_at(&key(name), value.as_ref(), true)
    }

    /// A `.nullish()` bigint id key: absent = `None`, `null` = `Some(None)`.
    pub fn nullish_id(&mut self, name: &str) -> Option<Option<i64>> {
        match self.obj.get(name).cloned() {
            None => None,
            Some(Value::Null) => Some(None),
            Some(value) => self.id_at(&key(name), Some(&value), true).map(Some),
        }
    }

    /// The `.refine(atLeastOneField)` of the update requests: one of `fields`
    /// (the schema keys besides the ids) was sent (`null` counts). Runs only
    /// when the fields themselves passed, like a zod object refinement.
    pub fn require_one_of(&mut self, fields: &[&str]) {
        if self.failed() {
            return;
        }
        if !fields.iter().any(|name| self.obj.contains_key(*name)) {
            self.issue(&[], "custom", "Send at least one field to update.");
        }
    }
}
