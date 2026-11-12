//! `packages/utils/src/json-path.ts`: a small JSONPath subset (`$.a.b[0]`,
//! `$["a-b"]`, bare `a.b.0`, dot-prop escapes `a\.b`).

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::js;

/// One path step: an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JsonPathSegment {
    Key(String),
    Index(u64),
}

fn index_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^(0|[1-9][0-9]{0,14})$").expect("valid regex"))
}

fn plain_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("valid regex"))
}

fn read_bare_key(path: &[char], start: usize) -> Option<(JsonPathSegment, usize)> {
    let mut key = String::new();
    let mut cursor = start;
    while cursor < path.len() && path[cursor] != '.' && path[cursor] != '[' {
        if path[cursor] == '\\' && cursor + 1 < path.len() {
            cursor += 1;
        }
        key.push(path[cursor]);
        cursor += 1;
    }
    (cursor != start).then_some((JsonPathSegment::Key(key), cursor))
}

fn read_double_quoted(path: &[char], start: usize) -> Option<(JsonPathSegment, usize)> {
    let literal_start = start + 1;
    let mut cursor = literal_start + 1;
    while cursor < path.len() && path[cursor] != '"' {
        cursor += if path[cursor] == '\\' { 2 } else { 1 };
    }
    if cursor >= path.len() || path.get(cursor + 1) != Some(&']') {
        return None;
    }
    let literal: String = path[literal_start..=cursor].iter().collect();
    let key: String = serde_json::from_str(&literal).ok()?;
    Some((JsonPathSegment::Key(key), cursor + 2))
}

fn index_of(path: &[char], needle: char, from: usize) -> Option<usize> {
    path.iter()
        .skip(from)
        .position(|c| *c == needle)
        .map(|p| p + from)
}

fn read_bracket(path: &[char], start: usize) -> Option<(JsonPathSegment, usize)> {
    match path.get(start + 1) {
        Some('"') => read_double_quoted(path, start),
        Some('\'') => {
            let close = index_of(path, '\'', start + 2)?;
            if path.get(close + 1) != Some(&']') {
                return None;
            }
            let key: String = path[start + 2..close].iter().collect();
            Some((JsonPathSegment::Key(key), close + 2))
        }
        _ => {
            let close = index_of(path, ']', start + 1)?;
            let index: String = path[start + 1..close].iter().collect();
            if !index_key_re().is_match(&index) {
                return None;
            }
            Some((JsonPathSegment::Index(index.parse().ok()?), close + 1))
        }
    }
}

fn read_segment(path: &[char], cursor: usize) -> Option<(JsonPathSegment, usize)> {
    if path[cursor] == '[' {
        return read_bracket(path, cursor);
    }
    read_bare_key(
        path,
        if path[cursor] == '.' {
            cursor + 1
        } else {
            cursor
        },
    )
}

/// `parseJsonPath`: the segments, or `None` for unsupported syntax.
pub fn parse_json_path(input: &str) -> Option<Vec<JsonPathSegment>> {
    let trimmed = js::trim(input);
    if trimmed.is_empty() {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let root = chars[0] == '$' && (chars.len() == 1 || chars[1] == '.' || chars[1] == '[');
    let path = if root { &chars[1..] } else { &chars[..] };
    let mut segments = Vec::new();
    let mut cursor = 0;
    while cursor < path.len() {
        let (segment, next) = read_segment(path, cursor)?;
        segments.push(segment);
        cursor = next;
    }
    Some(segments)
}

/// `formatJsonPath`: the canonical `$` form.
pub fn format_json_path(segments: &[JsonPathSegment]) -> String {
    segments
        .iter()
        .fold("$".to_owned(), |path, segment| match segment {
            JsonPathSegment::Index(i) => format!("{path}[{i}]"),
            JsonPathSegment::Key(key) if index_key_re().is_match(key) => format!("{path}[{key}]"),
            JsonPathSegment::Key(key) if plain_key_re().is_match(key) => format!("{path}.{key}"),
            JsonPathSegment::Key(key) => {
                format!("{path}[{}]", serde_json::to_string(key).unwrap_or_default())
            }
        })
}

/// `canonicalizeJsonPath`.
pub fn canonicalize_json_path(input: &str) -> Option<String> {
    parse_json_path(input).map(|segments| format_json_path(&segments))
}

/// `JsonPathEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPathEntry {
    pub segments: Vec<JsonPathSegment>,
    pub path: String,
    /// The primitive value as text (truncated); `None` for objects/arrays.
    pub preview: Option<String>,
}

/// `listJsonPathEntries`'s result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPathEntryListing {
    pub entries: Vec<JsonPathEntry>,
    pub truncated: bool,
}

