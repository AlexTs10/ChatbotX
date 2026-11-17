//! `trigger/services/condition-evaluator.ts` (`ConditionEvaluator`) and the
//! `parseDateTimeValue` of `trigger/utils/datetime-calculator.ts`: one
//! trigger / webhook condition against one event. Shared by the trigger and
//! the webhook workers.

use chatbotx_core::flow_config::z::truthy;
use chatbotx_core::js;
use chatbotx_core::utils::datetime::{is_real_calendar_date, to_zoned_wall_clock};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value};

/// The condition columns the evaluator reads.
#[derive(Debug, Clone, Default)]
pub struct Condition {
    pub r#type: String,
    pub source_id: Option<String>,
    pub operator: Option<String>,
    pub value: Option<Value>,
}

/// `ConditionEvaluationContext` (the event metadata, the contact and the
/// workspace zone).
pub struct EvalContext<'a> {
    pub metadata: &'a Map<String, Value>,
    pub contact_id: &'a str,
    /// `workspace.timezone`.
    pub workspace_timezone: Option<&'a str>,
    /// `Date.now()`.
    pub now_ms: i64,
}

/// `ConditionEvaluator.evaluate`. A DB error fails the evaluation (the
/// original rejects).
pub async fn evaluate(
    db: &Db,
    condition: &Condition,
    ctx: &EvalContext<'_>,
) -> Result<bool, String> {
    let source_id = condition.source_id.as_deref().filter(|s| !s.is_empty());
    let meta = ctx.metadata;
    Ok(match condition.r#type.as_str() {
        "tagApplied" | "tagRemoved" => source_id_match(source_id, meta.get("tagId")),
        "contactInfoUpdated" => source_id_match(source_id, meta.get("infoType")),
        "customFieldValueChanged" => custom_field_condition(db, condition, source_id, ctx).await?,
        "conversationTransferredToHuman"
        | "conversationTransferredToBot"
        | "newContact"
        | "contactUnsubscribedFormBroadcast"
        | "archived"
        | "followUp"
        | "conversationAssigned"
        | "conversationUnassigned"
        | "subscribedToSequence"
        | "unsubscribedFromSequence"
        | "contactReferredANewContact"
        | "contactReferredExistingContact"
        | "contactBlockedBot"
        | "contactUnblockedBot"
        | "incomingCall"
        | "missedAudioCall"
        | "callEnded"
        | "callRecorded"
        | "callTranscribed"
        | "newOrder"
        | "orderAccepted"
        | "orderShipped"
        | "orderConcluded"
        | "orderCancelled"
        | "orderPaid"
        | "orderRefunded"
        | "cartAbandoned" => true,
        "pipelineItemCreated"
        | "pipelineItemMovedToStage"
        | "pipelineItemValueChanged"
        | "pipelineItemStatusChanged"
        | "pipelineItemPriorityChanged" => pipeline_item(source_id, condition.value.as_ref(), meta),
        "shoppingCartSent"
        | "userAskedAboutProduct"
        | "productOrdered"
        | "productAddedToCart"
        | "productRemovedFromCart"
        | "categoryAddedToCart" => match source_id {
            None => true,
            Some(pinned) => referenced_product_ids(&condition.r#type, meta)
                .iter()
                .any(|id| id == pinned),
        },
        "dateTimeBasedTrigger" => datetime_condition(db, condition, source_id, ctx).await?,
        _ => false,
    })
}

