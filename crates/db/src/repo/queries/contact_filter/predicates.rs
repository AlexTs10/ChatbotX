//! Port of `queries/contact-filter/predicates.ts` and `continent.ts`: column, aggregate,
//! date and boolean predicates.

use chrono_tz::Tz;
use rusqlite::types::Value;
use serde_json::Value as Json;

use super::continents::COUNTRY_CODES_BY_CONTINENT;
use super::sqlx::{
    contact_col, contact_inbox_exists, in_list, interval, join_raw, js_string, json_scalar,
    non_empty_str, string_values,
};
use super::value_format::{NUMERIC_VALUE, instant_window, is_valid_date_time_filter_value};
use crate::repo::sql::Sql;
use crate::repo::utils::{escape_like_pattern, like_contains};

const MINUTE: i64 = 60_000;

fn values_of(value: Option<&Json>) -> Vec<Value> {
    match value {
        Some(Json::Array(a)) => a.iter().filter_map(json_scalar).collect(),
        Some(v) => json_scalar(v).into_iter().collect(),
        None => Vec::new(),
    }
}

fn or_null(cond: Sql, col: &str) -> Sql {
    Sql::or([Some(cond), Some(Sql::raw(format!("{col} IS NULL")))])
}

/// `buildColumnWhere` on a `"Contact"` column.
pub(crate) fn column_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
    ci_eq: bool,
) -> Option<Sql> {
    let col = contact_col(column);
    let s = non_empty_str(value);
    if let Some(s) = s {
        let ilike = |pattern: String| Sql::bind(format!("ilike({col}, "), pattern).closed(")");
        match operator {
            "eq" if ci_eq => return Some(ilike(escape_like_pattern(s))),
            "ne" if ci_eq => {
                let mut not = Sql::raw("NOT ");
                not.append(ilike(escape_like_pattern(s)));
                return Some(or_null(not, &col));
            }
            "startsWith" => return Some(ilike(format!("{}%", escape_like_pattern(s)))),
            "endsWith" => return Some(ilike(format!("%{}", escape_like_pattern(s)))),
            _ => {}
        }
    }
    match operator {
        "isEmpty" if column == "gender" => return Some(Sql::raw(format!("{col} IS NULL"))),
        "isEmpty" => {
            return Some(Sql::or([
                Some(Sql::raw(format!("{col} IS NULL"))),
                Some(Sql::raw(format!("{col} = ''"))),
            ]));
        }
        "isNotEmpty" if column == "gender" => return Some(Sql::raw(format!("{col} IS NOT NULL"))),
        "isNotEmpty" => {
            return Some(Sql::and([
                Some(Sql::raw(format!("{col} IS NOT NULL"))),
                Some(Sql::raw(format!("{col} <> ''"))),
            ]));
        }
        "isBetween" | "notBetween" => return None,
        _ => {}
    }
    let is_array = matches!(value, Some(Json::Array(_)));
    let scalar = || value.and_then(json_scalar);
    let cmp = |op: &str| scalar().map(|v| Sql::bind(format!("{col} {op} "), v));
    let condition = match operator {
        "eq" if is_array => Some(in_list(&col, values_of(value), false)),
        "eq" => cmp("="),
        "ne" if is_array => Some(in_list(&col, values_of(value), true)),
        "ne" => cmp("<>"),
        "in" => Some(in_list(&col, values_of(value), false)),
        "notIn" => Some(in_list(&col, values_of(value), true)),
        "contains" => {
            Some(Sql::bind(format!("ilike({col}, "), like_contains(&js_string(value))).closed(")"))
        }
        "notContains" => Some(
            Sql::bind(
                format!("NOT ilike({col}, "),
                like_contains(&js_string(value)),
            )
            .closed(")"),
        ),
        "lt" => cmp("<"),
        "lte" => cmp("<="),
        "gt" => cmp(">"),
        "gte" => cmp(">="),
        _ => None,
    }?;
    Some(if matches!(operator, "ne" | "notIn" | "notContains") {
        or_null(condition, &col)
    } else {
        condition
    })
}

