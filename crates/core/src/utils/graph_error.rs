//! `packages/utils/src/graph-error.ts`: one Graph failure as the stored /
//! displayed string, e.g. `(#10 - 1893063) <message>. <user message>`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::js;

/// `GraphErrorSource`. `code` / `sub_code` are the raw JSON values (a
/// number, or a string such as `"33"` or our own `"messengerError"`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphErrorSource {
    pub code: Option<Value>,
    pub sub_code: Option<Value>,
    pub message: Option<String>,
    pub user_message: Option<String>,
}

fn to_code(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Null => return None,
        Value::String(s) if js::trim(s).is_empty() => return None,
        Value::String(s) => js::to_number(s),
        Value::Number(n) => n.as_f64()?,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => return None,
    };
    (parsed.is_finite() && parsed != -1.0).then_some(parsed)
}

fn without_meta_code_prefix(part: Option<&str>, code: Option<f64>) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^\(#([0-9]+)\)\s*").expect("valid regex"));
    let part = part?;
    let Some(code) = code else {
        return Some(part.to_owned());
    };
    match re.captures(part) {
        Some(caps) if js::to_number(&caps[1]) == code => Some(part[caps[0].len()..].to_owned()),
        _ => Some(part.to_owned()),
    }
}

fn join_sentences(first: &str, second: &str) -> String {
    if first.ends_with(['.', '!', '?', '…']) {
        format!("{first} {second}")
    } else {
        format!("{first}. {second}")
    }
}

/// `formatGraphErrorMessage`: `None` when the body carried no text.
pub fn format_graph_error_message(source: &GraphErrorSource) -> Option<String> {
    let code = to_code(source.code.as_ref());
    let mut unique: Vec<String> = Vec::new();
    for part in [source.message.as_deref(), source.user_message.as_deref()] {
        if let Some(text) = without_meta_code_prefix(part, code) {
            let text = js::trim(&text).to_owned();
            if !text.is_empty() && !unique.contains(&text) {
                unique.push(text);
            }
        }
    }
    let first = unique.first()?.clone();
    let body = unique[1..]
        .iter()
        .fold(first, |acc, s| join_sentences(&acc, s));
    let prefix = match code {
        None => String::new(),
        Some(code) => match to_code(source.sub_code.as_ref()) {
            None => format!("(#{}) ", js::number_to_string(code)),
            Some(sub) => format!(
                "(#{} - {}) ",
                js::number_to_string(code),
                js::number_to_string(sub)
            ),
        },
    };
    Some(format!("{prefix}{body}"))
}

/// `formatGraphError`: a raw Graph `error` object (`code`,
/// `error_subcode`/`subcode`, `message`, `error_user_msg`/`error_user_title`).
pub fn format_graph_error(error: Option<&Value>) -> Option<String> {
    let error = error?.as_object()?;
    let pick = |a: &str, b: &str| -> Option<Value> {
        match error.get(a) {
            Some(Value::Null) | None => error.get(b).cloned(),
            Some(v) => Some(v.clone()),
        }
    };
    let text = |v: Option<Value>| v.and_then(|v| v.as_str().map(str::to_owned));
    format_graph_error_message(&GraphErrorSource {
        code: error.get("code").cloned(),
        sub_code: pick("error_subcode", "subcode"),
        message: text(error.get("message").cloned()),
        user_message: text(pick("error_user_msg", "error_user_title")),
    })
}
