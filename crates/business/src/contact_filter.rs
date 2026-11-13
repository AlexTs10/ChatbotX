//! `apps/builder/src/features/contact-filter/schema` (P3.6-D2 slice contact
//! filter schema): the strict `contactFilterCriteriaSchema` /
//! `flowConditionFilterCriteriaSchema` as validators over JSON. Each parser
//! answers the parsed value (zod output: unknown keys stripped, strings
//! trimmed where the schema trims, the leaderboard score coerced to a string)
//! or zod-shaped issues (`{code, path, message}`, paths relative to the
//! parsed value). The SQL side is `chatbotx_db::repo::queries::contact_filter`.

use chatbotx_core::utils::normalize_for_match::normalize_for_match;
use chatbotx_db::repo::partials::contact_activity::{
    CONTACT_ACTIVITY_MAX_FILTER_VALUES, ValueFormat, contact_activity_filter,
    parse_contact_activity_references,
};
use chatbotx_db::repo::queries::contact_filter::FilterCriteria;
use serde_json::{Map, Value, json};

/// `ContactFilterSurface`: `contacts` = every list / audience surface,
/// `flowCondition` = the flow Condition step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Contacts,
    FlowCondition,
}

/// The parsed value or the zod-shaped issues.
pub type Parsed = std::result::Result<Value, Vec<Value>>;

// ---- enums ------------------------------------------------------------------

/// `operatorTypes`.
const OPERATOR_TYPES: &[&str] = &[
    "in",
    "notIn",
    "isEmpty",
    "isNotEmpty",
    "eq",
    "ne",
    "startsWith",
    "endsWith",
    "contains",
    "notContains",
    "lt",
    "lte",
    "gt",
    "gte",
    "isBetween",
    "notBetween",
    "used",
];
/// `customFieldTypes`.
const CUSTOM_FIELD_TYPES: &[&str] = &[
    "shortText",
    "email",
    "phoneNumber",
    "number",
    "date",
    "datetime",
    "boolean",
    "longText",
];
/// `formFieldTypes`.
const FORM_FIELD_TYPES: &[&str] = &[
    "multiSelect",
    "select",
    "text",
    "boolean",
    "datetime",
    "number",
];

/// The `CONTACT_FILTER_FIELD_DEFINITIONS` fields of every surface, in order.
const CONTACT_FIELDS: &[&str] = &[
    "locale",
    "language",
    "fullName",
    "country",
    "continent",
    "gender",
    "subscribedToBroadcast",
    "contactCreatedAt",
    "contactCreatedDateMinutesAgo",
    "source",
    "interactedInLast24h",
    "conversationTransferredToHuman",
    "followUp",
    "archived",
    "blocked",
    "existingContact",
    "hasContactInfo",
    "currentChannel",
    "inbox",
    "timezone",
    "followsBusinessOnInstagram",
    "businessFollowsUserOnInstagram",
    "verifiedAccountOnInstagram",
    "followerCountOnInstagram",
    "lastSent",
    "lastDelivered",
    "lastSeen",
    "lastSeenMinutesAgo",
    "lastInteraction",
    "lastInteractionMinutesAgo",
    "unreplied",
    "unread",
    "tags",
    "subscribedToDripCampaign",
    "conversationAssigned",
    "entryPointsLinks",
    "completedWhatsAppFlows",
    "sentMessage",
    "keywordsReceived",
    "executedFlow",
    "executedStep",
    "consecutiveAiFailures",
    "questionnaireStarted",
    "questionnaireInProgress",
    "questionnaireFinished",
    "lastComment",
    "commentedOnPost",
    "phone",
    "broadcastSent",
    "broadcastDelivered",
    "broadcastSeen",
    "broadcastClicked",
    "broadcastFailed",
    "email",
    "emailWasVerified",
    "optedInForEmail",
    "emailSent",
    "emailDelivered",
    "emailOpened",
    "emailClicked",
    "fromCtwaAd",
    "fromGoogleAd",
    "clickedAd",
    "ctwaConversion",
    "totalSpent",
    "numberOfOrders",
    "bought",
    "boughtItems",
    "shoppingCartIsEmpty",
    "shoppingCartContainsItems",
    "shoppingCartSubtotal",
    "shoppingCartTotal",
    "lastUserInput",
    "lastUserInputType",
];
/// Fields limited to the `flowCondition` surface.
const FLOW_ONLY_FIELDS: &[&str] = &["isWithinWorkingHours"];

fn is_static_field_on(field: &str, surface: Surface) -> bool {
    CONTACT_FIELDS.contains(&field)
        || (surface == Surface::FlowCondition && FLOW_ONLY_FIELDS.contains(&field))
}

