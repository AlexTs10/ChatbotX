//! A small runtime port of the zod 4 (4.6.7) parse semantics used by
//! `packages/flow-config`: schemas are values built with the same
//! combinators, parse `serde_json::Value` input, apply defaults/transforms,
//! and report issues with zod's paths and English messages.
//!
//! `undefined` (an absent key) is `None`; JSON has no other `undefined`.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, OnceLock};

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::js;

// ------------------------------------------------------------------ issues

/// One step of an issue path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(untagged)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathSegment::Key(key) => f.write_str(key),
            PathSegment::Index(index) => write!(f, "{index}"),
        }
    }
}

impl From<&str> for PathSegment {
    fn from(value: &str) -> Self {
        PathSegment::Key(value.to_owned())
    }
}

impl From<String> for PathSegment {
    fn from(value: String) -> Self {
        PathSegment::Key(value)
    }
}

impl From<&String> for PathSegment {
    fn from(value: &String) -> Self {
        PathSegment::Key(value.clone())
    }
}

impl From<usize> for PathSegment {
    fn from(value: usize) -> Self {
        PathSegment::Index(value)
    }
}

impl From<&PathSegment> for PathSegment {
    fn from(value: &PathSegment) -> Self {
        value.clone()
    }
}

impl PathSegment {
    pub fn as_key(&self) -> Option<&str> {
        match self {
            PathSegment::Key(key) => Some(key),
            PathSegment::Index(_) => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            PathSegment::Key(key) => Value::String(key.clone()),
            PathSegment::Index(index) => json!(index),
        }
    }
}

/// `vec![PathSegment, …]` from keys and indexes.
#[macro_export]
macro_rules! zpath {
    ($($seg:expr),* $(,)?) => {
        vec![$($crate::flow_config::z::PathSegment::from($seg)),*]
    };
}
pub use zpath as path;

/// zod issue codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    InvalidType,
    InvalidValue,
    TooBig,
    TooSmall,
    InvalidFormat,
    NotMultipleOf,
    UnrecognizedKeys,
    InvalidKey,
    InvalidUnion,
    InvalidElement,
    Custom,
}

impl IssueCode {
    pub fn as_str(self) -> &'static str {
        match self {
            IssueCode::InvalidType => "invalid_type",
            IssueCode::InvalidValue => "invalid_value",
            IssueCode::TooBig => "too_big",
            IssueCode::TooSmall => "too_small",
            IssueCode::InvalidFormat => "invalid_format",
            IssueCode::NotMultipleOf => "not_multiple_of",
            IssueCode::UnrecognizedKeys => "unrecognized_keys",
            IssueCode::InvalidKey => "invalid_key",
            IssueCode::InvalidUnion => "invalid_union",
            IssueCode::InvalidElement => "invalid_element",
            IssueCode::Custom => "custom",
        }
    }
}

/// An issue while parsing (zod's raw issue before `finalizeIssue`).
#[derive(Debug, Clone)]
pub struct RawIssue {
    pub code: IssueCode,
    pub path: Vec<PathSegment>,
    pub message: Option<String>,
    /// zod `continue`: `true` lets later checks run.
    pub cont: bool,
    pub expected: Option<String>,
    /// The input (for `received`); `None` = undefined.
    pub input: Option<Value>,
    pub origin: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub inclusive: bool,
    pub exact: bool,
    pub values: Vec<Value>,
    pub keys: Vec<String>,
    pub format: Option<String>,
    pub pattern: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub includes: Option<String>,
    pub divisor: Option<f64>,
    pub options: Vec<Value>,
    pub errors: Vec<Vec<Issue>>,
    pub params: Option<Value>,
}

impl RawIssue {
    pub fn new(code: IssueCode) -> Self {
        RawIssue {
            code,
            path: Vec::new(),
            message: None,
            cont: false,
            expected: None,
            input: None,
            origin: None,
            minimum: None,
            maximum: None,
            inclusive: true,
            exact: false,
            values: Vec::new(),
            keys: Vec::new(),
            format: None,
            pattern: None,
            prefix: None,
            suffix: None,
            includes: None,
            divisor: None,
            options: Vec::new(),
            errors: Vec::new(),
            params: None,
        }
    }

    /// A `custom` issue (refine / superRefine `ctx.addIssue`).
    pub fn custom(path: Vec<PathSegment>, message: impl Into<String>) -> Self {
        let mut issue = RawIssue::new(IssueCode::Custom);
        issue.path = path;
        issue.message = Some(message.into());
        issue.cont = true;
        issue
    }

    pub fn invalid_type(expected: &str, input: Option<&Value>) -> Self {
        let mut issue = RawIssue::new(IssueCode::InvalidType);
        issue.expected = Some(expected.to_owned());
        issue.input = input.cloned();
        issue
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn finalize(self) -> Issue {
        let message = match &self.message {
            Some(message) => message.clone(),
            None => locale_message(&self),
        };
        Issue {
            code: self.code,
            path: self.path,
            message,
            expected: self.expected,
            origin: self.origin,
            minimum: self.minimum,
            maximum: self.maximum,
            keys: self.keys,
            values: self.values,
            format: self.format,
            errors: self.errors,
            params: self.params,
        }
    }
}

/// A finalized issue (`ZodError#issues[n]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub code: IssueCode,
    pub path: Vec<PathSegment>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Vec<Issue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Issue {
    /// `issue.path.join(".")`.
    pub fn path_string(&self) -> String {
        self.path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }

    /// The path as a JSON array (keys as strings, indexes as numbers).
    pub fn path_json(&self) -> Value {
        Value::Array(self.path.iter().map(PathSegment::to_json).collect())
    }
}

/// `ZodError`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZodError {
    pub issues: Vec<Issue>,
}

impl fmt::Display for ZodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let issues = serde_json::to_string_pretty(&self.issues).unwrap_or_default();
        f.write_str(&issues)
    }
}

impl std::error::Error for ZodError {}

/// `safeParse` result.
#[derive(Debug, Clone)]
pub struct SafeParse {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<ZodError>,
}

impl SafeParse {
    /// The issues (empty on success).
    pub fn issues(&self) -> &[Issue] {
        self.error
            .as_ref()
            .map(|e| e.issues.as_slice())
            .unwrap_or(&[])
    }

    pub fn messages(&self) -> Vec<String> {
        self.issues().iter().map(|i| i.message.clone()).collect()
    }

    pub fn paths(&self) -> Vec<String> {
        self.issues().iter().map(Issue::path_string).collect()
    }

    /// The parsed value; panics on failure (test helper).
    pub fn unwrap(self) -> Value {
        match (self.data, self.error) {
            (Some(data), None) => data,
            (_, Some(err)) => panic!("parse failed: {err}"),
            (None, None) => Value::Null,
        }
    }
}

// ----------------------------------------------------------------- messages

fn parsed_type(value: Option<&Value>) -> &'static str {
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

