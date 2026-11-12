//! Port of `packages/flow-config/src/steps/send-google-ads-conversion.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn google_ads_dedup_mode_schema = z::enum_(&["click", "id", "event"]));

schema!(pub fn google_ads_conversion_fields_schema = z::object(vec![("conversionActionId", z::string().regex(r#"^\d+$"#, "").msg("googleAds.conversionFields.validation.conversionActionRequired").describe("Numeric id of the Google Ads conversion action to record, picked from the workspace's synced conversion actions.")),
("value", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_value_transform).optional()).describe("Monetary value of the conversion as a decimal string, e.g. `49.90`, or a `{{variable}}`. Must be set together with `currency`.")),
("currency", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_currency_transform).optional()).describe("ISO 4217 currency code, e.g. `USD`, or a `{{variable}}`. Must be set together with `value`.")),
("dedupMode", google_ads_dedup_mode_schema().describe("How duplicate conversions are avoided: \"click\" counts this conversion action once per ad click (recommended for leads, sign-ups, bookings); \"id\" counts it once per `dedupId` (recommended for purchases and anything that can repeat); \"event\" records a new conversion every time the step runs, still safe against retries of the same run. Required.")),
("dedupId", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_dedup_id_transform).optional()).describe("Order or event ID, required when `dedupMode` is \"id\" and then up to 64 characters (or a `{{variable}}`); ignored when `dedupMode` is \"click\" or \"event\". Must stay the same across retries and be different for each purchase or event; never use the current time or a random value.")),
("matchEmail", z::union(vec![z::literal(json!("")),
z::string().trim().refine(is_google_ads_match_template, Some("googleAds.conversionFields.validation.matchTemplateInvalid"), None)]).optional().describe("Customer matching: a single `{{variable}}` that resolves to the contact's e-mail, e.g. `{{email}}` or a custom field. Hashed server-side when the conversion is sent, and only when ad user data consent is granted. Blank sends nothing; literal addresses are rejected.")),
("matchPhone", z::union(vec![z::literal(json!("")),
z::string().trim().refine(is_google_ads_match_template, Some("googleAds.conversionFields.validation.matchTemplateInvalid"), None)]).optional().describe("Customer matching: a single `{{variable}}` that resolves to the contact's phone number, e.g. `{{phone}}` or a custom field. It must be an international number; anything else is not sent. Same rules as `matchEmail`.")),
("customerType", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_customer_type_transform).optional()).describe("Optional: whether the customer is \"NEW\" or \"RETURNING\" when the conversion happened, or a `{{variable}}`. Sent to Google as a user property, only when ad user data consent is granted. Blank sends nothing.")),
("customerValueBucket", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_customer_value_bucket_transform).optional()).describe("Optional: how valuable the customer is, \"LOW\", \"MEDIUM\" or \"HIGH\", or a `{{variable}}`. Same rules as `customerType`.")),
("conversionTime", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_conversion_time_transform).optional()).describe("Optional time the conversion happened: a date-time with a timezone such as `2026-10-08T14:30:00+07:00`, or a `{{variable}}`. Leave unset to use the time the step runs. Cannot be in the future."))]));

schema!(pub fn send_google_ads_conversion_schema = z::object(vec![("conversionActionId", z::string().regex(r#"^\d+$"#, "").msg("googleAds.conversionFields.validation.conversionActionRequired").describe("Numeric id of the Google Ads conversion action to record, picked from the workspace's synced conversion actions.")),
("value", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_value_transform).optional()).describe("Monetary value of the conversion as a decimal string, e.g. `49.90`, or a `{{variable}}`. Must be set together with `currency`.")),
("currency", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_currency_transform).optional()).describe("ISO 4217 currency code, e.g. `USD`, or a `{{variable}}`. Must be set together with `value`.")),
("dedupMode", google_ads_dedup_mode_schema().describe("How duplicate conversions are avoided: \"click\" counts this conversion action once per ad click (recommended for leads, sign-ups, bookings); \"id\" counts it once per `dedupId` (recommended for purchases and anything that can repeat); \"event\" records a new conversion every time the step runs, still safe against retries of the same run. Required.")),
("dedupId", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_dedup_id_transform).optional()).describe("Order or event ID, required when `dedupMode` is \"id\" and then up to 64 characters (or a `{{variable}}`); ignored when `dedupMode` is \"click\" or \"event\". Must stay the same across retries and be different for each purchase or event; never use the current time or a random value.")),
("matchEmail", z::union(vec![z::literal(json!("")),
z::string().trim().refine(is_google_ads_match_template, Some("googleAds.conversionFields.validation.matchTemplateInvalid"), None)]).optional().describe("Customer matching: a single `{{variable}}` that resolves to the contact's e-mail, e.g. `{{email}}` or a custom field. Hashed server-side when the conversion is sent, and only when ad user data consent is granted. Blank sends nothing; literal addresses are rejected.")),
("matchPhone", z::union(vec![z::literal(json!("")),
z::string().trim().refine(is_google_ads_match_template, Some("googleAds.conversionFields.validation.matchTemplateInvalid"), None)]).optional().describe("Customer matching: a single `{{variable}}` that resolves to the contact's phone number, e.g. `{{phone}}` or a custom field. It must be an international number; anything else is not sent. Same rules as `matchEmail`.")),
("customerType", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_customer_type_transform).optional()).describe("Optional: whether the customer is \"NEW\" or \"RETURNING\" when the conversion happened, or a `{{variable}}`. Sent to Google as a user property, only when ad user data consent is granted. Blank sends nothing.")),
("customerValueBucket", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_customer_value_bucket_transform).optional()).describe("Optional: how valuable the customer is, \"LOW\", \"MEDIUM\" or \"HIGH\", or a `{{variable}}`. Same rules as `customerType`.")),
("conversionTime", z::preprocess(blank_to_undefined, z::string().transform(google_ads_conversion_fields_schema_conversion_time_transform).optional()).describe("Optional time the conversion happened: a date-time with a timezone such as `2026-10-08T14:30:00+07:00`, or a `{{variable}}`. Leave unset to use the time the step runs. Cannot be in the future.")),
("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("sendGoogleAdsConversion")).describe("Step type discriminator: \"sendGoogleAdsConversion\".")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(send_google_ads_conversion_schema_super_refine));