// ---- operator rules ---------------------------------------------------------

const BASE_OPERATORS: &[&str] = &["eq", "ne", "isEmpty"];
const SET_OPERATORS: &[&str] = &["in", "notIn", "eq", "ne", "isEmpty"];
const PRESENCE_SET_OPERATORS: &[&str] = &["in", "notIn", "isEmpty"];
const IN_OPERATORS: &[&str] = &["in", "notIn"];
const BOOLEAN_OPERATORS: &[&str] = &["eq", "isEmpty"];
const NON_NULLABLE_BOOLEAN_OPERATORS: &[&str] = &["eq"];
const TEXT_FREE_OPERATORS: &[&str] = &[
    "eq",
    "ne",
    "isNotEmpty",
    "isEmpty",
    "contains",
    "notContains",
    "startsWith",
    "endsWith",
];
const NUMBER_OPERATORS: &[&str] = &[
    "eq",
    "ne",
    "isEmpty",
    "gt",
    "lt",
    "gte",
    "lte",
    "isBetween",
    "notBetween",
    "contains",
    "notContains",
    "startsWith",
    "endsWith",
];
const DATE_OPERATORS: &[&str] = &[
    "eq",
    "ne",
    "isEmpty",
    "gt",
    "lt",
    "gte",
    "lte",
    "isBetween",
    "notBetween",
];
const DATE_PRESENCE_OPERATORS: &[&str] = &[
    "eq",
    "ne",
    "isEmpty",
    "gt",
    "lt",
    "gte",
    "lte",
    "isBetween",
    "notBetween",
    "isNotEmpty",
];
const LAST_USER_INPUT_TYPE_OPERATORS: &[&str] = &["eq", "ne", "isNotEmpty", "isEmpty"];

/// `MAX_SENT_MESSAGE_FILTER_LENGTH`.
const MAX_SENT_MESSAGE_FILTER_LENGTH: usize = 1000;
/// `MAX_PIPELINE_ITEM_FILTER_STAGE_IDS`.
const MAX_PIPELINE_ITEM_FILTER_STAGE_IDS: usize = 200;
/// `MAX_ADS_ANALYTICS_RANGE_DAYS`.
const MAX_ADS_ANALYTICS_RANGE_DAYS: f64 = 366.0;
const MS_PER_DAY: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// `enabledOperatorsForStaticField`: the operators of a static field
/// (`TEXT_FREE_OPERATORS` for a field without a rule).
pub fn enabled_operators_for_static_field(field: &str) -> &'static [&'static str] {
    match field {
        "locale"
        | "country"
        | "continent"
        | "gender"
        | "commentedOnPost"
        | "timezone"
        | "boughtItems"
        | "shoppingCartContainsItems" => BASE_OPERATORS,
        "language"
        | "source"
        | "currentChannel"
        | "inbox"
        | "tags"
        | "broadcastSent"
        | "broadcastDelivered"
        | "broadcastSeen"
        | "broadcastClicked"
        | "broadcastFailed"
        | "subscribedToDripCampaign"
        | "entryPointsLinks"
        | "conversationAssigned"
        | "executedFlow"
        | "executedStep"
        | "keywordsReceived"
        | "completedWhatsAppFlows"
        | "clickedAd"
        | "emailSent"
        | "emailDelivered"
        | "emailOpened"
        | "emailClicked" => SET_OPERATORS,
        "isWithinWorkingHours" => IN_OPERATORS,
        "contactCreatedDateMinutesAgo"
        | "lastSeenMinutesAgo"
        | "lastInteractionMinutesAgo"
        | "consecutiveAiFailures"
        | "followerCountOnInstagram"
        | "totalSpent"
        | "numberOfOrders"
        | "shoppingCartTotal"
        | "shoppingCartSubtotal" => NUMBER_OPERATORS,
        "lastDelivered" | "lastInteraction" => DATE_PRESENCE_OPERATORS,
        "lastUserInputType" => LAST_USER_INPUT_TYPE_OPERATORS,
        "subscribedToBroadcast"
        | "interactedInLast24h"
        | "conversationTransferredToHuman"
        | "followUp"
        | "archived"
        | "blocked"
        | "existingContact"
        | "unreplied"
        | "unread"
        | "fromCtwaAd"
        | "fromGoogleAd"
        | "followsBusinessOnInstagram"
        | "businessFollowsUserOnInstagram"
        | "verifiedAccountOnInstagram" => BOOLEAN_OPERATORS,
        "emailWasVerified" | "optedInForEmail" | "shoppingCartIsEmpty" => {
            NON_NULLABLE_BOOLEAN_OPERATORS
        }
        "hasContactInfo" | "ctwaConversion" => PRESENCE_SET_OPERATORS,
        "contactCreatedAt" | "bought" | "lastSent" | "lastSeen" => DATE_OPERATORS,
        _ => TEXT_FREE_OPERATORS,
    }
}