/// `referencedProductIdsByEventType`: an unresolved reference names no
/// product.
fn referenced_product_ids(event_type: &str, meta: &Map<String, Value>) -> Vec<String> {
    let resolved = |reference: &Value| -> Option<String> {
        let product = reference.get("product").filter(|p| truthy(Some(p)))?;
        Some(
            product
                .get("productId")
                .map(js::string_of)
                .unwrap_or_default(),
        )
    };
    let truthy_id = |value: Option<&Value>| -> Option<String> {
        value.filter(|v| truthy(Some(v))).map(js::string_of)
    };
    let list = |key: &str| {
        meta.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    match event_type {
        "shoppingCartSent" => list("products").iter().filter_map(resolved).collect(),
        "userAskedAboutProduct" => meta.get("product").and_then(resolved).into_iter().collect(),
        "productOrdered" => list("products")
            .iter()
            .filter_map(|p| truthy_id(p.get("productId")))
            .collect(),
        "productAddedToCart" | "productRemovedFromCart" => {
            truthy_id(meta.get("product").and_then(|p| p.get("productId")))
                .into_iter()
                .collect()
        }
        "categoryAddedToCart" => meta
            .get("product")
            .and_then(|p| p.get("categoryIds"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(js::string_of)
            .collect(),
        _ => Vec::new(),
    }
}

/// `evaluatePipelineItem`: `sourceId` pins one pipeline; a `{ stageId:
/// string }` value pins the destination stage.
fn pipeline_item(pinned: Option<&str>, value: Option<&Value>, meta: &Map<String, Value>) -> bool {
    if let Some(pinned) = pinned
        && meta.get("pipelineId").and_then(Value::as_str) != Some(pinned)
    {
        return false;
    }
    match value.and_then(|v| v.get("stageId")).and_then(Value::as_str) {
        Some(stage) => meta.get("stageId").and_then(Value::as_str) == Some(stage),
        None => true,
    }
}

/// `evaluateSourceIdMatch`.
fn source_id_match(expected: Option<&str>, actual: Option<&Value>) -> bool {
    expected.is_some_and(|e| actual.and_then(Value::as_str) == Some(e))
}

/// `contactCustomFieldService.findValue`.
pub async fn find_custom_field_value(
    db: &Db,
    contact_id: &str,
    custom_field_id: &str,
) -> Result<Option<String>, String> {
    let (Ok(contact), Ok(field)) = (contact_id.parse::<i64>(), custom_field_id.parse::<i64>())
    else {
        return Ok(None);
    };
    db.read(move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
                params![contact, field],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
    .map_err(|e| e.to_string())
}

async fn custom_field_type(db: &Db, id: &str) -> Result<Option<String>, String> {
    let Ok(id) = id.parse::<i64>() else {
        return Ok(None);
    };
    db.read(move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "type" FROM "CustomField" WHERE "id" = ?1"#,
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
    .map_err(|e| e.to_string())
}

/// `evaluateCustomFieldCondition`.
async fn custom_field_condition(
    db: &Db,
    condition: &Condition,
    field_id: Option<&str>,
    ctx: &EvalContext<'_>,
) -> Result<bool, String> {
    let Some(field_id) = field_id else {
        return Ok(false);
    };
    let actual: Option<Value> =
        if ctx.metadata.get("customFieldId").and_then(Value::as_str) == Some(field_id) {
            ctx.metadata.get("newValue").cloned()
        } else {
            Some(
                find_custom_field_value(db, ctx.contact_id, field_id)
                    .await?
                    .map_or(Value::Null, Value::String),
            )
        };
    let field_type = custom_field_type(db, field_id).await?;
    let Some(operator) = condition.operator.as_deref().filter(|o| !o.is_empty()) else {
        return Ok(true);
    };
    let expected = condition.value.clone().unwrap_or(Value::Null);
    let timezone = condition_timezone(&expected)
        .or(ctx.workspace_timezone.filter(|tz| !tz.is_empty()))
        .unwrap_or("UTC")
        .to_owned();
    Ok(evaluate_operator(
        operator,
        actual.as_ref(),
        &expected,
        field_type.as_deref(),
        &timezone,
    ))
}

/// `extractConditionTimezone`.
fn condition_timezone(value: &Value) -> Option<&str> {
    value
        .get("timezone")
        .and_then(Value::as_str)
        .filter(|tz| !tz.is_empty())
}

/// `normalizeOperator`.
fn normalize_operator(operator: &str) -> &str {
    match operator {
        "eq" => "is",
        "ne" => "isNot",
        "isEmpty" => "hasNoValue",
        "isNotEmpty" => "hasAnyValue",
        "notContains" => "doesNotContain",
        "isBetween" => "interval",
        "notBetween" => "notInterval",
        other => other,
    }
}

fn is_nullish_or_empty(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null)) || value.and_then(Value::as_str) == Some("")
}

/// `evaluateOperator`.
fn evaluate_operator(
    operator: &str,
    actual: Option<&Value>,
    expected_value: &Value,
    field_type: Option<&str>,
    timezone: &str,
) -> bool {
    let operator = normalize_operator(operator);
    match operator {
        "hasAnyValue" => return !is_nullish_or_empty(actual),
        "hasNoValue" => return is_nullish_or_empty(actual),
        _ => {}
    }
    let expected = extract_expected_value(expected_value);
    let is_date_field = matches!(field_type, Some("date" | "datetime"));
    if operator == "interval" || operator == "notInterval" {
        return interval_operator(operator, actual, expected, is_date_field, timezone);
    }
    if is_date_field {
        return date_operator(operator, actual, expected, timezone);
    }
    standard_operator(operator, actual, expected)
}

/// `extractExpectedValue`: `obj.text || obj.number || obj.date || obj`.
fn extract_expected_value(value: &Value) -> &Value {
    if value.is_object() || value.is_array() {
        for key in ["text", "number", "date"] {
            if truthy(value.get(key)) {
                return &value[key];
            }
        }
    }
    value
}

/// JS `Number(value)`.
fn js_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js::to_number(s),
        Some(_) => f64::NAN,
    }
}

fn js_string(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), js::string_of)
}