/// `DEFAULT_MAX_ENTRIES`.
pub const DEFAULT_MAX_ENTRIES: usize = 2000;
/// `DEFAULT_MAX_PREVIEW_LENGTH`.
pub const DEFAULT_MAX_PREVIEW_LENGTH: usize = 120;

fn preview_of(value: &Value, max_length: usize) -> Option<String> {
    if value.is_object() || value.is_array() {
        return None;
    }
    let text = js::string_of(value);
    Some(if js::utf16_len(&text) > max_length {
        format!("{}…", js::slice_utf16(&text, 0, max_length))
    } else {
        text
    })
}

/// `listJsonPathEntries`: every node in document order (object keys in map
/// order), at most `max_entries`.
pub fn list_json_path_entries(
    root: &Value,
    max_entries: Option<usize>,
    max_preview_length: Option<usize>,
) -> JsonPathEntryListing {
    let max_entries = max_entries.unwrap_or(DEFAULT_MAX_ENTRIES);
    let max_preview = max_preview_length.unwrap_or(DEFAULT_MAX_PREVIEW_LENGTH);
    let mut entries = Vec::new();
    let mut pending: Vec<(&Value, Vec<JsonPathSegment>)> = vec![(root, Vec::new())];
    while let Some((value, segments)) = pending.pop() {
        if entries.len() >= max_entries {
            return JsonPathEntryListing {
                entries,
                truncated: true,
            };
        }
        entries.push(JsonPathEntry {
            path: format_json_path(&segments),
            preview: preview_of(value, max_preview),
            segments: segments.clone(),
        });
        let children: Vec<(JsonPathSegment, &Value)> = match value {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, item)| (JsonPathSegment::Index(i as u64), item))
                .collect(),
            Value::Object(map) => map
                .iter()
                .map(|(k, item)| (JsonPathSegment::Key(k.clone()), item))
                .collect(),
            _ => Vec::new(),
        };
        for (key, item) in children.into_iter().rev() {
            let mut child = segments.clone();
            child.push(key);
            pending.push((item, child));
        }
    }
    JsonPathEntryListing {
        entries,
        truncated: false,
    }
}

fn canonical_array_index(text: &str) -> Option<usize> {
    if text == "0"
        || (!text.starts_with('0') && !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
    {
        text.parse().ok()
    } else {
        None
    }
}

fn member<'a>(container: &'a Value, segment: &JsonPathSegment) -> Option<&'a Value> {
    match container {
        Value::Array(items) => {
            let index = match segment {
                JsonPathSegment::Index(i) => usize::try_from(*i).ok()?,
                JsonPathSegment::Key(k) => canonical_array_index(k)?,
            };
            items.get(index)
        }
        Value::Object(map) => match segment {
            JsonPathSegment::Index(i) => map.get(&i.to_string()),
            JsonPathSegment::Key(k) => map.get(k),
        },
        _ => None,
    }
}

/// `readJsonPath`: own members only; `None` when the path is invalid, the
/// payload is not an object/array, or nothing is there.
pub fn read_json_path<'a>(payload: &'a Value, path: &str) -> Option<&'a Value> {
    let segments = parse_json_path(path)?;
    if !(payload.is_object() || payload.is_array()) {
        return None;
    }
    let mut current = payload;
    for segment in &segments {
        current = member(current, segment)?;
    }
    Some(current)
}

/// `stringifyJsonPathValue`: strings as-is, the rest JSON, null → `None`.
pub fn stringify_json_path_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(js::json_stringify(other)),
    }
}

const MAX_JSONB_DEPTH: usize = 64;

/// `isJsonbStorable`: no NUL (Rust strings are always well formed), at
/// most 64 levels of nesting.
pub fn is_jsonb_storable(root: &Value) -> bool {
    let mut pending: Vec<(&Value, usize)> = vec![(root, 1)];
    while let Some((value, depth)) = pending.pop() {
        match value {
            Value::String(s) => {
                if s.contains('\0') {
                    return false;
                }
            }
            Value::Array(items) => {
                if depth > MAX_JSONB_DEPTH {
                    return false;
                }
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(map) => {
                if depth > MAX_JSONB_DEPTH {
                    return false;
                }
                for (key, item) in map {
                    if key.contains('\0') {
                        return false;
                    }
                    pending.push((item, depth + 1));
                }
            }
            _ => {}
        }
    }
    true
}

/// [`is_jsonb_storable`] for JSON text: text that does not parse (a lone
/// surrogate `\ud800`, which no Rust string can hold) is not storable.
pub fn is_jsonb_storable_json(text: &str) -> bool {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => is_jsonb_storable(&value),
        Err(_) => false,
    }
}