fn is_valueless(operator: &str) -> bool {
    matches!(operator, "isEmpty" | "isNotEmpty")
}

fn is_interval(operator: &str) -> bool {
    matches!(operator, "isBetween" | "notBetween")
}

/// `operatorsForCustomField` / `operatorsForBotField`.
fn typed_field_operators(value_type: &str, field_type: Option<&str>) -> &'static [&'static str] {
    const BASE: &[&str] = &["eq", "ne", "isNotEmpty", "isEmpty"];
    const TEXT: &[&str] = &[
        "eq",
        "ne",
        "isNotEmpty",
        "isEmpty",
        "contains",
        "notContains",
        "startsWith",
        "endsWith",
    ];
    const NUMBER: &[&str] = &[
        "eq",
        "ne",
        "isNotEmpty",
        "isEmpty",
        "gt",
        "lt",
        "gte",
        "lte",
        "isBetween",
        "notBetween",
        "contains",
        "notContains",
        "startsWith",
        "endsWith",
    ];
    const DATE: &[&str] = &[
        "eq",
        "ne",
        "isNotEmpty",
        "isEmpty",
        "gt",
        "lt",
        "gte",
        "lte",
        "isBetween",
        "notBetween",
    ];
    const BOOLEAN: &[&str] = &["eq", "isNotEmpty", "isEmpty"];
    if field_type == Some("longText") {
        return BASE;
    }
    match value_type {
        "number" => NUMBER,
        "datetime" => DATE,
        "boolean" => BOOLEAN,
        _ => TEXT,
    }
}

// ---- zod-shaped primitives --------------------------------------------------

/// Issue collector over one object (paths relative to the parsed value).
struct Ctx {
    issues: Vec<Value>,
}

fn seg_path(path: &[Value]) -> Value {
    Value::Array(path.to_vec())
}

fn at(path: &[Value], name: &str) -> Vec<Value> {
    let mut out = path.to_vec();
    out.push(json!(name));
    out
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

fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}')
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn options_message(options: &[&str]) -> String {
    let quoted: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
    format!("Invalid option: expected one of {}", quoted.join("|"))
}

impl Ctx {
    fn new() -> Ctx {
        Ctx { issues: Vec::new() }
    }

    fn issue(&mut self, path: &[Value], code: &str, message: &str) {
        self.issues.push(json!({
            "code": code,
            "path": seg_path(path),
            "message": message,
        }));
    }

    fn wrong_type(&mut self, path: &[Value], expected: &str, value: Option<&Value>) {
        self.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected {expected}, received {}",
                type_name(value)
            ),
        );
    }

    fn finish(self, out: Map<String, Value>) -> Parsed {
        if self.issues.is_empty() {
            Ok(Value::Object(out))
        } else {
            Err(self.issues)
        }
    }

    /// `z.literal(expected)`.
    fn literal(&mut self, path: &[Value], value: Option<&Value>, expected: &str) -> bool {
        if value.and_then(Value::as_str) == Some(expected) {
            return true;
        }
        self.issue(
            path,
            "invalid_value",
            &format!("Invalid input: expected \"{expected}\""),
        );
        false
    }

    /// `z.enum(options)` (required).
    fn enumeration(
        &mut self,
        path: &[Value],
        value: Option<&Value>,
        options: &[&str],
    ) -> Option<String> {
        match value.and_then(Value::as_str) {
            Some(text) if options.contains(&text) => Some(text.to_owned()),
            _ => {
                self.issue(path, "invalid_value", &options_message(options));
                None
            }
        }
    }

    /// `z.string()` with an optional `trim` and length bounds.
    fn string(
        &mut self,
        path: &[Value],
        value: Option<&Value>,
        trim: bool,
        min: Option<usize>,
        max: Option<usize>,
    ) -> Option<String> {
        let Some(Value::String(raw)) = value else {
            self.wrong_type(path, "string", value);
            return None;
        };
        let text = if trim { js_trim(raw) } else { raw.as_str() };
        let len = utf16_len(text);
        if let Some(min) = min
            && len < min
        {
            self.issue(
                path,
                "too_small",
                &format!("Too small: expected string to have >={min} characters"),
            );
            return None;
        }
        if let Some(max) = max
            && len > max
        {
            self.issue(
                path,
                "too_big",
                &format!("Too big: expected string to have <={max} characters"),
            );
            return None;
        }
        Some(text.to_owned())
    }

    /// `sampleStringSchema` (`z.string().trim().min(1).max(255)`).
    fn sample(&mut self, path: &[Value], value: Option<&Value>) -> Option<String> {
        self.string(path, value, true, Some(1), Some(255))
    }

    fn union_failed(&mut self, path: &[Value]) {
        self.issue(path, "invalid_union", "Invalid input");
    }
}