/// `resolveTimestamptzComparison`: a comparison of an integer-ms target to the value's
/// precision window.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TsCmp {
    Inside(i64, i64),
    Outside(i64, i64),
    AtLeast(i64),
    Before(i64),
}

impl TsCmp {
    pub(crate) fn render(self, target: &str) -> Sql {
        match self {
            TsCmp::Inside(lo, hi) => Sql::new(
                format!("({target} >= ? AND {target} < ?)"),
                vec![Value::Integer(lo), Value::Integer(hi)],
            ),
            TsCmp::Outside(lo, hi) => Sql::new(
                format!("({target} < ? OR {target} >= ? OR {target} IS NULL)"),
                vec![Value::Integer(lo), Value::Integer(hi)],
            ),
            TsCmp::AtLeast(x) => Sql::bind(format!("{target} >= "), x),
            TsCmp::Before(x) => Sql::bind(format!("{target} < "), x),
        }
    }
}

pub(crate) fn timestamp_comparison(
    operator: &str,
    value: Option<&Json>,
    zone: Tz,
) -> Option<TsCmp> {
    if matches!(operator, "isBetween" | "notBetween") {
        let (lo, hi) = date_interval(value)?;
        let lo_start = instant_window(&lo, zone)?.0;
        let hi_end = instant_window(&hi, zone)?.1;
        return Some(if operator == "isBetween" {
            TsCmp::Inside(lo_start, hi_end)
        } else {
            TsCmp::Outside(lo_start, hi_end)
        });
    }
    let s = non_empty_str(value).filter(|s| is_valid_date_time_filter_value(s))?;
    let (start, end) = instant_window(s, zone)?;
    match operator {
        "eq" => Some(TsCmp::Inside(start, end)),
        "ne" => Some(TsCmp::Outside(start, end)),
        "gt" => Some(TsCmp::AtLeast(end)),
        "gte" => Some(TsCmp::AtLeast(start)),
        "lt" => Some(TsCmp::Before(start)),
        "lte" => Some(TsCmp::Before(end)),
        _ => None,
    }
}

