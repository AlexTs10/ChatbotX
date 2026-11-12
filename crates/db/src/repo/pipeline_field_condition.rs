//! Port of `repositories/pipeline/field-condition.ts`: the SQL of one custom-field
//! condition on `PipelineItem.fieldValues`.
//!
//! SQLite form: the stored text (`fieldValues ->> id`) is
//! `CASE json_type(fv, ?) WHEN 'text' THEN fv ->> ? WHEN 'null' THEN NULL ELSE fv -> ? END`
//! (a JSON `true` reads as `true`, like Postgres `->>`); the key is bound as the JSON path
//! `$."<id>"`. `~` is the app `regexp`, `::numeric` is `CAST(.. AS REAL)`, the datetime day
//! is `tz_wall_ms` of `unixepoch(stored)`.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::types::Value;

use super::pipeline_item_cursor::bind1;
use super::sql::Sql;
use super::utils::escape_like_pattern;

/// `PipelineFieldOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFieldOperator {
    Is,
    IsNot,
    HasValue,
    NoValue,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Gt,
    Lt,
    Between,
}

/// `PipelineFieldConditionInput`: number operands are numeric strings, date operands
/// `YYYY-MM-DD`, boolean operands `true` / `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineFieldCondition {
    pub operator: PipelineFieldOperator,
    pub value: Option<String>,
    pub value_to: Option<String>,
}

impl PipelineFieldCondition {
    /// A condition with only an operator.
    pub fn op(operator: PipelineFieldOperator) -> PipelineFieldCondition {
        PipelineFieldCondition {
            operator,
            value: None,
            value_to: None,
        }
    }

    /// A condition with an operator and one operand.
    pub fn with(operator: PipelineFieldOperator, value: &str) -> PipelineFieldCondition {
        PipelineFieldCondition {
            operator,
            value: Some(value.to_owned()),
            value_to: None,
        }
    }

    /// A `between` condition (either bound may be absent).
    pub fn between(value: Option<&str>, value_to: Option<&str>) -> PipelineFieldCondition {
        PipelineFieldCondition {
            operator: PipelineFieldOperator::Between,
            value: value.map(str::to_owned),
            value_to: value_to.map(str::to_owned),
        }
    }
}

/// `CustomFieldType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomFieldType {
    ShortText,
    LongText,
    Email,
    PhoneNumber,
    Number,
    Date,
    Datetime,
    Boolean,
}

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Number,
    Date,
    Datetime,
    Boolean,
}

/// `PIPELINE_FIELD_COMPARISON_KIND`.
fn comparison_kind(field_type: CustomFieldType) -> Kind {
    match field_type {
        CustomFieldType::ShortText
        | CustomFieldType::LongText
        | CustomFieldType::Email
        | CustomFieldType::PhoneNumber => Kind::Text,
        CustomFieldType::Number => Kind::Number,
        CustomFieldType::Date => Kind::Date,
        CustomFieldType::Datetime => Kind::Datetime,
        CustomFieldType::Boolean => Kind::Boolean,
    }
}

const DAY_OF_MONTH: &str = "(0[13578]|1[02])-(0[1-9]|[12][0-9]|3[01])|(0[469]|11)-(0[1-9]|[12][0-9]|30)|02-(0[1-9]|1[0-9]|2[0-8])";
const YEAR: &str = "(000[1-9]|00[1-9][0-9]|0[1-9][0-9]{2}|[1-9][0-9]{3})";
const LEAP_YEAR: &str =
    "(([0-9]{2}(0[48]|[2468][048]|[13579][26]))|((0[48]|[2468][048]|[13579][26])00))";

/// `STORED_TEMPORAL_PATTERN`.
pub static STORED_TEMPORAL_PATTERN: LazyLock<String> = LazyLock::new(|| {
    format!(
        "^(({YEAR}-({DAY_OF_MONTH}))|({LEAP_YEAR}-02-29))T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{{1,6}})?(Z|[+-](0[0-9]|1[0-5]):[0-5][0-9])$"
    )
});
/// `NUMERIC_STORED_PATTERN`.
pub const NUMERIC_STORED_PATTERN: &str = "^-?[0-9]+(\\.[0-9]+)?(e[-+][0-9]{1,3})?$";
const FALLBACK_TIMEZONE: &str = "UTC";

static NUMERIC_OPERAND_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?\d{1,30}(\.\d{1,30})?$").expect("numeric operand"));
static DAY_OPERAND_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").expect("day operand"));