fn stringify_primitive(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        Value::Number(n) => js::number_to_string(n.as_f64().unwrap_or(0.0)),
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn num(value: f64) -> String {
    js::number_to_string(value)
}

fn format_name(format: &str) -> &str {
    match format {
        "regex" => "input",
        "email" => "email address",
        "url" => "URL",
        "emoji" => "emoji",
        "uuid" => "UUID",
        "uuidv4" => "UUIDv4",
        "uuidv6" => "UUIDv6",
        "nanoid" => "nanoid",
        "guid" => "GUID",
        "cuid" => "cuid",
        "cuid2" => "cuid2",
        "ulid" => "ULID",
        "xid" => "XID",
        "ksuid" => "KSUID",
        "datetime" => "ISO datetime",
        "date" => "ISO date",
        "time" => "ISO time",
        "duration" => "ISO duration",
        "ipv4" => "IPv4 address",
        "ipv6" => "IPv6 address",
        "base64" => "base64-encoded string",
        "base64url" => "base64url-encoded string",
        "json_string" => "JSON string",
        "e164" => "E.164 number",
        "jwt" => "JWT",
        "template_literal" => "input",
        other => other,
    }
}

fn locale_message(issue: &RawIssue) -> String {
    match issue.code {
        IssueCode::InvalidType => {
            let expected = issue.expected.as_deref().unwrap_or("unknown");
            let received = parsed_type(issue.input.as_ref());
            format!("Invalid input: expected {expected}, received {received}")
        }
        IssueCode::InvalidValue => {
            if issue.values.len() == 1 {
                format!(
                    "Invalid input: expected {}",
                    stringify_primitive(&issue.values[0])
                )
            } else {
                let joined: Vec<String> = issue.values.iter().map(stringify_primitive).collect();
                format!("Invalid option: expected one of {}", joined.join("|"))
            }
        }
        IssueCode::TooBig => {
            let adj = if issue.exact {
                "exactly "
            } else if issue.inclusive {
                "<="
            } else {
                "<"
            };
            let origin = issue.origin.as_deref().unwrap_or("value");
            let max = num(issue.maximum.unwrap_or(0.0));
            match sizing(origin) {
                Some(unit) => format!("Too big: expected {origin} to have {adj}{max} {unit}"),
                None => format!("Too big: expected {origin} to be {adj}{max}"),
            }
        }
        IssueCode::TooSmall => {
            let adj = if issue.exact {
                "exactly "
            } else if issue.inclusive {
                ">="
            } else {
                ">"
            };
            let origin = issue.origin.as_deref().unwrap_or("undefined");
            let min = num(issue.minimum.unwrap_or(0.0));
            match sizing(origin) {
                Some(unit) => format!("Too small: expected {origin} to have {adj}{min} {unit}"),
                None => format!("Too small: expected {origin} to be {adj}{min}"),
            }
        }
        IssueCode::InvalidFormat => match issue.format.as_deref() {
            Some("starts_with") => format!(
                "Invalid string: must start with \"{}\"",
                issue.prefix.as_deref().unwrap_or("")
            ),
            Some("ends_with") => format!(
                "Invalid string: must end with \"{}\"",
                issue.suffix.as_deref().unwrap_or("")
            ),
            Some("includes") => format!(
                "Invalid string: must include \"{}\"",
                issue.includes.as_deref().unwrap_or("")
            ),
            Some("regex") => format!(
                "Invalid string: must match pattern {}",
                issue.pattern.as_deref().unwrap_or("")
            ),
            Some(other) => format!("Invalid {}", format_name(other)),
            None => "Invalid input".to_owned(),
        },
        IssueCode::NotMultipleOf => format!(
            "Invalid number: must be a multiple of {}",
            num(issue.divisor.unwrap_or(0.0))
        ),
        IssueCode::UnrecognizedKeys => {
            let joined: Vec<String> = issue.keys.iter().map(|k| format!("\"{k}\"")).collect();
            format!(
                "Unrecognized key{}: {}",
                if issue.keys.len() > 1 { "s" } else { "" },
                joined.join(", ")
            )
        }
        IssueCode::InvalidKey => format!(
            "Invalid key in {}",
            issue.origin.as_deref().unwrap_or("record")
        ),
        IssueCode::InvalidUnion => {
            if !issue.options.is_empty() {
                let opts: Vec<String> = issue
                    .options
                    .iter()
                    .map(|o| match o {
                        Value::String(s) => format!("'{s}'"),
                        other => format!("'{}'", stringify_primitive(other).trim_matches('"')),
                    })
                    .collect();
                format!("Invalid discriminator value. Expected {}", opts.join(" | "))
            } else {
                "Invalid input".to_owned()
            }
        }
        IssueCode::InvalidElement => format!(
            "Invalid value in {}",
            issue.origin.as_deref().unwrap_or("array")
        ),
        IssueCode::Custom => "Invalid input".to_owned(),
    }
}

fn sizing(origin: &str) -> Option<&'static str> {
    match origin {
        "string" => Some("characters"),
        "file" => Some("bytes"),
        "array" | "set" => Some("items"),
        "map" => Some("entries"),
        _ => None,
    }
}

// ------------------------------------------------------------------ payload

/// The value being parsed plus the issues so far (zod's `ParsePayload`).
#[derive(Debug, Clone, Default)]
pub struct Payload {
    pub value: Option<Value>,
    pub issues: Vec<RawIssue>,
    pub aborted: bool,
}

impl Payload {
    pub fn new(value: Option<Value>) -> Self {
        Payload {
            value,
            issues: Vec::new(),
            aborted: false,
        }
    }

    fn is_aborted_from(&self, start: usize) -> bool {
        self.aborted
            || self.issues[start.min(self.issues.len())..]
                .iter()
                .any(|i| !i.cont)
    }

    fn explicitly_aborted(&self) -> bool {
        self.aborted
    }
}

/// The context handed to refinements and transforms (`ctx.addIssue`).
#[derive(Debug, Default)]
pub struct RefineCtx {
    pub issues: Vec<RawIssue>,
    input: Option<Value>,
}

impl RefineCtx {
    /// `ctx.addIssue({ code: "custom", path, message })`.
    pub fn issue(&mut self, path: Vec<PathSegment>, message: impl Into<String>) {
        let mut issue = RawIssue::custom(path, message);
        issue.input = self.input.clone();
        self.issues.push(issue);
    }

    /// `ctx.addIssue({ code: "custom", message })` at the current path.
    pub fn message(&mut self, message: impl Into<String>) {
        self.issue(Vec::new(), message);
    }

    /// `ctx.addIssue(rawIssue)` (any code); `cont` defaults to true.
    pub fn add(&mut self, mut issue: RawIssue) {
        issue.cont = true;
        if issue.input.is_none() {
            issue.input = self.input.clone();
        }
        self.issues.push(issue);
    }

    /// Adds a finalized issue (re-emitted from a nested parse).
    pub fn add_issue(&mut self, issue: &Issue) {
        let mut raw = RawIssue::new(issue.code);
        raw.path = issue.path.clone();
        raw.message = Some(issue.message.clone());
        raw.cont = true;
        self.issues.push(raw);
    }

    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

// ------------------------------------------------------------------ schemas

pub type RefineFn = Arc<dyn Fn(&Value) -> bool + Send + Sync>;
pub type SuperRefineFn = Arc<dyn Fn(&Value, &mut RefineCtx) + Send + Sync>;
pub type TransformFn = Arc<dyn Fn(Value, &mut RefineCtx) -> Value + Send + Sync>;
pub type TransformOptFn = Arc<dyn Fn(Option<Value>, &mut RefineCtx) -> Option<Value> + Send + Sync>;
pub type PreprocessFn = Arc<dyn Fn(Option<Value>) -> Option<Value> + Send + Sync>;
pub type LazyFn = Arc<dyn Fn() -> Schema + Send + Sync>;
pub type DefaultFn = Arc<dyn Fn() -> Value + Send + Sync>;

/// The kind of value a schema parses (zod `def.type`).
#[derive(Clone)]
pub enum Kind {
    String {
        coerce: bool,
    },
    Number {
        coerce: bool,
        int: bool,
    },
    Boolean {
        coerce: bool,
    },
    Null,
    Undefined,
    Never,
    Any,
    Unknown,
    Date,
    Literal(Vec<Value>),
    Enum(Vec<String>),
    Array(Schema),
    Tuple(Vec<Schema>, Option<Schema>),
    Object {
        shape: Vec<(String, Schema)>,
        catchall: Catchall,
    },
    Record(Schema, Schema),
    Union(Vec<Schema>),
    DiscriminatedUnion(String, Vec<Schema>),
    Intersection(Schema, Schema),
    Optional(Schema),
    Nullable(Schema),
    NonOptional(Schema),
    Default(Schema, DefaultValue),
    Prefault(Schema, DefaultValue),
    Catch(Schema, Value),
    Pipe(Schema, Schema),
    Transform(TransformFn),
    TransformOpt(TransformOptFn),
    Preprocess(PreprocessFn, Schema),
    Lazy(LazyFn, Arc<OnceLock<Schema>>),
    Custom(RefineFn),
}

#[derive(Clone)]
pub enum DefaultValue {
    Value(Value),
    Fn(DefaultFn),
}

impl DefaultValue {
    pub fn get(&self) -> Value {
        match self {
            DefaultValue::Value(v) => v.clone(),
            DefaultValue::Fn(f) => f(),
        }
    }
}

/// Unknown-key policy of an object.
#[derive(Clone)]
pub enum Catchall {
    Strip,
    Strict,
    Loose,
    Schema(Schema),
}

/// A string/number/array check (zod `def.checks`).
#[derive(Clone)]
pub enum Check {
    MinLength(usize),
    MaxLength(usize),
    Length(usize),
    Gt(f64, bool),
    Lt(f64, bool),
    MultipleOf(f64),
    Int,
    Regex(Arc<Regex>, String),
    Email,
    Url {
        hostname: Option<Arc<Regex>>,
        protocol: Option<Arc<Regex>>,
        normalize: bool,
    },
    StartsWith(String),
    EndsWith(String),
    Includes(String),
    Lowercase,
    Uppercase,
    IsoDatetime {
        offset: bool,
        local: bool,
        precision: Option<i32>,
    },
    IsoDate,
    IsoTime,
    Uuid,
    Trim,
    ToLowerCase,
    ToUpperCase,
    Overwrite(TransformFn),
    Refine {
        f: RefineFn,
        message: Option<String>,
        path: Option<Vec<PathSegment>>,
        abort: bool,
    },
    SuperRefine(SuperRefineFn, bool),
    Raw(Arc<dyn Fn(&mut Payload) + Send + Sync>),
}

#[derive(Clone)]
pub struct CheckEntry {
    pub check: Check,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct Node {
    pub kind: Kind,
    pub checks: Vec<CheckEntry>,
    pub error: Option<String>,
    pub description: Option<String>,
    pub meta: Option<Value>,
}

/// A schema (cheap to clone).
#[derive(Clone)]
pub struct Schema(pub Arc<Node>);

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Schema({})", self.type_name())
    }
}