fn date_interval(value: Option<&Json>) -> Option<(String, String)> {
    match value {
        Some(Json::Array(a)) => match (a.first(), a.get(1)) {
            (Some(Json::String(lo)), Some(Json::String(hi)))
                if is_valid_date_time_filter_value(lo) && is_valid_date_time_filter_value(hi) =>
            {
                Some((lo.clone(), hi.clone()))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `buildDateColumnWhere`.
pub(crate) fn date_column_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
    zone: Tz,
) -> Option<Sql> {
    let col = contact_col(column);
    match operator {
        "isEmpty" => Some(Sql::raw(format!("{col} IS NULL"))),
        "isNotEmpty" => Some(Sql::raw(format!("{col} IS NOT NULL"))),
        _ => timestamp_comparison(operator, value, zone).map(|c| c.render(&col)),
    }
}

const LATEST: &str = "\"latestInteraction\".\"latest\"";

fn latest_aggregate_exists(column: &str, comparison: Sql) -> Sql {
    let mut out = Sql::raw(format!(
        "EXISTS (SELECT 1 FROM (SELECT MAX(\"ContactInbox\".\"{column}\") AS \"latest\" FROM \"ContactInbox\" WHERE \"ContactInbox\".\"contactId\" = \"Contact\".\"id\") AS \"latestInteraction\" WHERE "
    ));
    out.append(comparison);
    out.closed(")")
}

/// `buildLatestContactInboxDateWhere`.
pub(crate) fn latest_date_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
    zone: Tz,
) -> Option<Sql> {
    datetime_aggregate_comparison(operator, value, LATEST, zone)
        .map(|c| latest_aggregate_exists(column, c))
}

/// `buildDatetimeAggregateComparison`.
pub(crate) fn datetime_aggregate_comparison(
    operator: &str,
    value: Option<&Json>,
    latest: &str,
    zone: Tz,
) -> Option<Sql> {
    match operator {
        "isEmpty" => Some(Sql::raw(format!("{latest} IS NULL"))),
        "isNotEmpty" => Some(Sql::raw(format!("{latest} IS NOT NULL"))),
        _ => timestamp_comparison(operator, value, zone).map(|c| c.render(latest)),
    }
}

/// `buildLatestContactInboxNumberWhere`.
pub(crate) fn latest_number_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
) -> Option<Sql> {
    number_aggregate_comparison(operator, value, LATEST).map(|c| latest_aggregate_exists(column, c))
}

/// `Number(text)` as a bind (integer when exact).
pub(crate) fn number_value(text: &str) -> Value {
    text.parse::<i64>().map_or_else(
        |_| Value::Real(text.parse::<f64>().unwrap_or(0.0)),
        Value::Integer,
    )
}

/// `buildNumberAggregateComparison`.
pub(crate) fn number_aggregate_comparison(
    operator: &str,
    value: Option<&Json>,
    latest: &str,
) -> Option<Sql> {
    match operator {
        "isEmpty" => return Some(Sql::raw(format!("{latest} IS NULL"))),
        "isNotEmpty" => return Some(Sql::raw(format!("{latest} IS NOT NULL"))),
        "isBetween" | "notBetween" => {
            let (lo, hi) = interval(value)?;
            if !(NUMERIC_VALUE.is_match(&lo) && NUMERIC_VALUE.is_match(&hi)) {
                return None;
            }
            let params = vec![number_value(&lo), number_value(&hi)];
            return Some(if operator == "isBetween" {
                Sql::new(format!("({latest} >= ? AND {latest} <= ?)"), params)
            } else {
                Sql::new(
                    format!("({latest} < ? OR {latest} > ? OR {latest} IS NULL)"),
                    params,
                )
            });
        }
        _ => {}
    }
    let s = non_empty_str(value)?;
    if let Some(c) = text_search_comparison(
        operator,
        s,
        &format!("CAST({latest} AS TEXT)"),
        &format!("{latest} IS NULL"),
    ) {
        return Some(c);
    }
    if !NUMERIC_VALUE.is_match(s) {
        return None;
    }
    let n = number_value(s);
    let cmp = |op: &str| Some(Sql::bind(format!("{latest} {op} "), n.clone()));
    match operator {
        "eq" => cmp("="),
        "ne" => Some(
            Sql::bind(format!("({latest} <> "), n.clone())
                .closed(&format!(" OR {latest} IS NULL)")),
        ),
        "gt" => cmp(">"),
        "gte" => cmp(">="),
        "lt" => cmp("<"),
        "lte" => cmp("<="),
        _ => None,
    }
}

/// `buildLatestContactInboxMinutesAgoWhere`.
pub(crate) fn latest_minutes_ago_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
    now: i64,
) -> Option<Sql> {
    minutes_ago_comparison(operator, value, LATEST, now).map(|c| latest_aggregate_exists(column, c))
}

/// `buildLatestContactInboxTextWhere`: the column of the latest inbound contact inbox.
pub(crate) fn latest_text_where(column: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let comparison = text_value_comparison(operator, value, LATEST)?;
    let mut out = Sql::raw(format!(
        "EXISTS (SELECT 1 FROM (SELECT (SELECT \"ContactInbox\".\"{column}\" FROM \"ContactInbox\" WHERE \"ContactInbox\".\"contactId\" = \"Contact\".\"id\" AND \"ContactInbox\".\"lastIncomingMessageAt\" IS NOT NULL ORDER BY \"ContactInbox\".\"lastIncomingMessageAt\" DESC LIMIT 1) AS \"latest\") AS \"latestInteraction\" WHERE "
    ));
    out.append(comparison);
    Some(out.closed(")"))
}

/// `buildMinutesAgoWhere` on a `"Contact"` column.
pub(crate) fn minutes_ago_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
    now: i64,
) -> Option<Sql> {
    if !can_build_minutes_ago(operator, value) {
        return None;
    }
    Some(
        minutes_ago_comparison(operator, value, &contact_col(column), now)
            .unwrap_or_else(|| Sql::raw("FALSE")),
    )
}

fn non_negative_int(s: &str) -> Option<i64> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