/// `sampleStringSchema` without recording issues.
fn sample_quiet(value: &Value) -> Option<String> {
    Ctx::new().sample(&[], Some(value))
}

/// `z.union([sample, z.array(sample), z.tuple([sample, sample])])` (static)
/// or `z.union([sample, z.tuple([sample, sample])])` (`allow_list` false).
fn value_union(ctx: &mut Ctx, path: &[Value], value: &Value, allow_list: bool) -> Option<Value> {
    match value {
        Value::String(_) => {
            if let Some(text) = sample_quiet(value) {
                return Some(Value::String(text));
            }
        }
        Value::Array(items) if allow_list || items.len() == 2 => {
            let parsed: Option<Vec<Value>> = items
                .iter()
                .map(|item| sample_quiet(item).map(Value::String))
                .collect();
            if let Some(parsed) = parsed {
                return Some(Value::Array(parsed));
            }
        }
        _ => {}
    }
    ctx.union_failed(path);
    None
}

fn object_of<'a>(
    ctx: &mut Ctx,
    path: &[Value],
    value: &'a Value,
) -> Option<&'a Map<String, Value>> {
    match value {
        Value::Object(obj) => Some(obj),
        other => {
            ctx.wrong_type(path, "object", Some(other));
            None
        }
    }
}

/// `hasValue`: a non-empty string, or a non-empty list of non-empty strings.
fn has_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.as_str().is_some_and(|s| !s.is_empty()))
        }
        Some(Value::String(text)) => !text.is_empty(),
        _ => false,
    }
}

/// `isFilterValueId`: 1..19 digits in canonical form, at most `i64::MAX`.
fn is_filter_value_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 19
        && text.bytes().all(|b| b.is_ascii_digit())
        && text
            .parse::<i64>()
            .is_ok_and(|n| n >= 0 && n.to_string() == text)
}

/// `isChannelPostId`: `^[1-9]\d*$`, at most 19 characters, at most `i64::MAX`.
fn is_channel_post_id(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let mut chars = text.chars();
    text.len() <= 19
        && matches!(chars.next(), Some('1'..='9'))
        && chars.all(|c| c.is_ascii_digit())
        && text.parse::<i64>().is_ok()
}

// ---- static fields ----------------------------------------------------------

/// `staticFieldFilter(field)`: one static-field condition.
pub fn parse_static_field_condition(field: &str, value: &Value) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), field) {
        out.insert("field".into(), json!(field));
    }
    let operator = ctx.enumeration(&at(&[], "operator"), obj.get("operator"), OPERATOR_TYPES);
    if let Some(operator) = &operator {
        out.insert("operator".into(), json!(operator));
    }
    if let Some(raw) = obj.get("value")
        && let Some(parsed) = value_union(&mut ctx, &at(&[], "value"), raw, true)
    {
        out.insert("value".into(), parsed);
    }
    if !ctx.issues.is_empty() {
        return Err(ctx.issues);
    }
    let operator = operator.unwrap_or_default();
    refine_static(&mut ctx, field, &operator, out.get("value"));
    ctx.finish(out)
}