/// `isPipelineFilterDay`: a real calendar day written `YYYY-MM-DD` (year >= 1).
pub fn is_pipeline_filter_day(text: Option<&str>) -> bool {
    let Some(caps) = text.and_then(|t| DAY_OPERAND_PATTERN.captures(t)) else {
        return false;
    };
    let n = |i: usize| caps[i].parse::<i32>().unwrap_or(0);
    let (y, m, d) = (n(1), n(2), n(3));
    y >= 1
        && u32::try_from(m)
            .ok()
            .zip(u32::try_from(d).ok())
            .and_then(|(m, d)| chrono::NaiveDate::from_ymd_opt(y, m, d))
            .is_some()
}

/// `isPipelineFilterNumber`: a plain decimal number string.
pub fn is_pipeline_filter_number(text: Option<&str>) -> bool {
    text.is_some_and(|t| NUMERIC_OPERAND_PATTERN.is_match(t))
}

/// `isKnownTimezone`, else `UTC`.
fn known_timezone(timezone: Option<&str>) -> String {
    match timezone {
        Some(tz)
            if !tz.is_empty()
                && (tz.parse::<chrono_tz::Tz>().is_ok()
                    || crate::functions::tz_resolve(Some(tz)) != FALLBACK_TIMEZONE) =>
        {
            tz.to_owned()
        }
        _ => FALLBACK_TIMEZONE.to_owned(),
    }
}

/// The JSON path `$."<key>"` of a field id; `None` for a key the path syntax cannot hold
/// (such a key reads as absent).
pub fn field_key_path(key: &str) -> Option<String> {
    (!key.contains('"') && !key.contains('\\')).then(|| format!("$.\"{key}\""))
}

/// The stored value as text (`fieldValues ->> id`), with its bound path.
pub fn stored_sql(field_id: &str) -> Sql {
    let path = field_key_path(field_id).map_or(Value::Null, Value::Text);
    let fv = "\"PipelineItem\".\"fieldValues\"";
    Sql::new(
        format!(
            "(CASE json_type({fv}, ?) WHEN 'text' THEN {fv} ->> ? WHEN 'null' THEN NULL ELSE {fv} -> ? END)"
        ),
        vec![path.clone(), path.clone(), path],
    )
}

fn cat(parts: Vec<Sql>, texts: &[&str]) -> Sql {
    // texts.len() == parts.len() + 1: t0 p0 t1 p1 ... tn
    let mut out = Sql::raw(texts[0]);
    for (part, text) in parts.into_iter().zip(texts[1..].iter()) {
        out.append(part);
        out.push(text);
    }
    out
}

fn ordered(
    condition: &PipelineFieldCondition,
    subject: &Sql,
    operand_of: &dyn Fn(Option<&str>) -> Option<Sql>,
) -> Option<Sql> {
    let operand = operand_of(condition.value.as_deref());
    if condition.operator == PipelineFieldOperator::Between {
        let upper = operand_of(condition.value_to.as_deref());
        let bounds: Vec<Option<Sql>> = vec![
            operand.map(|o| cat(vec![subject.clone(), o], &["", " >= ", ""])),
            upper.map(|u| cat(vec![subject.clone(), u], &["", " <= ", ""])),
        ];
        if bounds.iter().all(Option::is_none) {
            return None;
        }
        return Some(Sql::and(bounds));
    }
    let operand = operand?;
    let op = match condition.operator {
        PipelineFieldOperator::Is => " = ",
        PipelineFieldOperator::IsNot => " <> ",
        PipelineFieldOperator::Gt => " > ",
        PipelineFieldOperator::Lt => " < ",
        _ => return None,
    };
    Some(cat(vec![subject.clone(), operand], &["", op, ""]))
}

fn guarded(stored: &Sql, pattern: &str, then: &str) -> Sql {
    // `case when regexp(pattern, S) then <then with S> end`
    let mut out = Sql::raw("(case when regexp(?, ");
    out.params.push(Value::Text(pattern.to_owned()));
    out.append(stored.clone());
    out.push(") then ");
    let (before, after) = then.split_once("{S}").unwrap_or((then, ""));
    out.push(before);
    out.append(stored.clone());
    out.push(after);
    out.push(" end)");
    out
}

fn number_strategy(stored: &Sql, condition: &PipelineFieldCondition) -> Option<Sql> {
    let subject = guarded(stored, NUMERIC_STORED_PATTERN, "CAST({S} AS REAL)");
    ordered(condition, &subject, &|text| {
        is_pipeline_filter_number(text)
            .then(|| bind1("CAST(? AS REAL)", text.unwrap_or_default().to_owned()))
    })
}

