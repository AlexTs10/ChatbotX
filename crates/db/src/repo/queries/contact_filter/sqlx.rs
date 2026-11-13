//! Shared SQL pieces of the contact filter (`exists.ts`, value readers).

use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::repo::sql::Sql;

/// `"Contact"."<column>"`.
pub(crate) fn contact_col(column: &str) -> String {
    format!("\"Contact\".\"{column}\"")
}

/// An id bind: an integer literal binds as INTEGER, anything else as TEXT.
pub(crate) fn id_value(id: &str) -> Value {
    id.parse::<i64>()
        .map_or_else(|_| Value::Text(id.to_owned()), Value::Integer)
}

/// A JSON scalar as a bind (`undefined` / `null` / objects -> `None`).
pub(crate) fn json_scalar(v: &Json) -> Option<Value> {
    match v {
        Json::String(s) => Some(Value::Text(s.clone())),
        Json::Number(n) => Some(
            n.as_i64()
                .map_or_else(|| Value::Real(n.as_f64().unwrap_or(0.0)), Value::Integer),
        ),
        Json::Bool(b) => Some(Value::Integer(i64::from(*b))),
        _ => None,
    }
}

/// JS `String(value)` of a JSON value.
pub(crate) fn js_string(v: Option<&Json>) -> String {
    match v {
        None => "undefined".to_owned(),
        Some(Json::String(s)) => s.clone(),
        Some(Json::Null) => "null".to_owned(),
        Some(Json::Array(items)) => items
            .iter()
            .map(|i| match i {
                Json::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Json::Object(_)) => "[object Object]".to_owned(),
        Some(other) => other.to_string(),
    }
}

/// The value as a non-empty string.
pub(crate) fn non_empty_str(v: Option<&Json>) -> Option<&str> {
    match v {
        Some(Json::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// `toStringArrayValue`: an array (or one value) of non-empty strings.
pub(crate) fn string_values(v: Option<&Json>) -> Vec<String> {
    let items: Vec<&Json> = match v {
        Some(Json::Array(a)) => a.iter().collect(),
        Some(other) => vec![other],
        None => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|i| match i {
            Json::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// `getCustomFieldIntervalValue` / `getFieldIntervalValue`: `[lo, hi]` non-empty strings.
pub(crate) fn interval(v: Option<&Json>) -> Option<(String, String)> {
    match v {
        Some(Json::Array(a)) => match (a.first(), a.get(1)) {
            (Some(Json::String(lo)), Some(Json::String(hi)))
                if !lo.is_empty() && !hi.is_empty() =>
            {
                Some((lo.clone(), hi.clone()))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `<expr> IN (?, ?, ..)` (an empty list is SQLite's `IN ()`, false).
pub(crate) fn in_list(expr: &str, values: Vec<Value>, negate: bool) -> Sql {
    let marks = vec!["?"; values.len()].join(", ");
    let op = if negate { "NOT IN" } else { "IN" };
    Sql::new(format!("{expr} {op} ({marks})"), values)
}

/// `existsWhere`: `[NOT ]EXISTS (<inner>)`.
pub(crate) fn exists(inner: Sql, negate: bool) -> Sql {
    let mut out = Sql::raw(if negate { "NOT EXISTS (" } else { "EXISTS (" });
    out.append(inner);
    out.closed(")")
}

/// `joinTableExists`: `[NOT ]EXISTS (SELECT 1 FROM "<table>" WHERE "<table>"."<col>" =
/// "Contact"."id"[ AND <predicate>])`.
pub(crate) fn join_exists(
    table: &str,
    contact_column: &str,
    predicate: Option<Sql>,
    negate: bool,
) -> Sql {
    let mut inner = Sql::raw(format!(
        "SELECT 1 FROM \"{table}\" WHERE \"{table}\".\"{contact_column}\" = \"Contact\".\"id\""
    ));
    if let Some(p) = predicate {
        inner.push(" AND ");
        inner.append(p);
    }
    exists(inner, negate)
}

/// `contactInboxExists`.
pub(crate) fn contact_inbox_exists(predicate: Option<Sql>, negate: bool) -> Sql {
    join_exists("ContactInbox", "contactId", predicate, negate)
}

/// `conversationExists`.
pub(crate) fn conversation_exists(predicate: Option<Sql>, negate: bool) -> Sql {
    join_exists("Conversation", "contactId", predicate, negate)
}

/// `a OR b OR ..` without extra parentheses (`combineWithOr`); `None` for none.
pub(crate) fn join_raw(parts: Vec<Sql>, sep: &str) -> Option<Sql> {
    let mut iter = parts.into_iter();
    let mut out = iter.next()?;
    for p in iter {
        out.push(sep);
        out.append(p);
    }
    Some(out)
}