fn refine_static(ctx: &mut Ctx, field: &str, operator: &str, value: Option<&Value>) {
    let value_path = at(&[], "value");
    if !enabled_operators_for_static_field(field).contains(&operator) {
        ctx.issue(
            &at(&[], "operator"),
            "custom",
            "Operator is not supported for this field",
        );
        return;
    }
    if is_valueless(operator) {
        return;
    }
    if field == "commentedOnPost" {
        let valid = match value {
            Some(Value::Array(items)) => {
                let mut seen = std::collections::HashSet::new();
                !items.is_empty()
                    && items.len() <= 100
                    && items.iter().all(|item| seen.insert(item.to_string()))
                    && items.iter().all(is_channel_post_id)
            }
            _ => false,
        };
        if !valid {
            ctx.issue(
                &value_path,
                "custom",
                "Commented posts require one to 100 unique bigint ids",
            );
        }
        return;
    }
    if let Some(config) = contact_activity_filter(field) {
        if parse_contact_activity_references(config, value).is_none() {
            let format = match config.value_format {
                ValueFormat::Id => "id",
                ValueFormat::FlowIdNodeId => "flowId:nodeId",
            };
            let fixed = if config.fixed_values.is_empty() {
                String::new()
            } else {
                format!(" or {}", config.fixed_values.join(", "))
            };
            ctx.issue(
                &value_path,
                "custom",
                &format!(
                    "Values must be {format} ids{fixed} (one to {CONTACT_ACTIVITY_MAX_FILTER_VALUES})"
                ),
            );
        }
        return;
    }
    if field == "sentMessage" {
        let valid = value.and_then(Value::as_str).is_some_and(|text| {
            !js_trim(&normalize_for_match(text)).is_empty()
                && utf16_len(text) <= MAX_SENT_MESSAGE_FILTER_LENGTH
        });
        if !valid {
            ctx.issue(
                &value_path,
                "custom",
                &format!(
                    "Sent message requires text of at most {MAX_SENT_MESSAGE_FILTER_LENGTH} characters"
                ),
            );
        }
        return;
    }
    if is_interval(operator) {
        if !matches!(value, Some(Value::Array(items)) if items.len() == 2) {
            ctx.issue(
                &value_path,
                "custom",
                "Interval operators require two values",
            );
        }
        return;
    }
    if !has_value(value) {
        ctx.issue(&value_path, "custom", "Operator requires a value");
    }
}

// ---- dynamic fields ---------------------------------------------------------

/// `customFieldConditionSchema` / `botFieldConditionSchema`.
fn parse_typed_field_condition(
    value: &Value,
    literal: &str,
    id_key: &str,
    type_key: &str,
    kind: &str,
) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), literal) {
        out.insert("field".into(), json!(literal));
    }
    if let Some(id) = ctx.string(&at(&[], id_key), obj.get(id_key), false, Some(1), None) {
        out.insert(id_key.into(), json!(id));
    }
    let mut field_type = None;
    if let Some(raw) = obj.get(type_key) {
        field_type = ctx.enumeration(&at(&[], type_key), Some(raw), CUSTOM_FIELD_TYPES);
        if let Some(t) = &field_type {
            out.insert(type_key.into(), json!(t));
        }
    }
    let value_type = ctx.enumeration(
        &at(&[], "valueType"),
        obj.get("valueType"),
        FORM_FIELD_TYPES,
    );
    if let Some(t) = &value_type {
        out.insert("valueType".into(), json!(t));
    }
    let operator = ctx.enumeration(&at(&[], "operator"), obj.get("operator"), OPERATOR_TYPES);
    if let Some(operator) = &operator {
        out.insert("operator".into(), json!(operator));
    }
    if let Some(raw) = obj.get("value")
        && let Some(parsed) = value_union(&mut ctx, &at(&[], "value"), raw, false)
    {
        out.insert("value".into(), parsed);
    }
    if !ctx.issues.is_empty() {
        return Err(ctx.issues);
    }
    let operator = operator.unwrap_or_default();
    let enabled = typed_field_operators(value_type.as_deref().unwrap_or(""), field_type.as_deref());
    let value_path = at(&[], "value");
    if !enabled.contains(&operator.as_str()) {
        ctx.issue(
            &at(&[], "operator"),
            "custom",
            &format!("Operator is not supported for this {kind}"),
        );
    } else if is_valueless(&operator) {
        // No value to check.
    } else if is_interval(&operator) {
        if !matches!(out.get("value"), Some(Value::Array(items)) if items.len() == 2) {
            ctx.issue(
                &value_path,
                "custom",
                "Interval operators require two values",
            );
        }
    } else if !out
        .get("value")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
    {
        ctx.issue(&value_path, "custom", "Operator requires a value");
    }
    ctx.finish(out)
}

/// `customFieldConditionSchema`.
pub fn parse_custom_field_condition(value: &Value) -> Parsed {
    parse_typed_field_condition(
        value,
        "customField",
        "customFieldId",
        "customFieldType",
        "custom field",
    )
}

/// `botFieldConditionSchema`.
pub fn parse_bot_field_condition(value: &Value) -> Parsed {
    parse_typed_field_condition(value, "botField", "botFieldId", "botFieldType", "bot field")
}

