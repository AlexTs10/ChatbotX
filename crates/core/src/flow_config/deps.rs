//! Schemas from `@chatbotx.io/utils` used by flow-config.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn channel_types = z::enum_(&["omnichannel", "webchat", "messenger", "whatsapp", "zalo", "smtp", "telegram", "instagram", "threads", "tiktok", "api"]));

schema!(pub fn error_log_providers = z::enum_(&["webchat", "messenger", "whatsapp", "zalo", "smtp", "telegram", "instagram", "threads", "tiktok", "api", "meta-catalog", "meta-conversions", "facebook-ads", "mailchimp", "sendgrid", "active-campaign", "klaviyo", "get-response", "drip", "mailer-lite", "moosend", "google-sheets", "external-request", "google-calendar", "google-ads", "openai", "gemini", "claude", "deepseek", "openrouter", "muse", "openai-compatible"]));

schema!(pub fn custom_field_types = z::enum_(&["shortText", "email", "phoneNumber", "number", "date", "datetime", "boolean", "longText"]));

schema!(pub fn meta_capi_event_name_schema = z::string().trim().min(1).max(50));

schema!(pub fn meta_capi_action_source_schema = z::enum_(&["business_messaging", "email", "phone_call", "chat", "physical_store", "system_generated", "other"]));

schema!(pub fn meta_capi_content_type_schema = z::enum_(&["product", "product_group"]));

/// `zodUrlWithVariables(message = "Invalid URL")`.
pub fn zod_url_with_variables(message: Option<&str>) -> Schema {
    z::string().refine_owned(
        url_with_variables_ok,
        Some(message.unwrap_or("Invalid URL").to_owned()),
        None,
    )
}

/// The `zodUrlWithVariables` check: a `{{variable}}` or a parseable URL.
pub fn url_with_variables_ok(value: &Value) -> bool {
    let text = value.as_str().unwrap_or("");
    crate::utils::variables::contains_variable_placeholder(text) || crate::utils::zod::is_url(text)
}

/// `metaCapiValueSchema`.
pub fn meta_capi_value_schema() -> Schema {
    z::string()
        .trim()
        .regex(r"^\d+(\.\d+)?$", "")
        .msg("Value must be a plain number such as 19.99")
        .refine(
            |v| js_number(Some(v)) <= 9_007_199_254_740_991.0,
            Some("Value is too large"),
            None,
        )
}

/// `metaCapiCurrencySchema`.
pub fn meta_capi_currency_schema() -> Schema {
    z::string().trim().to_upper_case().pipe(
        z::string()
            .regex("^[A-Z]{3}$", "")
            .msg("Currency must be a 3-letter ISO 4217 code such as USD"),
    )
}