fn node(kind: Kind) -> Schema {
    Schema(Arc::new(Node {
        kind,
        checks: Vec::new(),
        error: None,
        description: None,
        meta: None,
    }))
}

fn compile_js_regex(source: &str, flags: &str) -> Regex {
    let mut prefix = String::new();
    if flags.contains('i') {
        prefix.push_str("(?i)");
    }
    if flags.contains('m') {
        prefix.push_str("(?m)");
    }
    if flags.contains('s') {
        prefix.push_str("(?s)");
    }
    // JS `\d` / `\w` / `\s` are ASCII-only without the `u` flag (`\s` is
    // Unicode white space in both; close enough for the patterns here).
    let src = source
        .replace("\\d", "[0-9]")
        .replace("\\w", "[A-Za-z0-9_]")
        .replace("\\/", "/");
    Regex::new(&format!("{prefix}{src}"))
        .unwrap_or_else(|err| panic!("invalid regex /{source}/{flags}: {err}"))
}

// ------------------------------------------------------------- constructors

pub fn string() -> Schema {
    node(Kind::String { coerce: false })
}
pub fn number() -> Schema {
    node(Kind::Number {
        coerce: false,
        int: false,
    })
}
/// `z.int()`.
pub fn int() -> Schema {
    number().int()
}
pub fn boolean() -> Schema {
    node(Kind::Boolean { coerce: false })
}
pub fn null() -> Schema {
    node(Kind::Null)
}
pub fn undefined() -> Schema {
    node(Kind::Undefined)
}
pub fn never() -> Schema {
    node(Kind::Never)
}
pub fn any() -> Schema {
    node(Kind::Any)
}
pub fn unknown() -> Schema {
    node(Kind::Unknown)
}
pub fn date() -> Schema {
    node(Kind::Date)
}
pub fn literal(value: Value) -> Schema {
    node(Kind::Literal(vec![value]))
}
pub fn literals(values: Vec<Value>) -> Schema {
    node(Kind::Literal(values))
}
pub fn enum_(values: &[&str]) -> Schema {
    node(Kind::Enum(values.iter().map(|v| (*v).to_owned()).collect()))
}
pub fn enum_owned(values: Vec<String>) -> Schema {
    node(Kind::Enum(values))
}
pub fn array(item: Schema) -> Schema {
    node(Kind::Array(item))
}
pub fn tuple(items: Vec<Schema>) -> Schema {
    node(Kind::Tuple(items, None))
}
pub fn tuple_rest(items: Vec<Schema>, rest: Schema) -> Schema {
    node(Kind::Tuple(items, Some(rest)))
}
pub fn object<K: Into<String>>(shape: Vec<(K, Schema)>) -> Schema {
    node(Kind::Object {
        shape: shape.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        catchall: Catchall::Strip,
    })
}
pub fn strict_object<K: Into<String>>(shape: Vec<(K, Schema)>) -> Schema {
    object(shape).strict()
}
pub fn record(key: Schema, value: Schema) -> Schema {
    node(Kind::Record(key, value))
}
pub fn union(options: Vec<Schema>) -> Schema {
    node(Kind::Union(options))
}
pub fn discriminated_union(key: &str, options: Vec<Schema>) -> Schema {
    node(Kind::DiscriminatedUnion(key.to_owned(), options))
}
pub fn intersection(left: Schema, right: Schema) -> Schema {
    node(Kind::Intersection(left, right))
}
pub fn lazy(f: impl Fn() -> Schema + Send + Sync + 'static) -> Schema {
    node(Kind::Lazy(Arc::new(f), Arc::new(OnceLock::new())))
}
pub fn custom(f: impl Fn(&Value) -> bool + Send + Sync + 'static, message: Option<&str>) -> Schema {
    let s = node(Kind::Custom(Arc::new(f)));
    match message {
        Some(m) => s.error(m),
        None => s,
    }
}
pub fn transform(f: impl Fn(Value, &mut RefineCtx) -> Value + Send + Sync + 'static) -> Schema {
    node(Kind::Transform(Arc::new(f)))
}
pub fn preprocess(
    f: impl Fn(Option<Value>) -> Option<Value> + Send + Sync + 'static,
    schema: Schema,
) -> Schema {
    node(Kind::Preprocess(Arc::new(f), schema))
}

// ----------------------------------------------------------------- builders

impl Schema {
    fn edit(mut self, f: impl FnOnce(&mut Node)) -> Self {
        f(Arc::make_mut(&mut self.0));
        self
    }

    fn with_check(self, check: Check) -> Self {
        self.edit(|n| {
            n.checks.push(CheckEntry {
                check,
                message: None,
            })
        })
    }

    /// Message for the last check (`.min(1, "msg")`), or for the schema
    /// itself when it has no checks (a string format such as `z.email(msg)`).
    pub fn msg(self, message: &str) -> Self {
        self.edit(|n| match n.checks.last_mut() {
            Some(last) => last.message = Some(message.to_owned()),
            None => n.error = Some(message.to_owned()),
        })
    }

    /// Schema-level error message (`z.string({ error })`).
    pub fn error(self, message: &str) -> Self {
        self.edit(|n| n.error = Some(message.to_owned()))
    }

    pub fn describe(self, description: &str) -> Self {
        self.edit(|n| n.description = Some(description.to_owned()))
    }

    pub fn meta(self, meta: Value) -> Self {
        self.edit(|n| {
            if let Some(Value::String(d)) = meta.get("description") {
                n.description = Some(d.clone());
            }
            let mut rest = meta.clone();
            if let Some(obj) = rest.as_object_mut() {
                obj.remove("description");
            }
            n.meta = Some(rest);
        })
    }

    pub fn coerce(self) -> Self {
        self.edit(|n| match &mut n.kind {
            Kind::String { coerce } | Kind::Boolean { coerce } | Kind::Number { coerce, .. } => {
                *coerce = true
            }
            _ => {}
        })
    }

    pub fn min(self, n: usize) -> Self {
        self.with_check(Check::MinLength(n))
    }
    pub fn max(self, n: usize) -> Self {
        self.with_check(Check::MaxLength(n))
    }
    pub fn length(self, n: usize) -> Self {
        self.with_check(Check::Length(n))
    }
    /// `.nonempty()` on strings/arrays.
    pub fn nonempty(self) -> Self {
        self.min(1)
    }
    pub fn gt(self, v: f64) -> Self {
        self.with_check(Check::Gt(v, false))
    }
    pub fn gte(self, v: f64) -> Self {
        self.with_check(Check::Gt(v, true))
    }
    pub fn lt(self, v: f64) -> Self {
        self.with_check(Check::Lt(v, false))
    }
    pub fn lte(self, v: f64) -> Self {
        self.with_check(Check::Lt(v, true))
    }
    pub fn positive(self) -> Self {
        self.gt(0.0)
    }
    pub fn nonnegative(self) -> Self {
        self.gte(0.0)
    }
    pub fn multiple_of(self, v: f64) -> Self {
        self.with_check(Check::MultipleOf(v))
    }
    pub fn int(self) -> Self {
        let s = self.edit(|n| {
            if let Kind::Number { int, .. } = &mut n.kind {
                *int = true;
            }
        });
        s.with_check(Check::Int)
    }
    pub fn number_format(self, _format: &str) -> Self {
        self
    }
    /// `.regex(/source/flags)`.
    pub fn regex(self, source: &str, flags: &str) -> Self {
        let re = compile_js_regex(source, flags);
        self.with_check(Check::Regex(Arc::new(re), format!("/{source}/{flags}")))
    }
    pub fn email(self) -> Self {
        self.with_check(Check::Email)
    }
    pub fn url(self) -> Self {
        self.with_check(Check::Url {
            hostname: None,
            protocol: None,
            normalize: false,
        })
    }
    pub fn url_with(self, hostname: Option<&str>, protocol: Option<&str>, normalize: bool) -> Self {
        self.with_check(Check::Url {
            hostname: hostname.map(|h| Arc::new(compile_js_regex(h, ""))),
            protocol: protocol.map(|p| Arc::new(compile_js_regex(p, ""))),
            normalize,
        })
    }
    pub fn starts_with(self, prefix: &str) -> Self {
        self.with_check(Check::StartsWith(prefix.to_owned()))
    }
    pub fn ends_with(self, suffix: &str) -> Self {
        self.with_check(Check::EndsWith(suffix.to_owned()))
    }
    pub fn includes(self, needle: &str) -> Self {
        self.with_check(Check::Includes(needle.to_owned()))
    }
    pub fn lowercase(self) -> Self {
        self.with_check(Check::Lowercase)
    }
    pub fn uppercase(self) -> Self {
        self.with_check(Check::Uppercase)
    }
    pub fn iso_datetime(self, offset: bool, local: bool, precision: Option<i32>) -> Self {
        self.with_check(Check::IsoDatetime {
            offset,
            local,
            precision,
        })
    }
    pub fn iso_date(self) -> Self {
        self.with_check(Check::IsoDate)
    }
    pub fn iso_time(self) -> Self {
        self.with_check(Check::IsoTime)
    }
    pub fn uuid(self) -> Self {
        self.with_check(Check::Uuid)
    }
    pub fn trim(self) -> Self {
        self.with_check(Check::Trim)
    }
    pub fn to_lower_case(self) -> Self {
        self.with_check(Check::ToLowerCase)
    }
    pub fn to_upper_case(self) -> Self {
        self.with_check(Check::ToUpperCase)
    }
    pub fn overwrite(
        self,
        f: impl Fn(Value, &mut RefineCtx) -> Value + Send + Sync + 'static,
    ) -> Self {
        self.with_check(Check::Overwrite(Arc::new(f)))
    }
    /// `.refine(fn, message | { message, path })`.
    pub fn refine(
        self,
        f: impl Fn(&Value) -> bool + Send + Sync + 'static,
        message: Option<&str>,
        path: Option<&[&str]>,
    ) -> Self {
        self.refine_owned(
            f,
            message.map(str::to_owned),
            path.map(|p| p.iter().map(|s| PathSegment::from(*s)).collect()),
        )
    }
    pub fn refine_owned(
        self,
        f: impl Fn(&Value) -> bool + Send + Sync + 'static,
        message: Option<String>,
        path: Option<Vec<PathSegment>>,
    ) -> Self {
        self.with_check(Check::Refine {
            f: Arc::new(f),
            message,
            path,
            abort: false,
        })
    }
    pub fn super_refine(self, f: impl Fn(&Value, &mut RefineCtx) + Send + Sync + 'static) -> Self {
        self.with_check(Check::SuperRefine(Arc::new(f), false))
    }
    /// `.check(fn)` with a raw payload function.
    pub fn check(self, f: impl Fn(&mut Payload) + Send + Sync + 'static) -> Self {
        self.with_check(Check::Raw(Arc::new(f)))
    }
    /// Marks the last refinement as aborting (`{ abort: true }`).
    pub fn abort(self) -> Self {
        self.edit(|n| {
            if let Some(last) = n.checks.last_mut() {
                match &mut last.check {
                    Check::Refine { abort, .. } => *abort = true,
                    Check::SuperRefine(_, abort) => *abort = true,
                    _ => {}
                }
            }
        })
    }