/// `couponTopicConditionSchema`.
pub fn parse_coupon_topic_condition(value: &Value) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), "couponTopic") {
        out.insert("field".into(), json!("couponTopic"));
    }
    if let Some(id) = ctx.string(
        &at(&[], "topicId"),
        obj.get("topicId"),
        false,
        Some(1),
        None,
    ) {
        out.insert("topicId".into(), json!(id));
    }
    let operator = ctx.enumeration(&at(&[], "operator"), obj.get("operator"), OPERATOR_TYPES);
    if let Some(operator) = &operator {
        out.insert("operator".into(), json!(operator));
    }
    if let Some(raw) = obj.get("value")
        && let Some(text) = ctx.sample(&at(&[], "value"), Some(raw))
    {
        out.insert("value".into(), json!(text));
    }
    if !ctx.issues.is_empty() {
        return Err(ctx.issues);
    }
    let operator = operator.unwrap_or_default();
    if !["isNotEmpty", "used", "eq"].contains(&operator.as_str()) {
        ctx.issue(
            &at(&[], "operator"),
            "custom",
            "Operator is not supported for this field",
        );
    } else if operator == "eq"
        && !out
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    {
        ctx.issue(&at(&[], "value"), "custom", "Operator requires a value");
    }
    ctx.finish(out)
}

/// JS `String(value)` of a JSON value (`z.coerce.string()`).
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
            Some(f) => f.to_string(),
            None => n.to_string(),
        },
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `Number.isFinite(Number(text))`.
fn js_number_is_finite(text: &str) -> bool {
    let trimmed = js_trim(text);
    if trimmed.is_empty() {
        return true;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix));
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return false;
    }
    trimmed.parse::<f64>().is_ok_and(f64::is_finite)
}

/// `leaderboardScoreConditionSchema`.
pub fn parse_leaderboard_score_condition(value: &Value) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), "leaderboardScore") {
        out.insert("field".into(), json!("leaderboardScore"));
    }
    if let Some(id) = ctx.string(
        &at(&[], "leaderboardId"),
        obj.get("leaderboardId"),
        false,
        Some(1),
        None,
    ) {
        out.insert("leaderboardId".into(), json!(id));
    }
    let operator = ctx.enumeration(&at(&[], "operator"), obj.get("operator"), OPERATOR_TYPES);
    if let Some(operator) = &operator {
        out.insert("operator".into(), json!(operator));
    }
    let score = js_string(obj.get("value"));
    out.insert("value".into(), json!(score));
    if !ctx.issues.is_empty() {
        return Err(ctx.issues);
    }
    let operator = operator.unwrap_or_default();
    if !["eq", "lt", "lte", "gt", "gte"].contains(&operator.as_str()) {
        ctx.issue(
            &at(&[], "operator"),
            "custom",
            "Operator is not supported for this field",
        );
    } else if score.is_empty() || !js_number_is_finite(&score) {
        ctx.issue(&at(&[], "value"), "custom", "Enter a score");
    }
    ctx.finish(out)
}

/// `pipelineItemConditionSchema` (discriminated on `attribute`).
pub fn parse_pipeline_item_condition(value: &Value) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let attribute = obj.get("attribute").and_then(Value::as_str).unwrap_or("");
    let (operators, pipeline_required): (&[&str], bool) = match attribute {
        "exists" => (&["isNotEmpty", "isEmpty"], false),
        "status" | "priority" => (&["in", "notIn"], false),
        "stage" => (&["in", "notIn"], true),
        _ => {
            ctx.issue(&at(&[], "attribute"), "invalid_union", "Invalid input");
            return Err(ctx.issues);
        }
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), "pipelineItem") {
        out.insert("field".into(), json!("pipelineItem"));
    }
    out.insert("attribute".into(), json!(attribute));
    let pipeline_path = at(&[], "pipelineId");
    match obj.get("pipelineId") {
        Some(Value::Null) if !pipeline_required => {
            out.insert("pipelineId".into(), Value::Null);
        }
        Some(Value::String(id)) => {
            if is_filter_value_id(id) {
                out.insert("pipelineId".into(), json!(id));
            } else {
                ctx.issue(&pipeline_path, "custom", "Invalid pipeline id");
            }
        }
        other => ctx.wrong_type(&pipeline_path, "string", other),
    }
    if let Some(operator) = ctx.enumeration(&at(&[], "operator"), obj.get("operator"), operators) {
        out.insert("operator".into(), json!(operator));
    }
    let value_path = at(&[], "value");
    let raw = obj.get("value");
    match attribute {
        "exists" => {
            if let Some(v) = ctx.enumeration(&value_path, raw, &["any", "open", "won", "lost"]) {
                out.insert("value".into(), json!(v));
            }
        }
        _ => {
            let (allowed, max): (Option<&[&str]>, usize) = match attribute {
                "status" => (Some(&["open", "won", "lost"]), 3),
                "priority" => (Some(&["low", "medium", "high"]), 3),
                _ => (None, MAX_PIPELINE_ITEM_FILTER_STAGE_IDS),
            };
            match raw {
                Some(Value::Array(items)) => {
                    let mut parsed = Vec::new();
                    for (index, item) in items.iter().enumerate() {
                        let mut path = value_path.clone();
                        path.push(json!(index));
                        match allowed {
                            Some(options) => {
                                if let Some(v) = ctx.enumeration(&path, Some(item), options) {
                                    parsed.push(json!(v));
                                }
                            }
                            None => match item.as_str() {
                                Some(id) if is_filter_value_id(id) => parsed.push(json!(id)),
                                Some(_) => ctx.issue(&path, "custom", "Invalid stage id"),
                                None => ctx.wrong_type(&path, "string", Some(item)),
                            },
                        }
                    }
                    if items.is_empty() {
                        ctx.issue(
                            &value_path,
                            "too_small",
                            "Too small: expected array to have >=1 items",
                        );
                    } else if items.len() > max {
                        ctx.issue(
                            &value_path,
                            "too_big",
                            &format!("Too big: expected array to have <={max} items"),
                        );
                    }
                    out.insert("value".into(), Value::Array(parsed));
                }
                other => ctx.wrong_type(&value_path, "array", other),
            }
        }
    }
    ctx.finish(out)
}

