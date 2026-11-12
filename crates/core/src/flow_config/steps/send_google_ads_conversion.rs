//! Port of `packages/flow-config/src/steps/send-google-ads-conversion.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn google_ads_dedup_mode_schema = z::enum_(&["click", "id", "event"]));

// HAND-PORT requireValueAndCurrencyTogether:
// (data,ctx)=>{if(data.value&&!data.currency){ctx.addIssue({code:"custom",path:["currency"],message:validationKey("currencyRequired")})}if(data.currency&&!data.value){ctx.addIssue({code:"custom",path:["value"],message:validationKey("valueRequired")})}}

// HAND-PORT requireDedupIdForIdMode:
// (data,ctx)=>{if(data.dedupMode!=="id"){return}if(!data.dedupId){ctx.addIssue({code:"custom",path:["dedupId"],message:validationKey("dedupIdRequired")});return}if(data.dedupId.length>DEDUP_ID_MAX_LENGTH&&!containsVariablePlaceholder(data.dedupId)){ctx.addIssue({code:"custom",path:["dedupId"],message:validationKey("dedupIdTooLong")})}}

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

// HAND-PORT withGoogleAdsConversionRefinements:
// schema=>schema.superRefine((data,ctx)=>{requireValueAndCurrencyTogether(data,ctx);requireDedupIdForIdMode(data,ctx)})

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

// value=>typeof value==="string"&&value.trim()===""?void 0:value
fn blank_to_undefined(value: Option<Value>) -> Option<Value> {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_value_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_currency_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_dedup_id_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// value=>MATCH_TEMPLATE_PATTERN.test(value)
fn is_google_ads_match_template(value: &Value) -> bool {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_customer_type_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_customer_value_bucket_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn google_ads_conversion_fields_schema_conversion_time_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (data,ctx)=>{requireValueAndCurrencyTogether(data,ctx);requireDedupIdForIdMode(data,ctx)}
fn send_google_ads_conversion_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