    pub fn optional(self) -> Self {
        node(Kind::Optional(self))
    }
    pub fn nullable(self) -> Self {
        node(Kind::Nullable(self))
    }
    pub fn nullish(self) -> Self {
        self.nullable().optional()
    }
    pub fn nonoptional(self) -> Self {
        node(Kind::NonOptional(self))
    }
    pub fn default(self, value: Value) -> Self {
        node(Kind::Default(self, DefaultValue::Value(value)))
    }
    pub fn default_fn(self, f: impl Fn() -> Value + Send + Sync + 'static) -> Self {
        node(Kind::Default(self, DefaultValue::Fn(Arc::new(f))))
    }
    pub fn prefault(self, value: Value) -> Self {
        node(Kind::Prefault(self, DefaultValue::Value(value)))
    }
    pub fn catch_value(self, value: Value) -> Self {
        node(Kind::Catch(self, value))
    }
    pub fn pipe(self, out: Schema) -> Self {
        node(Kind::Pipe(self, out))
    }
    pub fn transform(
        self,
        f: impl Fn(Value, &mut RefineCtx) -> Value + Send + Sync + 'static,
    ) -> Self {
        self.pipe(transform(f))
    }
    /// `.transform(fn)` where the callback also sees `undefined` (`None`).
    pub fn transform_opt(
        self,
        f: impl Fn(Option<Value>, &mut RefineCtx) -> Option<Value> + Send + Sync + 'static,
    ) -> Self {
        self.pipe(node(Kind::TransformOpt(Arc::new(f))))
    }
    pub fn or(self, other: Schema) -> Self {
        union(vec![self, other])
    }
    pub fn and(self, other: Schema) -> Self {
        intersection(self, other)
    }
    /// `.array()`.
    pub fn array(self) -> Self {
        array(self)
    }

    pub fn strict(self) -> Self {
        self.edit(|n| {
            if let Kind::Object { catchall, .. } = &mut n.kind {
                *catchall = Catchall::Strict;
            }
        })
    }
    pub fn loose(self) -> Self {
        self.edit(|n| {
            if let Kind::Object { catchall, .. } = &mut n.kind {
                *catchall = Catchall::Loose;
            }
        })
    }
    pub fn passthrough(self) -> Self {
        self.loose()
    }
    pub fn catchall(self, schema: Schema) -> Self {
        self.edit(|n| {
            if let Kind::Object { catchall, .. } = &mut n.kind {
                *catchall = Catchall::Schema(schema);
            }
        })
    }

    /// `.extend({...})` (a new object without the checks; zod 4 keeps none
    /// of the refinements either when they would be invalidated).
    pub fn extend<K: Into<String>>(&self, fields: Vec<(K, Schema)>) -> Schema {
        let (mut shape, catchall) = self.object_parts();
        for (k, v) in fields {
            let k = k.into();
            if let Some(slot) = shape.iter_mut().find(|(name, _)| *name == k) {
                slot.1 = v;
            } else {
                shape.push((k, v));
            }
        }
        node(Kind::Object { shape, catchall })
    }
    pub fn pick(&self, keys: &[&str]) -> Schema {
        let (shape, catchall) = self.object_parts();
        let shape = shape
            .into_iter()
            .filter(|(k, _)| keys.contains(&k.as_str()))
            .collect();
        node(Kind::Object { shape, catchall })
    }
    pub fn omit(&self, keys: &[&str]) -> Schema {
        let (shape, catchall) = self.object_parts();
        let shape = shape
            .into_iter()
            .filter(|(k, _)| !keys.contains(&k.as_str()))
            .collect();
        node(Kind::Object { shape, catchall })
    }
    pub fn partial(&self) -> Schema {
        let (shape, catchall) = self.object_parts();
        let shape = shape.into_iter().map(|(k, v)| (k, v.optional())).collect();
        node(Kind::Object { shape, catchall })
    }
    fn object_parts(&self) -> (Vec<(String, Schema)>, Catchall) {
        match &self.0.kind {
            Kind::Object { shape, catchall } => (shape.clone(), catchall.clone()),
            _ => panic!("not an object schema"),
        }
    }

    // ------------------------------------------------------- introspection

    /// Identity (`toBe` on a schema object).
    pub fn same(&self, other: &Schema) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn kind(&self) -> &Kind {
        &self.0.kind
    }

    pub fn description(&self) -> Option<&str> {
        self.0.description.as_deref()
    }

    pub fn meta_value(&self) -> Option<&Value> {
        self.0.meta.as_ref()
    }

    pub fn checks(&self) -> &[CheckEntry] {
        &self.0.checks
    }