default_fn!(pub fn send_google_ads_conversion_default_fn = json!({"id": create_id(), "stepType": "sendGoogleAdsConversion", "conversionActionId": "", "dedupMode": "id", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

fn validation_key(key: &str) -> String {
    format!("googleAds.conversionFields.validation.{key}")
}

const DEDUP_ID_MAX_LENGTH: usize = 64;

fn blank_to_undefined(value: Option<Value>) -> Option<Value> {
    match value {
        Some(Value::String(s)) if crate::js::trim(&s).is_empty() => None,
        other => other,
    }
}

fn value_static_schema() -> Schema {
    z::string()
        .trim()
        .regex(r"^\d+(\.\d+)?$", "")
        .msg(&validation_key("valueInvalid"))
        .refine_owned(
            |v| js_number(Some(v)) <= 9_007_199_254_740_991.0,
            Some(validation_key("valueTooLarge")),
            None,
        )
}

fn currency_static_schema() -> Schema {
    z::string().trim().to_upper_case().pipe(
        z::string()
            .regex("^[A-Z]{3}$", "")
            .msg(&validation_key("currencyInvalid")),
    )
}

fn enum_static_schema(values: Vec<String>, key: &str) -> Schema {
    z::string()
        .trim()
        .to_upper_case()
        .pipe(z::enum_owned(values).error(&validation_key(key)))
}

fn customer_type_static_schema() -> Schema {
    use crate::utils::google_click::GoogleAdsCustomerType;
    enum_static_schema(
        GoogleAdsCustomerType::ALL
            .iter()
            .map(|v| v.as_str().to_owned())
            .collect(),
        "customerTypeInvalid",
    )
}

fn customer_value_bucket_static_schema() -> Schema {
    use crate::utils::google_click::GoogleAdsCustomerValueBucket;
    enum_static_schema(
        GoogleAdsCustomerValueBucket::ALL
            .iter()
            .map(|v| v.as_str().to_owned())
            .collect(),
        "customerValueBucketInvalid",
    )
}

fn dedup_id_static_schema() -> Schema {
    z::string().trim().min(1)
}

fn conversion_time_static_schema() -> Schema {
    use crate::utils::google_click::{has_rfc3339_shape, is_rfc3339_with_zone};
    z::string().trim().min(1).super_refine(|value, ctx| {
        let text = value.as_str().unwrap_or("");
        if !has_rfc3339_shape(text) {
            ctx.message(validation_key("timeFormat"));
        } else if !is_rfc3339_with_zone(text) {
            ctx.message(validation_key("timeInvalidDate"));
        }
    })
}

fn google_ads_conversion_fields_schema_value_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    template_or_static_value(value, ctx, &value_static_schema())
}

fn google_ads_conversion_fields_schema_currency_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    template_or_static_value(value, ctx, &currency_static_schema())
}

fn google_ads_conversion_fields_schema_dedup_id_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    template_or_static_value(value, ctx, &dedup_id_static_schema())
}

fn google_ads_conversion_fields_schema_customer_type_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    template_or_static_value(value, ctx, &customer_type_static_schema())
}

fn google_ads_conversion_fields_schema_customer_value_bucket_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    template_or_static_value(value, ctx, &customer_value_bucket_static_schema())
}

fn google_ads_conversion_fields_schema_conversion_time_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    template_or_static_value(value, ctx, &conversion_time_static_schema())
}

fn is_google_ads_match_template(value: &Value) -> bool {
    crate::utils::google_click::is_google_ads_match_template(value.as_str().unwrap_or(""))
}

/// `requireValueAndCurrencyTogether`.
pub fn require_value_and_currency_together(data: &Value, ctx: &mut RefineCtx) {
    let (value, currency) = (truthy(data.get("value")), truthy(data.get("currency")));
    if value && !currency {
        ctx.issue(path!["currency"], validation_key("currencyRequired"));
    }
    if currency && !value {
        ctx.issue(path!["value"], validation_key("valueRequired"));
    }
}

/// `requireDedupIdForIdMode`.
pub fn require_dedup_id_for_id_mode(data: &Value, ctx: &mut RefineCtx) {
    if data["dedupMode"] != "id" {
        return;
    }
    if !truthy(data.get("dedupId")) {
        ctx.issue(path!["dedupId"], validation_key("dedupIdRequired"));
        return;
    }
    let dedup_id = str_field(data, "dedupId");
    if crate::js::utf16_len(dedup_id) > DEDUP_ID_MAX_LENGTH
        && !crate::utils::variables::contains_variable_placeholder(dedup_id)
    {
        ctx.issue(path!["dedupId"], validation_key("dedupIdTooLong"));
    }
}

fn send_google_ads_conversion_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    require_value_and_currency_together(data, ctx);
    require_dedup_id_for_id_mode(data, ctx);
}

/// `withGoogleAdsConversionRefinements(schema)`.
pub fn with_google_ads_conversion_refinements(schema: Schema) -> Schema {
    schema.super_refine(send_google_ads_conversion_schema_super_refine)
}