/// `YYYY-MM-DD` as unix ms of its start (V8 `Date` reading: month 1..12,
/// day 1..31; a day beyond the month rolls over).
fn date_key_ms(text: &str) -> Option<f64> {
    let year: i32 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
    let date = first.checked_add_days(chrono::Days::new(u64::from(day - 1)))?;
    let ms = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis();
    Some(ms as f64)
}

fn is_date_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

/// `ctwaRetargetConditionSchema`.
pub fn parse_ctwa_retarget_condition(value: &Value) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if ctx.literal(&at(&[], "field"), obj.get("field"), "ctwaRetarget") {
        out.insert("field".into(), json!("ctwaRetarget"));
    }
    if let Some(segment) = ctx.enumeration(
        &at(&[], "segment"),
        obj.get("segment"),
        &["conversations", "leads", "purchases"],
    ) {
        out.insert("segment".into(), json!(segment));
    }
    if let Some(raw) = obj.get("adId")
        && let Some(ad) = ctx.string(&at(&[], "adId"), Some(raw), true, Some(1), None)
    {
        out.insert("adId".into(), json!(ad));
    }
    if let Some(raw) = obj.get("integrationWhatsappId")
        && let Some(id) = ctx.string(
            &at(&[], "integrationWhatsappId"),
            Some(raw),
            false,
            None,
            None,
        )
    {
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            out.insert("integrationWhatsappId".into(), json!(id));
        } else {
            ctx.issue(
                &at(&[], "integrationWhatsappId"),
                "invalid_format",
                "Invalid string: must match pattern /^\\d+$/",
            );
        }
    }
    if let Some(raw) = obj.get("channel")
        && let Some(channel) = ctx.enumeration(
            &at(&[], "channel"),
            Some(raw),
            &["whatsapp", "messenger", "instagram"],
        )
    {
        out.insert("channel".into(), json!(channel));
    }
    for key in ["since", "until"] {
        if let Some(text) = ctx.string(&at(&[], key), obj.get(key), false, None, None) {
            if is_date_key(&text) {
                out.insert(key.into(), json!(text));
            } else {
                ctx.issue(&at(&[], key), "invalid_format", "Expected YYYY-MM-DD");
            }
        }
    }
    if !ctx.issues.is_empty() {
        return Err(ctx.issues);
    }
    let since = out
        .get("since")
        .and_then(Value::as_str)
        .and_then(date_key_ms);
    let until = out
        .get("until")
        .and_then(Value::as_str)
        .and_then(date_key_ms)
        .map(|ms| ms + MS_PER_DAY - 1.0);
    match (since, until) {
        (Some(since), Some(until)) => {
            if since > until {
                ctx.issue(&at(&[], "until"), "custom", "since must not be after until");
            } else if (until - since) / MS_PER_DAY > MAX_ADS_ANALYTICS_RANGE_DAYS {
                ctx.issue(
                    &at(&[], "until"),
                    "custom",
                    "Date range exceeds the maximum allowed",
                );
            }
        }
        _ => ctx.issue(&at(&[], "since"), "custom", "Invalid date"),
    }
    ctx.finish(out)
}

// ---- unions and criteria ----------------------------------------------------