    /// zod `def.type`.
    pub fn type_name(&self) -> &'static str {
        match &self.0.kind {
            Kind::String { .. } => "string",
            Kind::Number { .. } => "number",
            Kind::Boolean { .. } => "boolean",
            Kind::Null => "null",
            Kind::Undefined => "undefined",
            Kind::Never => "never",
            Kind::Any => "any",
            Kind::Unknown => "unknown",
            Kind::Date => "date",
            Kind::Literal(_) => "literal",
            Kind::Enum(_) => "enum",
            Kind::Array(_) => "array",
            Kind::Tuple(..) => "tuple",
            Kind::Object { .. } => "object",
            Kind::Record(..) => "record",
            Kind::Union(_) | Kind::DiscriminatedUnion(..) => "union",
            Kind::Intersection(..) => "intersection",
            Kind::Optional(_) => "optional",
            Kind::Nullable(_) => "nullable",
            Kind::NonOptional(_) => "nonoptional",
            Kind::Default(..) => "default",
            Kind::Prefault(..) => "prefault",
            Kind::Catch(..) => "catch",
            Kind::Pipe(..) | Kind::Preprocess(..) => "pipe",
            Kind::Transform(_) | Kind::TransformOpt(_) => "transform",
            Kind::Lazy(..) => "lazy",
            Kind::Custom(_) => "custom",
        }
    }

    /// The object's shape (`schema.shape`), in declaration order.
    pub fn shape(&self) -> Vec<(String, Schema)> {
        match &self.0.kind {
            Kind::Object { shape, .. } => shape.clone(),
            Kind::Lazy(..) => self.resolve_lazy().shape(),
            _ => panic!("schema has no shape ({})", self.type_name()),
        }
    }

    /// `schema.shape[key]`.
    pub fn field(&self, key: &str) -> Schema {
        self.shape()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no field {key}"))
    }

    pub fn has_field(&self, key: &str) -> bool {
        matches!(&self.0.kind, Kind::Object { shape, .. } if shape.iter().any(|(k, _)| k == key))
    }

    /// Enum values (`schema.options` of a `z.enum`).
    pub fn enum_values(&self) -> Vec<String> {
        match &self.0.kind {
            Kind::Enum(values) => values.clone(),
            _ => panic!("not an enum ({})", self.type_name()),
        }
    }

    /// Union options (`schema.options` of a union).
    pub fn options(&self) -> Vec<Schema> {
        match &self.0.kind {
            Kind::Union(o) | Kind::DiscriminatedUnion(_, o) => o.clone(),
            _ => panic!("not a union ({})", self.type_name()),
        }
    }

    /// Literal value(s) (`schema.value` / `schema.values`).
    pub fn literal_values(&self) -> Vec<Value> {
        match &self.0.kind {
            Kind::Literal(v) => v.clone(),
            _ => panic!("not a literal ({})", self.type_name()),
        }
    }

    /// `.unwrap()` of optional / nullable / default / catch / nonoptional.
    pub fn unwrap(&self) -> Schema {
        match &self.0.kind {
            Kind::Optional(i)
            | Kind::Nullable(i)
            | Kind::NonOptional(i)
            | Kind::Default(i, _)
            | Kind::Prefault(i, _)
            | Kind::Catch(i, _) => i.clone(),
            _ => panic!("cannot unwrap {}", self.type_name()),
        }
    }

    /// Array element (`schema.element`).
    pub fn element(&self) -> Schema {
        match &self.0.kind {
            Kind::Array(i) => i.clone(),
            _ => panic!("not an array ({})", self.type_name()),
        }
    }

    pub fn resolve_lazy(&self) -> Schema {
        match &self.0.kind {
            Kind::Lazy(f, cell) => cell.get_or_init(|| f()).clone(),
            _ => self.clone(),
        }
    }

    /// zod `_zod.optin`: `Some(true)` = defaulted, `Some(false)` = optional.
    fn optin(&self) -> Option<bool> {
        match &self.0.kind {
            Kind::Optional(i) => Some(i.optin() == Some(true)),
            Kind::Default(..) | Kind::Prefault(..) => Some(true),
            Kind::Catch(i, _) => Some(i.optin() == Some(true)),
            Kind::Nullable(i) => i.optin(),
            Kind::Pipe(i, _) => i.optin(),
            // A transform accepts `undefined` (zod `$ZodTransform` optin).
            Kind::Preprocess(..) | Kind::Transform(_) | Kind::TransformOpt(_) => Some(false),
            Kind::Lazy(..) => self.resolve_lazy().optin(),
            Kind::Union(o) | Kind::DiscriminatedUnion(_, o) => {
                if o.iter().any(|s| s.optin() == Some(true)) {
                    Some(true)
                } else if o.iter().any(|s| s.optin().is_some()) {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// zod `_zod.optout === "optional"`.
    fn optout(&self) -> bool {
        match &self.0.kind {
            Kind::Optional(_) => true,
            Kind::Nullable(i) | Kind::Catch(i, _) => i.optout(),
            Kind::Pipe(_, o) => o.optout(),
            Kind::Preprocess(_, i) => i.optout(),
            Kind::Lazy(..) => self.resolve_lazy().optout(),
            Kind::Union(o) | Kind::DiscriminatedUnion(_, o) => o.iter().any(Schema::optout),
            _ => false,
        }
    }

    /// The literal/enum values a discriminator slot accepts (`_zod.values`).
    fn values(&self) -> Option<Vec<Option<Value>>> {
        match &self.0.kind {
            Kind::Literal(v) => Some(v.iter().cloned().map(Some).collect()),
            Kind::Enum(v) => Some(v.iter().map(|s| Some(Value::String(s.clone()))).collect()),
            Kind::Null => Some(vec![Some(Value::Null)]),
            Kind::Undefined => Some(vec![None]),
            Kind::Optional(i) => i.values().map(|mut v| {
                v.push(None);
                v
            }),
            Kind::Nullable(i) => i.values().map(|mut v| {
                v.push(Some(Value::Null));
                v
            }),
            Kind::Default(i, _)
            | Kind::Prefault(i, _)
            | Kind::Catch(i, _)
            | Kind::NonOptional(i) => i.values(),
            Kind::Pipe(i, _) => i.values(),
            Kind::Lazy(..) => self.resolve_lazy().values(),
            Kind::Union(o) => {
                let mut all = Vec::new();
                for s in o {
                    all.extend(s.values()?);
                }
                Some(all)
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------- parsing

    /// `schema.safeParse(value)`.
    pub fn safe_parse(&self, value: &Value) -> SafeParse {
        self.safe_parse_opt(Some(value.clone()))
    }

    /// `safeParse(undefined)` when `value` is `None`.
    pub fn safe_parse_opt(&self, value: Option<Value>) -> SafeParse {
        let mut payload = Payload::new(value);
        self.run(&mut payload);
        if payload.issues.is_empty() {
            SafeParse {
                success: true,
                data: Some(payload.value.unwrap_or(Value::Null)),
                error: None,
            }
        } else {
            SafeParse {
                success: false,
                data: None,
                error: Some(ZodError {
                    issues: payload.issues.into_iter().map(RawIssue::finalize).collect(),
                }),
            }
        }
    }

    /// `schema.parse(value)`.
    pub fn parse(&self, value: &Value) -> Result<Value, ZodError> {
        let r = self.safe_parse(value);
        match r.error {
            Some(e) => Err(e),
            None => Ok(r.data.unwrap_or(Value::Null)),
        }
    }

    /// Runs the schema on `payload` (zod `_zod.run`).
    pub fn run(&self, payload: &mut Payload) {
        let n = &*self.0;
        let start = payload.issues.len();
        self.parse_base(payload);
        if let Some(err) = &n.error {
            for issue in &mut payload.issues[start..] {
                if issue.message.is_none() && issue.path.is_empty() {
                    issue.message = Some(err.clone());
                }
            }
        }
        if n.checks.is_empty() {
            return;
        }
        let mut is_aborted = payload.is_aborted_from(0);
        for entry in &n.checks {
            let length_based = matches!(
                entry.check,
                Check::MinLength(_) | Check::MaxLength(_) | Check::Length(_)
            );
            if length_based {
                if payload.explicitly_aborted() {
                    continue;
                }
                let has_length = matches!(
                    payload.value,
                    Some(Value::String(_)) | Some(Value::Array(_))
                );
                if !has_length {
                    continue;
                }
            } else if is_aborted {
                continue;
            }
            let len = payload.issues.len();
            run_check(entry, payload);
            if payload.issues.len() == len {
                continue;
            }
            for issue in &mut payload.issues[len..] {
                if issue.message.is_none() {
                    if let Some(m) = &entry.message {
                        issue.message = Some(m.clone());
                    } else if let Some(err) = &n.error {
                        issue.message = Some(err.clone());
                    }
                }
            }
            if !is_aborted {
                is_aborted = payload.is_aborted_from(len);
            }
        }
    }

    fn parse_base(&self, p: &mut Payload) {
        match &self.0.kind {
            Kind::String { coerce } => {
                if *coerce {
                    p.value = Some(Value::String(match p.value.take() {
                        None => "undefined".to_owned(),
                        Some(v) => js::string_of(&v),
                    }));
                }
                if !matches!(p.value, Some(Value::String(_))) {
                    p.issues
                        .push(RawIssue::invalid_type("string", p.value.as_ref()));
                }
            }
            Kind::Number { coerce, .. } => {
                if *coerce {
                    let n = match &p.value {
                        None => f64::NAN,
                        Some(Value::Null) => 0.0,
                        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
                        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
                        Some(Value::String(s)) => js::to_number(s),
                        Some(_) => f64::NAN,
                    };
                    p.value = Some(number_value(n));
                    if n.is_nan() {
                        let mut issue = RawIssue::invalid_type("number", None);
                        issue.input = Some(Value::Null);
                        issue.message =
                            Some("Invalid input: expected number, received NaN".to_owned());
                        p.issues.push(issue);
                        return;
                    }
                }
                match &p.value {
                    Some(Value::Number(_)) => {}
                    other => {
                        let issue = RawIssue::invalid_type("number", other.as_ref());
                        p.issues.push(issue);
                    }
                }
            }
            Kind::Boolean { coerce } => {
                if *coerce {
                    let b = match &p.value {
                        None | Some(Value::Null) => false,
                        Some(Value::Bool(b)) => *b,
                        Some(Value::Number(n)) => {
                            n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan())
                        }
                        Some(Value::String(s)) => !s.is_empty(),
                        Some(_) => true,
                    };
                    p.value = Some(Value::Bool(b));
                }
                if !matches!(p.value, Some(Value::Bool(_))) {
                    p.issues
                        .push(RawIssue::invalid_type("boolean", p.value.as_ref()));
                }
            }
            Kind::Null => {
                if !matches!(p.value, Some(Value::Null)) {
                    p.issues
                        .push(RawIssue::invalid_type("null", p.value.as_ref()));
                }
            }
            Kind::Undefined => {
                if p.value.is_some() {
                    p.issues
                        .push(RawIssue::invalid_type("undefined", p.value.as_ref()));
                }
            }
            Kind::Never => {
                p.issues
                    .push(RawIssue::invalid_type("never", p.value.as_ref()));
            }
            Kind::Any | Kind::Unknown => {}
            Kind::Date => {
                p.issues
                    .push(RawIssue::invalid_type("date", p.value.as_ref()));
            }
            Kind::Literal(values) => {
                let ok = p
                    .value
                    .as_ref()
                    .is_some_and(|v| values.iter().any(|x| json_eq(x, v)));
                if !ok {
                    let mut issue = RawIssue::new(IssueCode::InvalidValue);
                    issue.values = values.clone();
                    issue.input = p.value.clone();
                    p.issues.push(issue);
                }
            }
            Kind::Enum(values) => {
                let ok = matches!(&p.value, Some(Value::String(s)) if values.contains(s));
                if !ok {
                    let mut issue = RawIssue::new(IssueCode::InvalidValue);
                    issue.values = values.iter().map(|v| Value::String(v.clone())).collect();
                    issue.input = p.value.clone();
                    p.issues.push(issue);
                }
            }
            Kind::Array(item) => {
                let Some(Value::Array(items)) = &p.value else {
                    p.issues
                        .push(RawIssue::invalid_type("array", p.value.as_ref()));
                    return;
                };
                let items = items.clone();
                let mut out = Vec::with_capacity(items.len());
                for (i, v) in items.into_iter().enumerate() {
                    let mut r = Payload::new(Some(v));
                    item.run(&mut r);
                    prefix_into(p, i.into(), r.issues);
                    out.push(r.value.unwrap_or(Value::Null));
                }
                p.value = Some(Value::Array(out));
            }
            Kind::Tuple(items, rest) => self.parse_tuple(p, items, rest.as_ref()),
            Kind::Object { shape, catchall } => self.parse_object(p, shape, catchall),
            Kind::Record(key, value) => self.parse_record(p, key, value),
            Kind::Union(options) => parse_union(p, options),
            Kind::DiscriminatedUnion(disc, options) => parse_discriminated(p, disc, options),
            Kind::Intersection(left, right) => {
                let mut l = Payload::new(p.value.clone());
                left.run(&mut l);
                let mut r = Payload::new(p.value.clone());
                right.run(&mut r);
                let any_issues = !l.issues.is_empty() || !r.issues.is_empty();
                p.issues.append(&mut l.issues);
                p.issues.append(&mut r.issues);
                if any_issues {
                    return;
                }
                match merge_values(l.value, r.value) {
                    Some(v) => p.value = v,
                    None => {
                        p.issues.push(
                            RawIssue::new(IssueCode::Custom)
                                .with_message("Intersection results could not be merged"),
                        );
                    }
                }
            }
            Kind::Optional(inner) => {
                if p.value.is_none() {
                    if inner.optin() != Some(true) {
                        return;
                    }
                    let mut r = Payload::new(None);
                    inner.run(&mut r);
                    if r.issues.is_empty() {
                        p.value = r.value;
                    } else {
                        p.issues.extend(r.issues);
                    }
                    return;
                }
                inner.run(p);
            }
            Kind::Nullable(inner) => {
                if matches!(p.value, Some(Value::Null)) {
                    return;
                }
                inner.run(p);
            }
            Kind::NonOptional(inner) => {
                inner.run(p);
                if p.issues.is_empty() && p.value.is_none() {
                    p.issues.push(RawIssue::invalid_type("nonoptional", None));
                }
            }
            Kind::Default(inner, default) => {
                if p.value.is_none() {
                    p.value = Some(default.get());
                    return;
                }
                inner.run(p);
                if p.value.is_none() {
                    p.value = Some(default.get());
                }
            }
            Kind::Prefault(inner, default) => {
                if p.value.is_none() {
                    p.value = Some(default.get());
                }
                inner.run(p);
            }
            Kind::Catch(inner, value) => {
                let mut r = Payload::new(p.value.clone());
                inner.run(&mut r);
                if r.issues.is_empty() {
                    p.value = r.value;
                } else {
                    p.value = Some(value.clone());
                }
            }
            Kind::Pipe(left, right) => {
                left.run(p);
                if p.issues
                    .iter()
                    .any(|i| i.code != IssueCode::UnrecognizedKeys)
                {
                    p.aborted = true;
                    return;
                }
                right.run(p);
            }
            Kind::Preprocess(f, inner) => {
                p.value = f(p.value.take());
                inner.run(p);
            }
            Kind::Transform(f) => {
                let Some(v) = p.value.take() else {
                    // A transform on `undefined`: the TS callbacks used here
                    // all pass `undefined` through.
                    return;
                };
                let mut ctx = RefineCtx {
                    issues: Vec::new(),
                    input: Some(v.clone()),
                };
                let out = f(v, &mut ctx);
                p.value = Some(out);
                p.issues.extend(ctx.issues);
            }
            Kind::TransformOpt(f) => {
                let mut ctx = RefineCtx {
                    issues: Vec::new(),
                    input: p.value.clone(),
                };
                p.value = f(p.value.take(), &mut ctx);
                p.issues.extend(ctx.issues);
            }
            Kind::Lazy(..) => {
                self.resolve_lazy().run(p);
            }
            Kind::Custom(f) => {
                let ok = p.value.as_ref().is_some_and(|v| f(v));
                if !ok {
                    let mut issue = RawIssue::new(IssueCode::Custom);
                    issue.input = p.value.clone();
                    p.issues.push(issue);
                }
            }
        }
    }

    fn parse_tuple(&self, p: &mut Payload, items: &[Schema], rest: Option<&Schema>) {
        let Some(Value::Array(input)) = &p.value else {
            p.issues
                .push(RawIssue::invalid_type("tuple", p.value.as_ref()));
            return;
        };
        let input = input.clone();
        let optin_start = items
            .iter()
            .rposition(|s| s.optin().is_none())
            .map_or(0, |i| i + 1);
        let optout_start = items.iter().rposition(|s| !s.optout()).map_or(0, |i| i + 1);
        if rest.is_none() {
            if input.len() < optin_start {
                let mut issue = RawIssue::new(IssueCode::TooSmall);
                issue.minimum = Some(optin_start as f64);
                issue.origin = Some("array".to_owned());
                issue.input = p.value.clone();
                p.issues.push(issue);
                return;
            }
            if input.len() > items.len() {
                let mut issue = RawIssue::new(IssueCode::TooBig);
                issue.maximum = Some(items.len() as f64);
                issue.origin = Some("array".to_owned());
                issue.input = p.value.clone();
                p.issues.push(issue);
            }
        }
        let mut out: Vec<Value> = Vec::new();
        let mut results = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let mut r = Payload::new(input.get(i).cloned());
            item.run(&mut r);
            results.push(r);
        }
        let mut rest_out = Vec::new();
        if let Some(rest) = rest {
            for (offset, v) in input.iter().skip(items.len()).enumerate() {
                let mut r = Payload::new(Some(v.clone()));
                rest.run(&mut r);
                prefix_into(p, (items.len() + offset).into(), r.issues);
                rest_out.push(r.value.unwrap_or(Value::Null));
            }
        }
        // handleTupleResults: trailing optional-out items that are absent and
        // failed are dropped.
        for (i, r) in results.into_iter().enumerate() {
            let present = i < input.len();
            if !r.issues.is_empty() {
                if !present && i >= optout_start {
                    continue;
                }
                prefix_into(p, i.into(), r.issues);
            }
            if present || r.value.is_some() {
                while out.len() < i {
                    out.push(Value::Null);
                }
                out.push(r.value.unwrap_or(Value::Null));
            }
        }
        out.extend(rest_out);
        p.value = Some(Value::Array(out));
    }

    fn parse_object(&self, p: &mut Payload, shape: &[(String, Schema)], catchall: &Catchall) {
        let Some(Value::Object(input)) = &p.value else {
            p.issues
                .push(RawIssue::invalid_type("object", p.value.as_ref()));
            return;
        };
        let input = input.clone();
        let mut out = Map::new();
        for (key, schema) in shape {
            let present = input.contains_key(key);
            let mut r = Payload::new(input.get(key).cloned());
            schema.run(&mut r);
            let optin = schema.optin();
            let optout = schema.optout();
            handle_property(p, &mut out, key, present, optin, optout, r);
        }
        match catchall {
            Catchall::Strip => {}
            Catchall::Strict => {
                let unknown: Vec<String> = input
                    .keys()
                    .filter(|k| !shape.iter().any(|(s, _)| s == *k))
                    .cloned()
                    .collect();
                if !unknown.is_empty() {
                    let mut issue = RawIssue::new(IssueCode::UnrecognizedKeys);
                    issue.keys = unknown;
                    issue.cont = true;
                    issue.input = p.value.clone();
                    p.issues.push(issue);
                }
            }
            Catchall::Loose => {
                for (k, v) in &input {
                    if !shape.iter().any(|(s, _)| s == k) {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
            Catchall::Schema(schema) => {
                for (k, v) in &input {
                    if shape.iter().any(|(s, _)| s == k) {
                        continue;
                    }
                    let mut r = Payload::new(Some(v.clone()));
                    schema.run(&mut r);
                    handle_property(p, &mut out, k, true, schema.optin(), schema.optout(), r);
                }
            }
        }
        p.value = Some(Value::Object(out));
    }

    fn parse_record(&self, p: &mut Payload, key: &Schema, value: &Schema) {
        let Some(Value::Object(input)) = &p.value else {
            p.issues
                .push(RawIssue::invalid_type("record", p.value.as_ref()));
            return;
        };
        let input = input.clone();
        let mut out = Map::new();
        let exhaustive = key.values().filter(|v| v.iter().all(Option::is_some));
        if let Some(keys) = exhaustive {
            let mut known = BTreeSet::new();
            for k in keys.into_iter().flatten() {
                let ks = match &k {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                known.insert(ks.clone());
                let mut r = Payload::new(input.get(&ks).cloned());
                value.run(&mut r);
                prefix_into(p, ks.as_str().into(), r.issues);
                if let Some(v) = r.value {
                    out.insert(ks, v);
                }
            }
            let unknown: Vec<String> = input
                .keys()
                .filter(|k| !known.contains(*k))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                let mut issue = RawIssue::new(IssueCode::UnrecognizedKeys);
                issue.keys = unknown;
                issue.input = p.value.clone();
                p.issues.push(issue);
            }
        } else {
            for (k, v) in input {
                let mut kr = Payload::new(Some(Value::String(k.clone())));
                key.run(&mut kr);
                if !kr.issues.is_empty() {
                    let mut issue = RawIssue::new(IssueCode::InvalidKey);
                    issue.origin = Some("record".to_owned());
                    issue.path = vec![PathSegment::Key(k.clone())];
                    issue.input = Some(Value::String(k.clone()));
                    p.issues.push(issue);
                    continue;
                }
                let out_key = match kr.value {
                    Some(Value::String(s)) => s,
                    _ => k.clone(),
                };
                let mut r = Payload::new(Some(v));
                value.run(&mut r);
                prefix_into(p, k.as_str().into(), r.issues);
                out.insert(out_key, r.value.unwrap_or(Value::Null));
            }
        }
        p.value = Some(Value::Object(out));
    }
}

fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        json!(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn prefix_into(p: &mut Payload, seg: PathSegment, issues: Vec<RawIssue>) {
    for mut issue in issues {
        issue.path.insert(0, seg.clone());
        p.issues.push(issue);
    }
}

fn handle_property(
    p: &mut Payload,
    out: &mut Map<String, Value>,
    key: &str,
    present: bool,
    optin: Option<bool>,
    optout: bool,
    r: Payload,
) {
    if !present && optout && optin == Some(false) {
        return;
    }
    let had_issues = !r.issues.is_empty();
    if had_issues {
        if optin.is_some() && optout && !present {
            return;
        }
        prefix_into(p, key.into(), r.issues);
    }
    if !present && optin.is_none() {
        if !had_issues {
            let mut issue = RawIssue::invalid_type("nonoptional", None);
            issue.path = vec![key.into()];
            p.issues.push(issue);
        }
        return;
    }
    match r.value {
        None => {}
        Some(v) => {
            out.insert(key.to_owned(), v);
        }
    }
}

fn parse_union(p: &mut Payload, options: &[Schema]) {
    if options.len() == 1 {
        options[0].run(p);
        return;
    }
    let mut results = Vec::new();
    for option in options {
        let mut r = Payload::new(p.value.clone());
        option.run(&mut r);
        if r.issues.is_empty() {
            p.value = r.value;
            return;
        }
        results.push(r);
    }
    let nonaborted: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.is_aborted_from(0))
        .map(|(i, _)| i)
        .collect();
    if nonaborted.len() == 1 {
        let r = results.swap_remove(nonaborted[0]);
        p.value = r.value;
        p.issues.extend(r.issues);
        p.aborted = p.aborted || r.aborted;
        return;
    }
    let mut issue = RawIssue::new(IssueCode::InvalidUnion);
    issue.input = p.value.clone();
    issue.errors = results
        .into_iter()
        .map(|r| r.issues.into_iter().map(RawIssue::finalize).collect())
        .collect();
    p.issues.push(issue);
}

fn parse_discriminated(p: &mut Payload, disc: &str, options: &[Schema]) {
    let Some(Value::Object(input)) = &p.value else {
        p.issues
            .push(RawIssue::invalid_type("object", p.value.as_ref()));
        return;
    };
    let tag = input.get(disc).cloned();
    let mut seen: Vec<Value> = Vec::new();
    for option in options {
        let option = option.resolve_lazy();
        let values = discriminator_values(&option, disc);
        for v in &values {
            if let Some(v) = v
                && !seen.iter().any(|s| json_eq(s, v))
            {
                seen.push(v.clone());
            }
        }
        let hit = values.iter().any(|v| match (v, &tag) {
            (None, None) => true,
            (Some(a), Some(b)) => json_eq(a, b),
            _ => false,
        });
        if hit {
            option.run(p);
            return;
        }
    }
    let mut issue = RawIssue::new(IssueCode::InvalidUnion);
    issue.options = seen;
    issue.path = vec![disc.into()];
    issue.input = p.value.clone();
    p.issues.push(issue);
}

fn discriminator_values(option: &Schema, disc: &str) -> Vec<Option<Value>> {
    match option.kind() {
        Kind::Object { shape, .. } => shape
            .iter()
            .find(|(k, _)| k == disc)
            .and_then(|(_, s)| s.values())
            .unwrap_or_default(),
        Kind::Pipe(i, _) => discriminator_values(i, disc),
        Kind::Lazy(..) => discriminator_values(&option.resolve_lazy(), disc),
        Kind::Intersection(l, r) => {
            let mut v = discriminator_values(l, disc);
            v.extend(discriminator_values(r, disc));
            v
        }
        Kind::Union(o) | Kind::DiscriminatedUnion(_, o) => o
            .iter()
            .flat_map(|s| discriminator_values(s, disc))
            .collect(),
        _ => Vec::new(),
    }
}

fn merge_values(a: Option<Value>, b: Option<Value>) -> Option<Option<Value>> {
    match (a, b) {
        (None, None) => Some(None),
        (Some(a), Some(b)) => merge_json(a, b).map(Some),
        (Some(a), None) | (None, Some(a)) => Some(Some(a)),
    }
}

fn merge_json(a: Value, b: Value) -> Option<Value> {
    if a == b {
        return Some(a);
    }
    match (a, b) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(existing) => merge_json(existing, v)?,
                    None => v,
                };
                a.insert(k, merged);
            }
            Some(Value::Object(a))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return None;
            }
            a.into_iter()
                .zip(b)
                .map(|(x, y)| merge_json(x, y))
                .collect::<Option<Vec<_>>>()
                .map(Value::Array)
        }
        _ => None,
    }
}

// ------------------------------------------------------------------- checks

fn code_points(s: &str) -> usize {
    s.chars().count()
}

fn length_of(v: &Value) -> Option<(usize, &'static str)> {
    match v {
        Value::String(s) => Some((code_points(s), "string")),
        Value::Array(a) => Some((a.len(), "array")),
        _ => None,
    }
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$")
            .expect("email regex")
    })
}

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$")
            .expect("uuid regex")
    })
}

