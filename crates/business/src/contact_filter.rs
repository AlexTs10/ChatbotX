//! `apps/builder/src/features/contact-filter/schema` (P3.6-D2 slice contact
//! filter schema): the strict `contactFilterCriteriaSchema` /
//! `flowConditionFilterCriteriaSchema` as validators over JSON. Each parser
//! answers the parsed value (zod output: unknown keys stripped, strings
//! trimmed where the schema trims, the leaderboard score coerced to a string)
//! or zod-shaped issues (`{code, path, message}`, paths relative to the
//! parsed value). The SQL side is `chatbotx_db::repo::queries::contact_filter`.

use chatbotx_db::repo::queries::contact_filter::FilterCriteria;
use serde_json::Value;

/// `ContactFilterSurface`: `contacts` = every list / audience surface,
/// `flowCondition` = the flow Condition step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Contacts,
    FlowCondition,
}

/// The parsed value or the zod-shaped issues.
pub type Parsed = std::result::Result<Value, Vec<Value>>;

/// `enabledOperatorsForStaticField`: the operators of a static field
/// (`TEXT_FREE_OPERATORS` for a field without a rule).
pub fn enabled_operators_for_static_field(field: &str) -> &'static [&'static str] {
    let _ = field;
    todo!("P3.6-D2")
}

/// `staticFieldFilter(field)`: one static-field condition.
pub fn parse_static_field_condition(field: &str, value: &Value) -> Parsed {
    let _ = (field, value);
    todo!("P3.6-D2")
}

/// `customFieldConditionSchema`.
pub fn parse_custom_field_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `botFieldConditionSchema`.
pub fn parse_bot_field_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `couponTopicConditionSchema`.
pub fn parse_coupon_topic_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `leaderboardScoreConditionSchema`.
pub fn parse_leaderboard_score_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `pipelineItemConditionSchema`.
pub fn parse_pipeline_item_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `ctwaRetargetConditionSchema`.
pub fn parse_ctwa_retarget_condition(value: &Value) -> Parsed {
    let _ = value;
    todo!("P3.6-D2")
}

/// `singleContactFilterConditionSchema` (`Contacts`) /
/// `singleFlowConditionFilterConditionSchema` (`FlowCondition`).
pub fn parse_condition(value: &Value, surface: Surface) -> Parsed {
    let _ = (value, surface);
    todo!("P3.6-D2")
}

/// `contactFilterCriteriaSchema` (`Contacts`) /
/// `flowConditionFilterCriteriaSchema` (`FlowCondition`).
pub fn parse_criteria(value: &Value, surface: Surface) -> Parsed {
    let _ = (value, surface);
    todo!("P3.6-D2")
}

/// The `contactFilter` key of `listContactsRequest` (builder UI): a string
/// is parsed as JSON and an invalid filter is dropped (`Ok(None)`); any other
/// value goes through [`parse_criteria`]. Issue paths start at `contactFilter`.
pub fn parse_lenient_param(
    value: Option<&Value>,
) -> std::result::Result<Option<Value>, Vec<Value>> {
    let _ = value;
    todo!("P3.6-D2")
}

/// `strictContactFilter` (public API): a string is parsed as JSON (an
/// unparsable string is kept and rejected); an invalid filter is a 422.
/// Issue paths start at `contactFilter`.
pub fn parse_strict_param(value: Option<&Value>) -> std::result::Result<Option<Value>, Vec<Value>> {
    let _ = value;
    todo!("P3.6-D2")
}

/// A parsed criteria value as the repository input.
pub fn to_filter_criteria(parsed: &Value) -> FilterCriteria {
    let _ = parsed;
    todo!("P3.6-D2")
}
