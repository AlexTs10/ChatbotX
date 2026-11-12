//! Schemas from `@chatbotx.io/utils` used by flow-config.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn channel_types = z::enum_(&["omnichannel", "webchat", "messenger", "whatsapp", "zalo", "smtp", "telegram", "instagram", "threads", "tiktok", "api"]));

schema!(pub fn error_log_providers = z::enum_(&["webchat", "messenger", "whatsapp", "zalo", "smtp", "telegram", "instagram", "threads", "tiktok", "api", "meta-catalog", "meta-conversions", "facebook-ads", "mailchimp", "sendgrid", "active-campaign", "klaviyo", "get-response", "drip", "mailer-lite", "moosend", "google-sheets", "external-request", "google-calendar", "google-ads", "openai", "gemini", "claude", "deepseek", "openrouter", "muse", "openai-compatible"]));

schema!(pub fn custom_field_types = z::enum_(&["shortText", "email", "phoneNumber", "number", "date", "datetime", "boolean", "longText"]));

schema!(pub fn meta_capi_event_name_schema = z::string().trim().min(1).max(50));

schema!(pub fn meta_capi_action_source_schema = z::enum_(&["business_messaging", "email", "phone_call", "chat", "physical_store", "system_generated", "other"]));

schema!(pub fn meta_capi_content_type_schema = z::enum_(&["product", "product_group"]));

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn zod_url_with_variables(message: Option<&str>) -> Schema {
    todo!("P3.4")
}