fn iso_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))$")
            .expect("date regex")
    })
}

fn time_source(precision: Option<i32>) -> String {
    let hhmm = "(?:[01]\\d|2[0-3]):[0-5]\\d";
    match precision {
        Some(-1) => hhmm.to_owned(),
        Some(0) => format!("{hhmm}:[0-5]\\d"),
        Some(p) => format!("{hhmm}:[0-5]\\d\\.\\d{{{p}}}"),
        None => format!("{hhmm}(?::[0-5]\\d(?:\\.\\d+)?)?"),
    }
}

fn iso_datetime_matches(s: &str, offset: bool, local: bool, precision: Option<i32>) -> bool {
    let date = iso_date_re()
        .as_str()
        .trim_start_matches('^')
        .trim_end_matches('$')
        .to_owned();
    let time = time_source(precision);
    let mut opts = vec!["Z".to_owned()];
    if local {
        opts.push(String::new());
    }
    if offset {
        opts.push("([+-](?:[01]\\d|2[0-3]):[0-5]\\d)".to_owned());
    }
    let tz = format!("({})", opts.join("|"));
    let re = Regex::new(&format!("^{date}T(?:{time}){tz}$")).expect("datetime regex");
    re.is_match(s)
}

fn run_check(entry: &CheckEntry, p: &mut Payload) {
    let fmt_issue = |format: &str, input: &Value| {
        let mut issue = RawIssue::new(IssueCode::InvalidFormat);
        issue.format = Some(format.to_owned());
        issue.origin = Some("string".to_owned());
        issue.input = Some(input.clone());
        issue.cont = true;
        issue
    };
    match &entry.check {
        Check::MinLength(n) => {
            let Some(v) = &p.value else { return };
            if let Some((len, origin)) = length_of(v)
                && len < *n
            {
                let mut issue = RawIssue::new(IssueCode::TooSmall);
                issue.minimum = Some(*n as f64);
                issue.origin = Some(origin.to_owned());
                issue.input = Some(v.clone());
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::MaxLength(n) => {
            let Some(v) = &p.value else { return };
            if let Some((len, origin)) = length_of(v)
                && len > *n
            {
                let mut issue = RawIssue::new(IssueCode::TooBig);
                issue.maximum = Some(*n as f64);
                issue.origin = Some(origin.to_owned());
                issue.input = Some(v.clone());
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::Length(n) => {
            let Some(v) = &p.value else { return };
            if let Some((len, origin)) = length_of(v)
                && len != *n
            {
                let mut issue = RawIssue::new(if len > *n {
                    IssueCode::TooBig
                } else {
                    IssueCode::TooSmall
                });
                if len > *n {
                    issue.maximum = Some(*n as f64);
                } else {
                    issue.minimum = Some(*n as f64);
                }
                issue.exact = true;
                issue.origin = Some(origin.to_owned());
                issue.input = Some(v.clone());
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::Gt(limit, inclusive) => {
            let Some(x) = p.value.as_ref().and_then(Value::as_f64) else {
                return;
            };
            let ok = if *inclusive { x >= *limit } else { x > *limit };
            if !ok {
                let mut issue = RawIssue::new(IssueCode::TooSmall);
                issue.minimum = Some(*limit);
                issue.inclusive = *inclusive;
                issue.origin = Some("number".to_owned());
                issue.input = p.value.clone();
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::Lt(limit, inclusive) => {
            let Some(x) = p.value.as_ref().and_then(Value::as_f64) else {
                return;
            };
            let ok = if *inclusive { x <= *limit } else { x < *limit };
            if !ok {
                let mut issue = RawIssue::new(IssueCode::TooBig);
                issue.maximum = Some(*limit);
                issue.inclusive = *inclusive;
                issue.origin = Some("number".to_owned());
                issue.input = p.value.clone();
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::MultipleOf(d) => {
            let Some(x) = p.value.as_ref().and_then(Value::as_f64) else {
                return;
            };
            let ratio = x / d;
            if (ratio - ratio.round()).abs() > 1e-9 {
                let mut issue = RawIssue::new(IssueCode::NotMultipleOf);
                issue.divisor = Some(*d);
                issue.origin = Some("number".to_owned());
                issue.input = p.value.clone();
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::Int => {
            let Some(x) = p.value.as_ref().and_then(Value::as_f64) else {
                return;
            };
            if x.fract() != 0.0 || !x.is_finite() {
                let mut issue = RawIssue::invalid_type("int", p.value.as_ref());
                issue.cont = false;
                p.issues.push(issue);
                return;
            }
            if x.abs() > 9_007_199_254_740_991.0 {
                let mut issue = RawIssue::new(if x > 0.0 {
                    IssueCode::TooBig
                } else {
                    IssueCode::TooSmall
                });
                if x > 0.0 {
                    issue.maximum = Some(9_007_199_254_740_991.0);
                } else {
                    issue.minimum = Some(-9_007_199_254_740_991.0);
                }
                issue.origin = Some("int".to_owned());
                issue.input = p.value.clone();
                issue.cont = true;
                p.issues.push(issue);
            }
        }
        Check::Regex(re, display) => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !re.is_match(s) {
                let mut issue = fmt_issue("regex", &Value::String(s.clone()));
                issue.pattern = Some(display.clone());
                p.issues.push(issue);
            }
        }
        Check::Email => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !email_re().is_match(s) {
                let issue = fmt_issue("email", &Value::String(s.clone()));
                p.issues.push(issue);
            }
        }
        Check::Uuid => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !uuid_re().is_match(s) {
                let issue = fmt_issue("uuid", &Value::String(s.clone()));
                p.issues.push(issue);
            }
        }
        Check::Url {
            hostname,
            protocol,
            normalize,
        } => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            let trimmed = js::trim(s).to_owned();
            let input = Value::String(s.clone());
            let http_guard = protocol
                .as_ref()
                .is_some_and(|re| re.as_str() == "^https?$");
            if !normalize && http_guard {
                let lower = trimmed.to_ascii_lowercase();
                if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                    p.issues.push(fmt_issue("url", &input));
                    return;
                }
            }
            let Ok(url) = url::Url::parse(&trimmed) else {
                p.issues.push(fmt_issue("url", &input));
                return;
            };
            let mut failed = false;
            if let Some(re) = hostname
                && !re.is_match(url.host_str().unwrap_or(""))
            {
                p.issues.push(fmt_issue("url", &input));
                failed = true;
            }
            if let Some(re) = protocol
                && !re.is_match(url.scheme())
            {
                p.issues.push(fmt_issue("url", &input));
                failed = true;
            }
            let _ = failed;
            let stripped: String = trimmed
                .chars()
                .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
                .collect();
            p.value = Some(Value::String(if *normalize {
                url.to_string()
            } else {
                stripped
            }));
        }
        Check::StartsWith(prefix) => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !s.starts_with(prefix.as_str()) {
                let mut issue = fmt_issue("starts_with", &Value::String(s.clone()));
                issue.prefix = Some(prefix.clone());
                p.issues.push(issue);
            }
        }
        Check::EndsWith(suffix) => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !s.ends_with(suffix.as_str()) {
                let mut issue = fmt_issue("ends_with", &Value::String(s.clone()));
                issue.suffix = Some(suffix.clone());
                p.issues.push(issue);
            }
        }
        Check::Includes(needle) => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !s.contains(needle.as_str()) {
                let mut issue = fmt_issue("includes", &Value::String(s.clone()));
                issue.includes = Some(needle.clone());
                p.issues.push(issue);
            }
        }
        Check::Lowercase => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if s.chars().any(char::is_uppercase) {
                p.issues
                    .push(fmt_issue("lowercase", &Value::String(s.clone())));
            }
        }
        Check::Uppercase => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if s.chars().any(char::is_lowercase) {
                p.issues
                    .push(fmt_issue("uppercase", &Value::String(s.clone())));
            }
        }
        Check::IsoDatetime {
            offset,
            local,
            precision,
        } => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !iso_datetime_matches(s, *offset, *local, *precision) {
                p.issues
                    .push(fmt_issue("datetime", &Value::String(s.clone())));
            }
        }
        Check::IsoDate => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            if !iso_date_re().is_match(s) {
                p.issues.push(fmt_issue("date", &Value::String(s.clone())));
            }
        }
        Check::IsoTime => {
            let Some(Value::String(s)) = &p.value else {
                return;
            };
            let re = Regex::new(&format!("^{}$", time_source(None))).expect("time regex");
            if !re.is_match(s) {
                p.issues.push(fmt_issue("time", &Value::String(s.clone())));
            }
        }
        Check::Trim => {
            if let Some(Value::String(s)) = &p.value {
                p.value = Some(Value::String(js::trim(s).to_owned()));
            }
        }
        Check::ToLowerCase => {
            if let Some(Value::String(s)) = &p.value {
                p.value = Some(Value::String(js::to_lower(s)));
            }
        }
        Check::ToUpperCase => {
            if let Some(Value::String(s)) = &p.value {
                p.value = Some(Value::String(js::to_upper(s)));
            }
        }
        Check::Overwrite(f) => {
            if let Some(v) = p.value.take() {
                let mut ctx = RefineCtx::default();
                p.value = Some(f(v, &mut ctx));
            }
        }
        Check::Refine {
            f,
            message,
            path,
            abort,
        } => {
            let v = p.value.clone().unwrap_or(Value::Null);
            if !f(&v) {
                let mut issue = RawIssue::new(IssueCode::Custom);
                issue.message = message.clone();
                issue.path = path.clone().unwrap_or_default();
                issue.input = p.value.clone();
                issue.cont = !abort;
                p.issues.push(issue);
            }
        }
        Check::SuperRefine(f, abort) => {
            let v = p.value.clone().unwrap_or(Value::Null);
            let mut ctx = RefineCtx {
                issues: Vec::new(),
                input: p.value.clone(),
            };
            f(&v, &mut ctx);
            for mut issue in ctx.issues {
                if *abort {
                    issue.cont = false;
                }
                p.issues.push(issue);
            }
        }
        Check::Raw(f) => f(p),
    }
}

// ------------------------------------------------------------ value helpers

/// `{ ...a, ...b }` for JSON objects (shallow).
pub fn spread(a: &Value, b: &Value) -> Value {
    let mut out = a.as_object().cloned().unwrap_or_default();
    if let Some(b) = b.as_object() {
        for (k, v) in b {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// Truthiness of a JSON value (JS `Boolean(x)`); `None` is `undefined`.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `z.toJSONSchema(schema)` (draft 2020-12, zod 4.6.7 output conventions).
pub fn to_json_schema(schema: &Schema) -> Value {
    let _ = schema;
    todo!("P3.4b")
}
