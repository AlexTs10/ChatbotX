//! Port of `packages/flow-config/src/steps/send-meta-capi-event.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT templateOrStatic:
// staticSchema=>z.string().transform((value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER})

// HAND-PORT optionalTemplateOrStatic:
// staticSchema=>z.preprocess(blankToUndefined,templateOrStatic(staticSchema).optional())

// HAND-PORT requireValueAndCurrencyForEvent:
// (data,ctx)=>{if(!eventsRequiringValueAndCurrency.has(data.eventName)){return}if(!data.value){ctx.addIssue({code:"custom",path:["value"],message:`Value is required for ${data.eventName} events`})}if(!data.currency){ctx.addIssue({code:"custom",path:["currency"],message:`Currency is required for ${data.eventName} events`})}}

// HAND-PORT requireEventNameAllowedForActionSource:
// (data,ctx)=>{const policy=metaCapiActionSourcePolicy[data.actionSource];const ownCatalogNames=eventNamesByCatalog[policy.eventCatalog];if(ownCatalogNames.includes(data.eventName)){return}if(!policy.allowsCustomEventNames){ctx.addIssue({code:"custom",path:["eventName"],message:`"${data.eventName}" is not a supported event for the ${data.actionSource} action source`});return}if(metaCapiBusinessMessagingEventNames.includes(data.eventName)){ctx.addIssue({code:"custom",path:["eventName"],message:`"${data.eventName}" is reserved by another action source's event catalog and cannot be used as a custom event name`})}}

schema!(pub fn meta_capi_event_fields_schema = z::object(vec![("eventName", deps::meta_capi_event_name_schema().default(json!("LeadSubmitted")).describe("Meta event name. `business_messaging` allows only its documented events (e.g. LeadSubmitted, Purchase); other action sources allow Meta Pixel standard events or a custom name up to 50 characters.")),
("actionSource", deps::meta_capi_action_source_schema().default(json!("business_messaging")).describe("Where the conversion happened (e.g. `business_messaging`, `website`, `email`); it decides which event names are valid.")),
("contentType", deps::meta_capi_content_type_schema().optional().describe("Kind of the content ids: `product` or `product_group`.")),
("contentIds", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_content_ids_transform).optional()).describe("Product ids (retailer ids from the catalog) the event relates to.")),
("value", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_value_transform).optional()).describe("Monetary value of the conversion as a decimal string, e.g. `49.90`. Required for Purchase.")),
("currency", z::preprocess(blank_to_undefined, z::string().transform(meta_capi_event_fields_schema_currency_transform).optional()).describe("ISO 4217 currency code, e.g. `USD`. Required for Purchase.")),
("contentCategory", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Category of the content, free text.")),
("contentName", z::preprocess(blank_to_undefined, z::string().trim().min(1).max(200).optional()).describe("Name of the content, free text."))]));

// HAND-PORT withMetaCapiEventRefinements:
// schema=>schema.superRefine(requireValueAndCurrencyForEvent).superRefine(requireEventNameAllowedForActionSource)

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

// value=>typeof value==="string"&&value.trim()===""?void 0:value
fn blank_to_undefined(value: Option<Value>) -> Option<Value> {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn meta_capi_event_fields_schema_content_ids_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn meta_capi_event_fields_schema_value_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{if(containsVariablePlaceholder(value)){return value}const result=staticSchema.safeParse(value);if(result.success){return result.data}for(const issue of result.error.issues){ctx.addIssue(issue.message)}return z.NEVER}
fn meta_capi_event_fields_schema_currency_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (data,ctx)=>{if(!eventsRequiringValueAndCurrency.has(data.eventName)){return}if(!data.value){ctx.addIssue({code:"custom",path:["value"],message:`Value is required for ${data.eventName} events`})}if(!data.currency){ctx.addIssue({code:"custom",path:["currency"],message:`Currency is required for ${data.eventName} events`})}}
fn require_value_and_currency_for_event(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// (data,ctx)=>{const policy=metaCapiActionSourcePolicy[data.actionSource];const ownCatalogNames=eventNamesByCatalog[policy.eventCatalog];if(ownCatalogNames.includes(data.eventName)){return}if(!policy.allowsCustomEventNames){ctx.addIssue({code:"custom",path:["eventName"],message:`"${data.eventName}" is not a supported event for the ${data.actionSource} action source`});return}if(metaCapiBusinessMessagingEventNames.includes(data.eventName)){ctx.addIssue({code:"custom",path:["eventName"],message:`"${data.eventName}" is reserved by another action source's event catalog and cannot be used as a custom event name`})}}
fn require_event_name_allowed_for_action_source(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
