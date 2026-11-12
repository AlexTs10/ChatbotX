//! Port of `packages/flow-config/src/steps/send-meta-capi-event.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn meta_capi_event_fields_schema = z::object(vec![("eventName", deps::meta_capi_event_name_schema().default(json!("LeadSubmitted")).describe("Meta event name. `business_messaging` allows only its documented events (e.g. LeadSubmitted, Purchase); other action sources allow Meta Pixel standard events or a custom name up to 50 characters.")),
("actionSource", deps::meta_capi_action_source_schema().default(json!("business_messaging")).describe("Where the conversion happened (e.g. `business_messaging`, `website`, `email`); it decides which event names are valid.")),
("contentType", deps::meta_capi_content_type_schema().optional().describe("Kind of the content ids: `product` or `product_group`.")),
("contentIds", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_content_ids_transform).optional()).describe("Product ids (retailer ids from the catalog) the event relates to.")),
("value", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_value_transform).optional()).describe("Monetary value of the conversion as a decimal string, e.g. `49.90`. Required for Purchase.")),
("currency", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_currency_transform).optional()).describe("ISO 4217 currency code, e.g. `USD`. Required for Purchase.")),
("contentCategory", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Category of the content, free text.")),
("contentName", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Name of the content, free text."))]));

schema!(pub fn send_meta_capi_event_schema = z::object(vec![("eventName", deps::meta_capi_event_name_schema().default(json!("LeadSubmitted")).describe("Meta event name. `business_messaging` allows only its documented events (e.g. LeadSubmitted, Purchase); other action sources allow Meta Pixel standard events or a custom name up to 50 characters.")),
("actionSource", deps::meta_capi_action_source_schema().default(json!("business_messaging")).describe("Where the conversion happened (e.g. `business_messaging`, `website`, `email`); it decides which event names are valid.")),
("contentType", deps::meta_capi_content_type_schema().optional().describe("Kind of the content ids: `product` or `product_group`.")),
("contentIds", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_content_ids_transform).optional()).describe("Product ids (retailer ids from the catalog) the event relates to.")),
("value", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_value_transform).optional()).describe("Monetary value of the conversion as a decimal string, e.g. `49.90`. Required for Purchase.")),
("currency", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_currency_transform).optional()).describe("ISO 4217 currency code, e.g. `USD`. Required for Purchase.")),
("contentCategory", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Category of the content, free text.")),
("contentName", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Name of the content, free text.")),
("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("sendMetaCapiEvent")).describe("Step type discriminator: \"sendMetaCapiEvent\".")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(require_value_and_currency_for_event).super_refine(require_event_name_allowed_for_action_source));

default_fn!(pub fn send_meta_capi_event_default_fn = json!({"id": create_id(), "stepType": "sendMetaCapiEvent", "eventName": "LeadSubmitted", "actionSource": "business_messaging", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

fn blank_to_undefined(value: Option<Value>) -> Option<Value> {
    match value {
        Some(Value::String(s)) if crate::js::trim(&s).is_empty() => None,
        other => other,
    }
}

/// The body of `templateOrStatic(staticSchema)`'s transform: a `{{variable}}`
/// passes through, else the static schema's output (or its issues).
pub fn template_or_static_value(
    value: Value,
    ctx: &mut RefineCtx,
    static_schema: &Schema,
) -> Value {
    if crate::utils::variables::contains_variable_placeholder(value.as_str().unwrap_or("")) {
        return value;
    }
    let result = static_schema.safe_parse(&value);
    if let Some(data) = result.data {
        return data;
    }
    for issue in result.issues() {
        ctx.message(issue.message.clone());
    }
    Value::Null
}

/// `templateOrStatic(staticSchema)`.
pub fn template_or_static(static_schema: Schema) -> Schema {
    z::string().transform(move |value, ctx| template_or_static_value(value, ctx, &static_schema))
}

/// `optionalTemplateOrStatic(staticSchema)`.
pub fn optional_template_or_static(static_schema: Schema) -> Schema {
    z::preprocess(
        blank_to_undefined,
        template_or_static(static_schema).optional(),
    )
}

fn meta_capi_content_ids_static_schema() -> Schema {
    z::string().trim().min(1)
}

fn meta_capi_event_fields_schema_content_ids_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    template_or_static_value(value, ctx, &meta_capi_content_ids_static_schema())
}

fn meta_capi_event_fields_schema_value_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    template_or_static_value(value, ctx, &deps::meta_capi_value_schema())
}

fn meta_capi_event_fields_schema_currency_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    template_or_static_value(value, ctx, &deps::meta_capi_currency_schema())
}

/// `requireValueAndCurrencyForEvent`: Purchase needs value and currency.
pub fn require_value_and_currency_for_event(data: &Value, ctx: &mut RefineCtx) {
    let event_name = str_field(data, "eventName");
    if event_name != "Purchase" {
        return;
    }
    if !truthy(data.get("value")) {
        ctx.issue(
            path!["value"],
            format!("Value is required for {event_name} events"),
        );
    }
    if !truthy(data.get("currency")) {
        ctx.issue(
            path!["currency"],
            format!("Currency is required for {event_name} events"),
        );
    }
}

/// `requireEventNameAllowedForActionSource`.
pub fn require_event_name_allowed_for_action_source(data: &Value, ctx: &mut RefineCtx) {
    use crate::utils::meta_capi::{
        META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES, MetaCapiActionSource, event_names_by_catalog,
        meta_capi_action_source_policy,
    };
    let event_name = str_field(data, "eventName");
    let action_source = str_field(data, "actionSource");
    let Some(source) = MetaCapiActionSource::parse(action_source) else {
        return;
    };
    let policy = meta_capi_action_source_policy(source);
    if event_names_by_catalog(policy.event_catalog).contains(&event_name) {
        return;
    }
    if !policy.allows_custom_event_names {
        ctx.issue(
            path!["eventName"],
            format!(
                "\"{event_name}\" is not a supported event for the {action_source} action source"
            ),
        );
        return;
    }
    if META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES.contains(&event_name) {
        ctx.issue(
            path!["eventName"],
            format!(
                "\"{event_name}\" is reserved by another action source's event catalog and cannot be used as a custom event name"
            ),
        );
    }
}

/// `withMetaCapiEventRefinements(schema)`.
pub fn with_meta_capi_event_refinements(schema: Schema) -> Schema {
    schema
        .super_refine(require_value_and_currency_for_event)
        .super_refine(require_event_name_allowed_for_action_source)
}