/// JS `===` (objects are never equal: different references).
fn strict_eq(a: Option<&Value>, b: &Value) -> bool {
    match (a, b) {
        (Some(Value::Number(x)), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Some(Value::Object(_) | Value::Array(_)), _) => false,
        (Some(a), b) => a == b,
        (None, _) => false,
    }
}

fn standard_operator(operator: &str, actual: Option<&Value>, expected: &Value) -> bool {
    let (a, e) = (js_number(actual), js_number(Some(expected)));
    let (sa, se) = (js_string(actual), js::string_of(expected));
    match operator {
        "is" => strict_eq(actual, expected),
        "isNot" => !strict_eq(actual, expected),
        "gt" => a > e,
        "lt" => a < e,
        "gte" => a >= e,
        "lte" => a <= e,
        "contains" => sa.contains(&se),
        "doesNotContain" => !sa.contains(&se),
        "startsWith" => sa.starts_with(&se),
        "endsWith" => sa.ends_with(&se),
        _ => false,
    }
}

fn date_operator(operator: &str, actual: Option<&Value>, expected: &Value, timezone: &str) -> bool {
    if !(truthy(actual) && truthy(Some(expected))) {
        return false;
    }
    let (Some(a), Some(e)) = (
        actual.and_then(|a| parse_date_time_value(a, Some(timezone))),
        parse_date_time_value(expected, Some(timezone)),
    ) else {
        return false;
    };
    match operator {
        "is" => a == e,
        "isNot" => a != e,
        "gt" => a > e,
        "lt" => a < e,
        "gte" => a >= e,
        "lte" => a <= e,
        _ => false,
    }
}

/// `new Date(value).getTime()` (`NaN` = Invalid Date).
fn js_date(value: &Value) -> f64 {
    match value {
        Value::String(s) => js::parse_date(s).map_or(f64::NAN, |ms| ms as f64),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        _ => f64::NAN,
    }
}

fn interval_operator(
    operator: &str,
    actual: Option<&Value>,
    expected: &Value,
    is_date_field: bool,
    timezone: &str,
) -> bool {
    if !(truthy(actual) && truthy(Some(expected))) {
        return false;
    }
    let Some(actual) = actual else { return false };
    let zone = is_date_field.then_some(timezone);
    let Some((start, end)) = parse_interval(expected, zone) else {
        return false;
    };
    let value = if is_date_field {
        match parse_date_time_value(actual, Some(timezone)) {
            Some(v) => v,
            None => return false,
        }
    } else {
        js_date(actual)
    };
    if operator == "interval" {
        value >= start && value <= end
    } else {
        value < start || value > end
    }
}

