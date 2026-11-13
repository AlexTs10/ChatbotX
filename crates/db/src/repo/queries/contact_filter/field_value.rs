//! Port of `field-value-predicates.ts`, `custom-field-predicates.ts` and
//! `bot-field-predicates.ts`: a predicate on a stored text value (`ContactCustomField.value`,
//! `BotField.value`).
//!
//! SQLite forms: `::numeric` is `CAST(.. AS REAL)` (DECISIONS 2026-11-13), the
//! `pg_input_is_valid(.., 'timestamptz')` guard is "starts with `YYYY-MM-DD` and
//! `unixepoch()` reads it", a `::timestamptz` instant is integer unix ms of
//! `unixepoch(value, 'subsec')`, a `::timestamp` wall clock is `unixepoch()` of the first 19
//! characters (offset ignored, as the Postgres cast).

use chrono_tz::Tz;
use rusqlite::types::Value;
use serde_json::Value as Json;

use super::FilterCondition;
use super::sqlx::{exists, id_value, interval, non_empty_str};
use super::value_format::{
    NUMERIC_VALUE, date_part, has_explicit_offset, has_time_component, instant_window,
    is_valid_date_time_filter_value, wall_day, wall_window,
};
use crate::repo::sql::Sql;
use crate::repo::utils::{escape_like_pattern, like_contains};

const TRUTHY: &str = "'t', 'true', 'y', 'yes', 'on', '1'";
const FALSY: &str = "'f', 'false', 'n', 'no', 'off', '0'";
const BOOLEAN_LITERAL_PATTERN: &str = "^(t|true|y|yes|on|1|f|false|n|no|off|0)$";

/// `resolveFieldValueNegation`.
pub(crate) fn resolve_negation(operator: &str) -> (&str, bool) {
    match operator {
        "ne" => ("eq", true),
        "notContains" => ("contains", true),
        "notBetween" => ("isBetween", true),
        "isEmpty" => ("isNotEmpty", true),
        other => (other, false),
    }
}

/// `buildFieldValuePositivePredicate`.
pub(crate) fn positive_predicate(
    col: &str,
    operator: &str,
    value: Option<&Json>,
    field_type: Option<&str>,
    value_type: Option<&str>,
    zone: Tz,
) -> Option<Sql> {
    if operator == "isNotEmpty" {
        return Some(Sql::raw(format!("({col} IS NOT NULL AND {col} <> '')")));
    }
    match value_type {
        Some("number") => number_predicate(col, operator, value),
        Some("boolean") => boolean_predicate(col, operator, value),
        Some("datetime") => {
            let field_type = field_type?;
            temporal_branch(col, operator, value, field_type == "date", zone)
        }
        _ => text_predicate(col, operator, value),
    }
}

fn ilike(col: &str, pattern: String) -> Sql {
    Sql::bind(format!("ilike({col}, "), pattern).closed(")")
}

fn text_search(col: &str, operator: &str, s: &str) -> Option<Sql> {
    match operator {
        "contains" => Some(ilike(col, like_contains(s))),
        "startsWith" => Some(ilike(col, format!("{}%", escape_like_pattern(s)))),
        "endsWith" => Some(ilike(col, format!("%{}", escape_like_pattern(s)))),
        _ => None,
    }
}

fn number(s: &str) -> Value {
    Value::Real(s.parse::<f64>().unwrap_or(0.0))
}

fn number_predicate(col: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let numeric = format!("CAST(NULLIF({col}, '') AS REAL)");
    let guard = format!("regexp('^-?[0-9]+(\\.[0-9]+)?$', {col})");
    if operator == "isBetween" {
        let (lo, hi) = interval(value)?;
        if !(NUMERIC_VALUE.is_match(&lo) && NUMERIC_VALUE.is_match(&hi)) {
            return None;
        }
        return Some(Sql::new(
            format!("({guard} AND {numeric} >= ? AND {numeric} <= ?)"),
            vec![number(&lo), number(&hi)],
        ));
    }
    let s = non_empty_str(value)?;
    if let Some(p) = text_search(col, operator, s) {
        return Some(p);
    }
    if !NUMERIC_VALUE.is_match(s) {
        return None;
    }
    let op = match operator {
        "eq" => "=",
        "gt" => ">",
        "gte" => ">=",
        "lt" => "<",
        "lte" => "<=",
        _ => return None,
    };
    Some(Sql::bind(format!("({guard} AND {numeric} {op} "), number(s)).closed(")"))
}