fn is_text_search(operator: &str) -> bool {
    matches!(
        operator,
        "contains" | "notContains" | "startsWith" | "endsWith"
    )
}

fn can_build_minutes_ago(operator: &str, value: Option<&Json>) -> bool {
    match operator {
        "isEmpty" | "isNotEmpty" => true,
        "isBetween" | "notBetween" => interval(value).is_some_and(|(lo, hi)| {
            non_negative_int(&lo).is_some() && non_negative_int(&hi).is_some()
        }),
        _ => match non_empty_str(value) {
            None => false,
            Some(s) => is_text_search(operator) || non_negative_int(s).is_some(),
        },
    }
}

/// `buildMinutesAgoComparison` (`NOW() - make_interval(mins => n)` = the bound
/// `now - n * 60000`).
pub(crate) fn minutes_ago_comparison(
    operator: &str,
    value: Option<&Json>,
    t: &str,
    now: i64,
) -> Option<Sql> {
    let ago = |m: i64| Value::Integer(now - m * MINUTE);
    match operator {
        "isEmpty" => return Some(Sql::raw(format!("{t} IS NULL"))),
        "isNotEmpty" => return Some(Sql::raw(format!("{t} IS NOT NULL"))),
        "isBetween" | "notBetween" => {
            let (lo, hi) = interval(value)?;
            let (a, b) = (non_negative_int(&lo)?, non_negative_int(&hi)?);
            let params = vec![ago(a.max(b)), ago(a.min(b))];
            let between = format!("({t} >= ? AND {t} <= ?)");
            return Some(if operator == "isBetween" {
                Sql::new(between, params)
            } else {
                Sql::new(format!("(NOT {between} OR {t} IS NULL)"), params)
            });
        }
        _ => {}
    }
    let s = non_empty_str(value)?;
    // FLOOR((now - t) / 1 min) as text (floor, also for future instants).
    let minutes = Sql::new(
        format!("CAST(((? - {t}) - (((? - {t}) % 60000) + 60000) % 60000) / 60000 AS TEXT)"),
        vec![Value::Integer(now), Value::Integer(now)],
    );
    if is_text_search(operator) {
        return text_search_sql(operator, s, minutes, Sql::raw(format!("{t} IS NULL")));
    }
    let m = non_negative_int(s)?;
    let cmp = |op: &str, x: i64| Some(Sql::bind(format!("{t} {op} "), ago(x)));
    match operator {
        "gt" => cmp("<", m),
        "gte" => cmp("<=", m),
        "lt" => cmp(">", m),
        "lte" => cmp(">=", m),
        "eq" => Some(Sql::new(
            format!("({t} <= ? AND {t} > ?)"),
            vec![ago(m), ago(m + 1)],
        )),
        "ne" => Some(Sql::new(
            format!("(NOT ({t} <= ? AND {t} > ?) OR {t} IS NULL)"),
            vec![ago(m), ago(m + 1)],
        )),
        _ => None,
    }
}

/// `buildTextValueComparison`.
fn text_value_comparison(operator: &str, value: Option<&Json>, latest: &str) -> Option<Sql> {
    let t = format!("CAST({latest} AS TEXT)");
    match operator {
        "isEmpty" => return Some(Sql::raw(format!("({latest} IS NULL OR {t} = '')"))),
        "isNotEmpty" => return Some(Sql::raw(format!("({latest} IS NOT NULL AND {t} <> '')"))),
        _ => {}
    }
    let s = non_empty_str(value)?;
    match operator {
        "eq" => Some(Sql::bind(format!("ilike({t}, "), escape_like_pattern(s)).closed(")")),
        "ne" => Some(
            Sql::bind(format!("(NOT ilike({t}, "), escape_like_pattern(s))
                .closed(&format!(") OR {latest} IS NULL)")),
        ),
        _ => text_search_comparison(operator, s, &t, &format!("{latest} IS NULL OR {t} = ''")),
    }
}

/// `buildTextSearchComparison` on a text expression.
pub(crate) fn text_search_comparison(
    operator: &str,
    value: &str,
    text: &str,
    empty: &str,
) -> Option<Sql> {
    text_search_sql(operator, value, Sql::raw(text), Sql::raw(empty))
}