/// The dynamic condition schemas, in the union order.
/// One condition parser.
type ConditionParser = fn(&Value) -> Parsed;

const DYNAMIC: &[(&str, ConditionParser)] = &[
    ("customField", parse_custom_field_condition),
    ("botField", parse_bot_field_condition),
    ("couponTopic", parse_coupon_topic_condition),
    ("leaderboardScore", parse_leaderboard_score_condition),
    ("pipelineItem", parse_pipeline_item_condition),
    ("ctwaRetarget", parse_ctwa_retarget_condition),
];

/// `singleContactFilterConditionSchema` (`Contacts`) /
/// `singleFlowConditionFilterConditionSchema` (`FlowCondition`).
pub fn parse_condition(value: &Value, surface: Surface) -> Parsed {
    let field = value.get("field").and_then(Value::as_str);
    if let Some(field) = field
        && is_static_field_on(field, surface)
    {
        return parse_static_field_condition(field, value);
    }
    let mut matching = None;
    for (literal, parse) in DYNAMIC {
        match parse(value) {
            Ok(parsed) => return Ok(parsed),
            Err(issues) if field == Some(*literal) => matching = Some(issues),
            Err(_) => {}
        }
    }
    match matching {
        Some(issues) => Err(issues),
        None => {
            let mut ctx = Ctx::new();
            ctx.union_failed(&[]);
            Err(ctx.issues)
        }
    }
}

fn prefixed(issues: Vec<Value>, prefix: &[Value]) -> Vec<Value> {
    issues
        .into_iter()
        .map(|mut issue| {
            let rest = issue
                .get("path")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut path = prefix.to_vec();
            path.extend(rest);
            issue["path"] = Value::Array(path);
            issue
        })
        .collect()
}

/// `contactFilterCriteriaSchema` (`Contacts`) /
/// `flowConditionFilterCriteriaSchema` (`FlowCondition`).
pub fn parse_criteria(value: &Value, surface: Surface) -> Parsed {
    let mut ctx = Ctx::new();
    let Some(obj) = object_of(&mut ctx, &[], value) else {
        return Err(ctx.issues);
    };
    let mut out = Map::new();
    if let Some(operator) =
        ctx.enumeration(&at(&[], "operator"), obj.get("operator"), &["and", "or"])
    {
        out.insert("operator".into(), json!(operator));
    }
    match obj.get("conditions") {
        Some(Value::Array(items)) => {
            let mut parsed = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                match parse_condition(item, surface) {
                    Ok(condition) => parsed.push(condition),
                    Err(issues) => ctx
                        .issues
                        .extend(prefixed(issues, &[json!("conditions"), json!(index)])),
                }
            }
            out.insert("conditions".into(), Value::Array(parsed));
        }
        other => ctx.wrong_type(&at(&[], "conditions"), "array", other),
    }
    if let Some(raw) = obj.get("timezone")
        && let Some(zone) = ctx.string(&at(&[], "timezone"), Some(raw), false, None, Some(64))
    {
        out.insert("timezone".into(), json!(zone));
    }
    ctx.finish(out)
}

fn contact_filter_issues(issues: Vec<Value>) -> Vec<Value> {
    prefixed(issues, &[json!("contactFilter")])
}

/// The `contactFilter` key of `listContactsRequest` (builder UI): a string
/// is parsed as JSON and an invalid filter is dropped (`Ok(None)`); any other
/// value goes through [`parse_criteria`]. Issue paths start at `contactFilter`.
pub fn parse_lenient_param(
    value: Option<&Value>,
) -> std::result::Result<Option<Value>, Vec<Value>> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|json| parse_criteria(&json, Surface::Contacts).ok())),
        Some(other) => parse_criteria(other, Surface::Contacts)
            .map(Some)
            .map_err(contact_filter_issues),
    }
}

/// `strictContactFilter` (public API): a string is parsed as JSON (an
/// unparsable string is kept and rejected); an invalid filter is a 422.
/// Issue paths start at `contactFilter`.
pub fn parse_strict_param(value: Option<&Value>) -> std::result::Result<Option<Value>, Vec<Value>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let decoded = match value {
        Value::String(text) => {
            serde_json::from_str::<Value>(text).unwrap_or_else(|_| value.clone())
        }
        other => other.clone(),
    };
    parse_criteria(&decoded, Surface::Contacts)
        .map(Some)
        .map_err(contact_filter_issues)
}

/// A parsed criteria value as the repository input.
pub fn to_filter_criteria(parsed: &Value) -> FilterCriteria {
    serde_json::from_value(parsed.clone()).unwrap_or_default()
}