fn day_operand(text: Option<&str>) -> Option<Sql> {
    is_pipeline_filter_day(text).then(|| bind1("?", text.unwrap_or_default().to_owned()))
}

fn date_strategy(stored: &Sql, condition: &PipelineFieldCondition) -> Option<Sql> {
    let subject = guarded(stored, &STORED_TEMPORAL_PATTERN, "substr({S}, 1, 10)");
    ordered(condition, &subject, &day_operand)
}

fn datetime_strategy(
    stored: &Sql,
    condition: &PipelineFieldCondition,
    timezone: &str,
) -> Option<Sql> {
    // The instant's calendar day in `timezone` ("at time zone").
    let instant = guarded(
        stored,
        &STORED_TEMPORAL_PATTERN,
        "CAST(unixepoch({S}, 'subsec') * 1000 AS INTEGER)",
    );
    let mut subject = Sql::raw("strftime('%Y-%m-%d', tz_wall_ms(");
    subject.append(instant);
    subject.push(", tz_resolve(");
    subject.push_bind(timezone.to_owned());
    subject.push(")) / 1000.0, 'unixepoch')");
    ordered(condition, &subject, &day_operand)
}

fn like_operand(operator: PipelineFieldOperator, text: &str) -> String {
    let escaped = escape_like_pattern(text);
    match operator {
        PipelineFieldOperator::StartsWith => format!("{escaped}%"),
        PipelineFieldOperator::EndsWith => format!("%{escaped}"),
        _ => format!("%{escaped}%"),
    }
}

fn text_strategy(stored: &Sql, condition: &PipelineFieldCondition) -> Option<Sql> {
    let value = condition.value.as_deref().filter(|v| !v.is_empty())?;
    let s = || stored.clone();
    let v = || bind1("?", value.to_owned());
    match condition.operator {
        PipelineFieldOperator::Is => Some(cat(vec![s(), v()], &["", " = ", ""])),
        PipelineFieldOperator::IsNot => Some(cat(
            vec![s(), s(), s(), v()],
            &["(", " is not null and ", " <> '' and ", " <> ", ")"],
        )),
        PipelineFieldOperator::NotContains => {
            let p = bind1("?", like_operand(condition.operator, value));
            Some(cat(
                vec![s(), s(), s(), p],
                &[
                    "(",
                    " is not null and ",
                    " <> '' and not ilike(",
                    ", ",
                    "))",
                ],
            ))
        }
        PipelineFieldOperator::Contains
        | PipelineFieldOperator::StartsWith
        | PipelineFieldOperator::EndsWith => {
            let p = bind1("?", like_operand(condition.operator, value));
            Some(cat(vec![s(), p], &["coalesce(ilike(", ", ", "), 0)"]))
        }
        _ => None,
    }
}

fn boolean_strategy(stored: &Sql, condition: &PipelineFieldCondition) -> Option<Sql> {
    let value = condition
        .value
        .as_deref()
        .filter(|v| *v == "true" || *v == "false")?;
    let v = bind1("?", value.to_owned());
    match condition.operator {
        PipelineFieldOperator::Is => Some(cat(vec![stored.clone(), v], &["", " = ", ""])),
        PipelineFieldOperator::IsNot => Some(cat(
            vec![stored.clone(), stored.clone(), v],
            &["(", " in ('true', 'false') and ", " <> ", ")"],
        )),
        _ => None,
    }
}

/// `fieldConditionSql`: one custom-field condition; `None` when it lacks the operand it
/// needs or the operator does not apply to the field's kind. A missing key or an empty
/// string is "no value" for every kind.
pub fn field_condition_sql(
    field_id: &str,
    field_type: CustomFieldType,
    condition: &PipelineFieldCondition,
    timezone: Option<&str>,
) -> Option<Sql> {
    let stored = stored_sql(field_id);
    match condition.operator {
        PipelineFieldOperator::HasValue => {
            return Some(cat(
                vec![stored.clone(), stored],
                &["(", " is not null and ", " <> '')"],
            ));
        }
        PipelineFieldOperator::NoValue => {
            return Some(cat(
                vec![stored.clone(), stored],
                &["(", " is null or ", " = '')"],
            ));
        }
        _ => {}
    }
    match comparison_kind(field_type) {
        Kind::Text => text_strategy(&stored, condition),
        Kind::Number => number_strategy(&stored, condition),
        Kind::Date => date_strategy(&stored, condition),
        Kind::Datetime => datetime_strategy(&stored, condition, &known_timezone(timezone)),
        Kind::Boolean => boolean_strategy(&stored, condition),
    }
}