fn text_search_sql(operator: &str, value: &str, text: Sql, empty: Sql) -> Option<Sql> {
    let pattern = match operator {
        "contains" | "notContains" => like_contains(value),
        "startsWith" => format!("{}%", escape_like_pattern(value)),
        "endsWith" => format!("%{}", escape_like_pattern(value)),
        _ => return None,
    };
    let negate = operator == "notContains";
    let mut out = Sql::raw(if negate { "(NOT ilike(" } else { "ilike(" });
    out.append(text);
    out.push(", ");
    out.push_bind(pattern);
    out.push(")");
    if negate {
        out.push(" OR ");
        out.append(empty);
        out.push(")");
    }
    Some(out)
}

/// `buildBooleanFromTimestamp`.
pub(crate) fn boolean_from_timestamp(
    column: &str,
    operator: &str,
    value: Option<&Json>,
) -> Option<Sql> {
    let col = contact_col(column);
    let not_null = Sql::raw(format!("{col} IS NOT NULL"));
    let null = Sql::raw(format!("{col} IS NULL"));
    match operator {
        "isEmpty" => Some(null),
        "isNotEmpty" => Some(not_null),
        "eq" if is_true(value) => Some(not_null),
        "eq" => Some(null),
        _ => None,
    }
}

pub(crate) fn is_true(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::String(s)) if s == "true")
}

/// `buildBooleanColumn`.
pub(crate) fn boolean_column(column: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let col = contact_col(column);
    match operator {
        "isEmpty" => Some(Sql::raw(format!("{col} IS NULL"))),
        "eq" => Some(Sql::bind(format!("{col} = "), i64::from(is_true(value)))),
        _ => None,
    }
}

fn presence(info: &str) -> String {
    let col = contact_col(if info == "phone" {
        "phoneNumber"
    } else {
        "email"
    });
    format!("({col} IS NOT NULL AND {col} <> '')")
}

fn has_any_contact_info() -> String {
    format!("({} OR {})", presence("phone"), presence("email"))
}

/// `buildHasContactInfoWhere`.
pub(crate) fn has_contact_info_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator == "isEmpty" {
        return Some(Sql::raw(format!("NOT {}", has_any_contact_info())));
    }
    let raw: Vec<&Json> = match value {
        Some(Json::Array(a)) => a.iter().collect(),
        Some(v) => vec![v],
        None => vec![&Json::Null],
    };
    let mut selected: Vec<&str> = Vec::new();
    for v in raw {
        match v {
            Json::String(s) if matches!(s.as_str(), "phone" | "email" | "phoneAndEmail") => {
                if !selected.contains(&s.as_str()) {
                    selected.push(s);
                }
            }
            _ => return None,
        }
    }
    if selected.is_empty() {
        return None;
    }
    let any = format!(
        "({})",
        selected
            .iter()
            .map(|v| if *v == "phoneAndEmail" {
                format!("({} AND {})", presence("phone"), presence("email"))
            } else {
                presence(v)
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    );
    match operator {
        "in" => Some(Sql::raw(any)),
        "notIn" => Some(Sql::raw(format!("NOT {any}"))),
        _ => None,
    }
}

/// `buildExistingContactWhere`.
pub(crate) fn existing_contact_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    match operator {
        "eq" if is_true(value) => Some(Sql::raw(has_any_contact_info())),
        "eq" | "isEmpty" => Some(Sql::raw(format!("NOT {}", has_any_contact_info()))),
        _ => None,
    }
}

/// `buildExistsBooleanWhere`: `exists(yes, negate)` for yes / no answers.
pub(crate) fn exists_boolean_where(
    exists: impl FnOnce(Option<Sql>, bool) -> Sql,
    yes: Sql,
    operator: &str,
    value: Option<&Json>,
) -> Option<Sql> {
    let is_yes = (operator == "eq" && is_true(value)) || operator == "isNotEmpty";
    let is_no = (operator == "eq" && !is_true(value)) || operator == "isEmpty";
    if !(is_yes || is_no) {
        return None;
    }
    Some(exists(Some(yes), is_no))
}