fn canonical_boolean(raw: &str) -> Option<bool> {
    let s = raw.trim().to_lowercase();
    match s.as_str() {
        "t" | "true" | "y" | "yes" | "on" | "1" => Some(true),
        "f" | "false" | "n" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn boolean_predicate(col: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator != "eq" {
        return None;
    }
    let s = non_empty_str(value)?;
    let Some(b) = canonical_boolean(s) else {
        return Some(Sql::raw("FALSE"));
    };
    let norm = format!("lower_u(trim({col}))");
    let set = if b { TRUTHY } else { FALSY };
    Some(Sql::raw(format!(
        "(regexp('{BOOLEAN_LITERAL_PATTERN}', {norm}) AND {norm} IN ({set}))"
    )))
}

fn text_predicate(col: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let s = non_empty_str(value)?;
    if operator == "eq" {
        return Some(Sql::bind(format!("{col} = "), s.to_owned()));
    }
    text_search(col, operator, s)
}

/// `buildDatetimeGuard`.
pub(crate) fn datetime_guard(col: &str) -> String {
    format!(
        "({col} IS NOT NULL AND {col} <> '' AND {col} GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*' AND unixepoch({col}) IS NOT NULL)"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cast {
    /// Wall clock (`::timestamp`): text operands.
    Wall,
    /// Instant (`::timestamptz`): integer ms operands.
    Instant,
}

enum Operand {
    Wall(String),
    Instant(i64),
}

impl Operand {
    fn push(self, out: &mut Sql) {
        match self {
            Operand::Wall(t) => {
                out.push("unixepoch(");
                out.push_bind(t);
                out.push(", 'subsec')");
            }
            Operand::Instant(ms) => {
                out.push_bind(ms);
            }
        }
    }
}

fn stored(col: &str, guard: &str, cast: Cast) -> String {
    match cast {
        Cast::Wall => {
            format!("CASE WHEN {guard} THEN unixepoch(substr({col}, 1, 19), 'subsec') END")
        }
        Cast::Instant => {
            format!(
                "CASE WHEN {guard} THEN CAST(round(unixepoch({col}, 'subsec') * 1000) AS INTEGER) END"
            )
        }
    }
}

fn compare(col: &str, guard: &str, cast: Cast, parts: Vec<(&str, Operand)>) -> Sql {
    let s = stored(col, guard, cast);
    let mut out = Sql::raw(format!("({guard}"));
    for (op, operand) in parts {
        out.push(&format!(" AND {s} {op} "));
        operand.push(&mut out);
    }
    out.closed(")")
}

fn window_comparison(
    col: &str,
    operator: &str,
    guard: &str,
    start: Operand,
    end: Operand,
    cast: Cast,
) -> Option<Sql> {
    Some(match operator {
        "eq" => compare(col, guard, cast, vec![(">=", start), ("<", end)]),
        "gt" => compare(col, guard, cast, vec![(">=", end)]),
        "gte" => compare(col, guard, cast, vec![(">=", start)]),
        "lt" => compare(col, guard, cast, vec![("<", start)]),
        "lte" => compare(col, guard, cast, vec![("<", end)]),
        _ => return None,
    })
}

fn instant_comparison(
    col: &str,
    operator: &str,
    value: &str,
    zone: Tz,
    guard: &str,
) -> Option<Sql> {
    let (start, end) = instant_window(value, zone)?;
    window_comparison(
        col,
        operator,
        guard,
        Operand::Instant(start),
        Operand::Instant(end),
        Cast::Instant,
    )
}

/// `buildTemporalBranchPredicate`.
fn temporal_branch(
    col: &str,
    operator: &str,
    value: Option<&Json>,
    is_date: bool,
    zone: Tz,
) -> Option<Sql> {
    let guard = datetime_guard(col);
    if operator == "isBetween" {
        return temporal_range(col, value, is_date, zone, &guard);
    }
    let s = non_empty_str(value).filter(|s| is_valid_date_time_filter_value(s))?;
    if !is_date || has_explicit_offset(s) {
        return instant_comparison(col, operator, s, zone, &guard);
    }
    if has_time_component(s) {
        let (start, end) = wall_window(s)?;
        return window_comparison(
            col,
            operator,
            &guard,
            Operand::Wall(start),
            Operand::Wall(end),
            Cast::Wall,
        );
    }
    // Naive date only.
    if operator == "eq" {
        return Some(
            Sql::bind(
                format!("({col} IS NOT NULL AND {col} <> '' AND substr({col}, 1, 10) = "),
                date_part(s)?,
            )
            .closed(")"),
        );
    }
    let (day_start, day_end) = wall_day(s)?;
    let (op, operand) = match operator {
        "gt" => (">=", day_end),
        "gte" => (">=", day_start),
        "lt" => ("<", day_start),
        "lte" => ("<", day_end),
        _ => return None,
    };
    Some(compare(
        col,
        &guard,
        Cast::Wall,
        vec![(op, Operand::Wall(operand))],
    ))
}

fn temporal_range(
    col: &str,
    value: Option<&Json>,
    is_date: bool,
    zone: Tz,
    guard: &str,
) -> Option<Sql> {
    let (lo, hi) = interval(value)?;
    if !(is_valid_date_time_filter_value(&lo) && is_valid_date_time_filter_value(&hi)) {
        return None;
    }
    let instant = !is_date || (has_explicit_offset(&lo) && has_explicit_offset(&hi));
    if instant {
        let lo_start = instant_window(&lo, zone)?.0;
        let hi_end = instant_window(&hi, zone)?.1;
        Some(compare(
            col,
            guard,
            Cast::Instant,
            vec![
                (">=", Operand::Instant(lo_start)),
                ("<", Operand::Instant(hi_end)),
            ],
        ))
    } else {
        let lo_start = wall_window(&lo)?.0;
        let hi_end = wall_window(&hi)?.1;
        Some(compare(
            col,
            guard,
            Cast::Wall,
            vec![
                (">=", Operand::Wall(lo_start)),
                ("<", Operand::Wall(hi_end)),
            ],
        ))
    }
}

const CF_VALUE: &str = "\"ContactCustomField\".\"value\"";

/// `buildCustomFieldWhere`.
pub(crate) fn custom_field_where(c: &FilterCondition, zone: Tz) -> Option<Sql> {
    let id = c.custom_field_id.as_deref().filter(|s| !s.is_empty())?;
    let (positive, negate) = resolve_negation(&c.operator);
    let value = c.value.as_ref();
    let predicate = if c.value_type.as_deref() == Some("datetime")
        && c.custom_field_type.is_none()
        && positive != "isNotEmpty"
    {
        legacy_temporal(id, positive, value, zone)
    } else {
        positive_predicate(
            CF_VALUE,
            positive,
            value,
            c.custom_field_type.as_deref(),
            c.value_type.as_deref(),
            zone,
        )
    }?;
    let mut inner = Sql::bind(
        "SELECT 1 FROM \"ContactCustomField\" WHERE \"ContactCustomField\".\"contactId\" = \"Contact\".\"id\" AND \"ContactCustomField\".\"customFieldId\" = ",
        id_value(id),
    );
    inner.push(" AND ");
    inner.append(predicate);
    Some(exists(inner, negate))
}

fn legacy_temporal(id: &str, operator: &str, value: Option<&Json>, zone: Tz) -> Option<Sql> {
    let date = temporal_branch(CF_VALUE, operator, value, true, zone)?;
    let datetime = temporal_branch(CF_VALUE, operator, value, false, zone)?;
    let is_date = || {
        Sql::bind(
            "EXISTS (SELECT 1 FROM \"CustomField\" WHERE \"CustomField\".\"id\" = ",
            id_value(id),
        )
        .closed(" AND \"CustomField\".\"type\" = 'date')")
    };
    let mut out = Sql::raw("((");
    out.append(is_date());
    out.push(" AND ");
    out.append(date);
    out.push(") OR (NOT ");
    out.append(is_date());
    out.push(" AND ");
    out.append(datetime);
    Some(out.closed("))"))
}

/// `buildBotFieldWhere` (workspace-level value, not per contact).
pub(crate) fn bot_field_where(
    c: &FilterCondition,
    zone: Tz,
    workspace_id: Option<i64>,
) -> Option<Sql> {
    let id = c.bot_field_id.as_deref().filter(|s| !s.is_empty())?;
    let workspace_id = workspace_id?;
    let (positive, negate) = resolve_negation(&c.operator);
    let predicate = positive_predicate(
        "\"BotField\".\"value\"",
        positive,
        c.value.as_ref(),
        c.bot_field_type.as_deref(),
        c.value_type.as_deref(),
        zone,
    )?;
    let mut inner = Sql::new(
        "SELECT 1 FROM \"BotField\" WHERE \"BotField\".\"workspaceId\" = ? AND \"BotField\".\"id\" = ?",
        vec![Value::Integer(workspace_id), id_value(id)],
    );
    inner.push(" AND ");
    inner.append(predicate);
    Some(exists(inner, negate))
}