/// `parseInterval`.
fn parse_interval(value: &Value, timezone: Option<&str>) -> Option<(f64, f64)> {
    let pick = |keys: [&str; 3]| -> Option<&Value> {
        keys.into_iter()
            .map(|k| value.get(k))
            .find(|v| truthy(*v))
            .flatten()
    };
    let pair = if value.is_object() {
        pick(["start", "from", "startDate"]).zip(pick(["end", "to", "endDate"]))
    } else {
        None
    };
    let pair = pair.or_else(|| match value.as_array() {
        Some(items) if items.len() == 2 => Some((&items[0], &items[1])),
        _ => None,
    })?;
    if let Some(tz) = timezone
        && let (Some(s), Some(e)) = (
            parse_date_time_value(pair.0, Some(tz)),
            parse_date_time_value(pair.1, Some(tz)),
        )
    {
        return Some((s, e));
    }
    Some((js_date(pair.0), js_date(pair.1)))
}

/// `toZonedWallClock(...).getTime()` on a UTC process: the instant shifted
/// to its wall clock in `timezone` (UTC when unusable).
pub fn wall_ms(instant_ms: i64, timezone: Option<&str>) -> i64 {
    let f = to_zoned_wall_clock(instant_ms, timezone);
    js::date_utc(
        f.year,
        i64::from(f.month) - 1,
        i64::from(f.day),
        i64::from(f.hour),
        i64::from(f.minute),
        i64::from(f.second),
        i64::from(f.millisecond),
    )
}

/// `parseDateTimeValue(...).getTime()`: `None` = `null`, `Some(NaN)` = an
/// Invalid Date. A bare calendar day is local (UTC) midnight; a string is
/// shifted to its wall clock in `timezone`; a number is kept.
pub fn parse_date_time_value(value: &Value, timezone: Option<&str>) -> Option<f64> {
    if !truthy(Some(value)) {
        return None;
    }
    match value {
        Value::String(s) => {
            let s = js::trim(s);
            let instant = if is_real_calendar_date(s) {
                js::parse_date(&format!("{s} 00:00:00"))
            } else {
                js::parse_date(s)
            };
            Some(instant.map_or(f64::NAN, |ms| wall_ms(ms, timezone.or(Some("UTC"))) as f64))
        }
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// `evaluateDateTimeCondition`.
async fn datetime_condition(
    db: &Db,
    condition: &Condition,
    field_id: Option<&str>,
    ctx: &EvalContext<'_>,
) -> Result<bool, String> {
    let config = condition.value.as_ref().filter(|v| truthy(Some(v)));
    let (Some(field_id), Some(config)) = (field_id, config) else {
        return Ok(false);
    };
    let Some(stored) = find_custom_field_value(db, ctx.contact_id, field_id)
        .await?
        .filter(|v| !v.is_empty())
    else {
        return Ok(false);
    };
    let trigger_type = config
        .get("triggerType")
        .and_then(Value::as_str)
        .unwrap_or("");
    if trigger_type.is_empty() {
        return Ok(false);
    }
    let timezone = config
        .get("timezone")
        .and_then(Value::as_str)
        .filter(|tz| !tz.is_empty())
        .or(ctx.workspace_timezone);
    let Some(target) = parse_date_time_value(&Value::String(stored), timezone) else {
        return Ok(false);
    };
    if target.is_nan() {
        return Ok(false);
    }
    let now = wall_ms(ctx.now_ms, timezone) as f64;
    let offset = || {
        let value = js_number(config.get("timeValue"));
        let unit = config.get("timeType").and_then(Value::as_str).unwrap_or("");
        if value == 0.0 || value.is_nan() || unit.is_empty() {
            return None;
        }
        Some(
            value
                * match unit {
                    "minutes" => 60_000.0,
                    "hours" => 3_600_000.0,
                    "days" => 86_400_000.0,
                    "weeks" => 604_800_000.0,
                    _ => 0.0,
                },
        )
    };
    Ok(match trigger_type {
        "before" => offset().is_some_and(|ms| now <= target - ms),
        "after" => offset().is_some_and(|ms| now >= target + ms),
        "atTheDayOf" => {
            let t = js::utc_fields(target as i64);
            let n = js::utc_fields(now as i64);
            let at = match config.get("at") {
                Some(v) if truthy(Some(v)) => js::string_of(v),
                _ => t.hour.to_string(),
            };
            let same_day = n.year == t.year && n.month == t.month && n.day == t.day;
            same_day && js::parse_int(&at).is_some_and(|h| h == f64::from(n.hour))
        }
        _ => false,
    })
}

#[cfg(test)]
#[path = "condition_tests.rs"]
mod tests;