/// `buildContactInboxTriStateBooleanWhere`.
pub(crate) fn contact_inbox_tri_state_where(
    column: &str,
    operator: &str,
    value: Option<&Json>,
) -> Option<Sql> {
    let col = format!("\"ContactInbox\".\"{column}\"");
    match (operator, value) {
        ("eq", Some(Json::String(s))) if s == "true" => Some(contact_inbox_exists(
            Some(Sql::raw(format!("{col} = 1"))),
            false,
        )),
        ("eq", Some(Json::String(s))) if s == "false" => Some(contact_inbox_exists(
            Some(Sql::raw(format!("{col} = 0"))),
            false,
        )),
        ("isEmpty", _) => Some(contact_inbox_exists(
            Some(Sql::raw(format!("{col} IS NOT NULL"))),
            true,
        )),
        _ => None,
    }
}

/// `buildLastCommentWhere`.
pub(crate) fn last_comment_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    let id = "\"ContactInbox\".\"lastCommentMessageId\"";
    match operator {
        "isEmpty" => {
            return Some(contact_inbox_exists(
                Some(Sql::raw(format!("{id} IS NOT NULL"))),
                true,
            ));
        }
        "isNotEmpty" => {
            return Some(contact_inbox_exists(
                Some(Sql::raw(format!("{id} IS NOT NULL"))),
                false,
            ));
        }
        _ => {}
    }
    let s = non_empty_str(value)?;
    let text = "\"Message\".\"text\"";
    let empty = format!("{text} IS NULL OR {text} = ''");
    let message = match operator {
        "eq" | "ne" => {
            Some(Sql::bind(format!("ilike({text}, "), escape_like_pattern(s)).closed(")"))
        }
        "notContains" => text_search_comparison("contains", s, text, &empty),
        _ => text_search_comparison(operator, s, text, &empty),
    }?;
    let mut predicate = Sql::raw(format!(
        "regexp('^[0-9]+$', {id}) AND EXISTS (SELECT 1 FROM \"Message\" WHERE \"Message\".\"id\" = CASE WHEN regexp('^[0-9]+$', {id}) THEN CAST({id} AS INTEGER) END AND "
    ));
    predicate.append(message);
    predicate.push(")");
    Some(contact_inbox_exists(
        Some(predicate),
        matches!(operator, "ne" | "notContains"),
    ))
}

/// `buildContinentWhere`.
pub(crate) fn continent_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator == "isEmpty" {
        return column_where("country", operator, value, false);
    }
    if !matches!(operator, "eq" | "ne") {
        return None;
    }
    let col = contact_col("country");
    let values = string_values(value);
    let mut codes: Vec<&str> = Vec::new();
    for v in &values {
        if let Some((_, list)) = COUNTRY_CODES_BY_CONTINENT.iter().find(|(c, _)| c == v) {
            for code in *list {
                if !codes.contains(code) {
                    codes.push(code);
                }
            }
        }
    }
    let text_values = |list: &[&str]| {
        list.iter()
            .map(|c| Value::Text((*c).to_owned()))
            .collect::<Vec<_>>()
    };
    let mut predicates = Vec::new();
    if !codes.is_empty() {
        let mut p = Sql::raw(format!("({col} IS NOT NULL AND {col} <> '' AND "));
        p.append(in_list(&col, text_values(&codes), false));
        predicates.push(p.closed(")"));
    }
    if values.iter().any(|v| v == "unknown") {
        let mapped: Vec<&str> = COUNTRY_CODES_BY_CONTINENT
            .iter()
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        let mut p = Sql::raw(format!("({col} IS NULL OR {col} = '' OR NOT ("));
        p.append(in_list(&col, text_values(&mapped), false));
        predicates.push(p.closed("))"));
    }
    let Some(selected) = join_raw(predicates, " OR ") else {
        return Some(Sql::raw("FALSE"));
    };
    Some(if operator == "eq" {
        selected
    } else {
        let mut out = Sql::raw("NOT (");
        out.append(selected);
        out.closed(")")
    })
}
