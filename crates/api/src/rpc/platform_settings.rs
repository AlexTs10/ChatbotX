//! The platform settings actions of the builder (Q.D11 slice 10) under the
//! `/rpc` key `platformSettingsActions`:
//!
//! - `updatePlatformBranding` (`platformAdminActionClient`, the caller's own
//!   tenant) and `updateRootPlatformBranding` (`superAdminActionClient`, the
//!   root tenant) of `enterprise/features/platform-branding`;
//! - `updateEmailTemplate` / `updateRootEmailTemplate` of
//!   `enterprise/features/platform-email-templates`;
//! - `updatePlatformSmtp` / `deletePlatformSmtp` of
//!   `enterprise/features/platform-smtp`;
//! - `updatePlatformChannels` (own tenant) / `updateRootPlatformChannels`
//!   (root tenant) of `features/platform-channels`;
//! - `updateDefaultTemplate` / `clearDefaultTemplate` (own tenant) and
//!   `updateRootDefaultTemplate` / `clearRootDefaultTemplate` (root tenant)
//!   of `features/platform-default-template` (Q.D4 slice 6);
//!
//! and under the key `billingActions`: `reconcileTenantEntitlement`
//! (`enterprise/features/billing`).
//!
//! `updatePlatformSmtp` checks the server first (`verifySmtpTransport`).
//! `previewEmailTemplate` (Q.F2-admin) renders `compileEmailPreview` of
//! `@chatbotx.io/mail/preview`: MJML through `mrml`, plain HTML wrapped.

use chatbotx_business::template::tenant_default as default_template;
use chatbotx_business::tenant::{self, ROOT_TENANT_ID};
use chatbotx_db::rusqlite::types::Value as SqlValue;
use serde_json::{Map, Value, json};

use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::platform_credentials::{is_super_admin, save, stored, unauthorized};
use crate::handlers::ApiState;
use crate::handlers::contacts::is_email;
use crate::handlers::input::{Input, Num, Seg, Str, child, key};

/// The actions of the `platformSettingsActions` key.
pub fn action_gate(name: &str) -> Option<Gate> {
    matches!(
        name,
        "updatePlatformBranding"
            | "updateRootPlatformBranding"
            | "updateEmailTemplate"
            | "updateRootEmailTemplate"
            | "updatePlatformSmtp"
            | "deletePlatformSmtp"
            | "updateDefaultTemplate"
            | "clearDefaultTemplate"
            | "updateRootDefaultTemplate"
            | "clearRootDefaultTemplate"
            | "updatePlatformChannels"
            | "updateRootPlatformChannels"
            | "previewEmailTemplate"
    )
    .then_some(Gate::Auth)
}

/// The actions of the `billingActions` key.
pub fn billing_action_gate(name: &str) -> Option<Gate> {
    (name == "reconcileTenantEntitlement").then_some(Gate::Auth)
}

/// `themeOptions`.
const THEMES: &[&str] = &[
    "Amber", "Blue", "Cyan", "Emerald", "Fuchsia", "Green", "Indigo", "Lime", "Orange", "Pink",
    "Purple", "Red", "Rose", "Sky", "Stone", "Teal", "Violet", "Yellow",
];
/// `emailTemplateTypes` and their `Tenant` columns.
const TEMPLATES: &[(&str, &str)] = &[
    ("signup", "signupEmailTemplate"),
    ("forgotPassword", "forgotPasswordEmailTemplate"),
    ("magicLink", "magicLinkEmailTemplate"),
    ("accountCredentials", "accountCredentialsEmailTemplate"),
];

/// `isPlatformAdmin`: on cloud the owner of an active tenant, else a super admin.
async fn is_platform_admin(api: &ApiState, user_id: i64) -> Result<bool, ActionError> {
    if api.edition.is_cloud() {
        let owned = tenant::find_by_owner(&api.db, user_id).await?;
        return Ok(owned.is_some_and(|t| t.status == "active"));
    }
    is_super_admin(api, user_id).await
}

async fn assert_enterprise_features(api: &ApiState) -> Result<(), ActionError> {
    Ok(api.assert_enterprise_features().await?)
}

fn text_or_null(value: Option<String>) -> SqlValue {
    value.map_or(SqlValue::Null, SqlValue::Text)
}

fn is_url(text: &str) -> bool {
    url::Url::parse(text).is_ok()
}

/// `z.union([z.url(), z.literal("")])` / `+ z.null()` at `name`.
fn url_or_empty(input: &mut Input, name: &str, nullable: bool) -> Option<String> {
    let value = input.get(name).cloned();
    match value {
        Some(Value::Null) if nullable => None,
        Some(Value::String(text)) if text.is_empty() || is_url(&text) => Some(text),
        _ => {
            input.issue(&key(name), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `logoField` (`{ url: url | "", mode: "file" | "url" = "file" }`): the
/// stored path (`url || null`).
fn logo(input: &mut Input, name: &str) -> Option<String> {
    let Some(Value::Object(field)) = input.get(name).cloned() else {
        input.issue(&key(name), "invalid_type", "Invalid input: expected object");
        return None;
    };
    match field.get("url") {
        Some(Value::String(url)) if url.is_empty() || is_url(url) => {}
        _ => input.issue(&child(&key(name), "url"), "invalid_union", "Invalid input"),
    }
    if let Some(mode) = field.get("mode") {
        input.enum_at(
            &child(&key(name), "mode"),
            Some(mode),
            &["file", "url"],
            true,
        );
    }
    field
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
}

/// `updatePlatformBrandingSchema` → `toTenantBrandingData`.
fn branding(input: &mut Input) -> Vec<(&'static str, SqlValue)> {
    let brand_name = input.nullish_string("brandName", Str::trimmed(1, usize::MAX));
    let logo_light = logo(input, "logoLight");
    let logo_dark = logo(input, "logoDark");
    let favicon = logo(input, "favicon");
    let theme = match input.get("theme").cloned() {
        None | Some(Value::Null) => None,
        Some(value) => input.enum_at(&key("theme"), Some(&value), THEMES, true),
    };
    let custom_css = input.nullish_string("customCss", Str::max(50_000));
    let custom_js = input.nullish_string("customJs", Str::max(50_000));
    let policy = url_or_empty(input, "policyUrl", true);
    let terms = url_or_empty(input, "termsOfServiceUrl", true);
    vec![
        ("brandName", text_or_null(brand_name.flatten())),
        ("logoLightPath", text_or_null(logo_light)),
        ("logoDarkPath", text_or_null(logo_dark)),
        ("faviconPath", text_or_null(favicon)),
        ("theme", text_or_null(theme)),
        ("customCss", text_or_null(custom_css.flatten())),
        ("customJs", text_or_null(custom_js.flatten())),
        ("policyUrl", text_or_null(policy)),
        ("termsOfServiceUrl", text_or_null(terms)),
    ]
}

/// `updateEmailTemplateSchema` → `toTemplateUpdate`: the column and the
/// stored `{ subject?, body }` (`null` for a blank body).
fn template(input: &mut Input) -> Vec<(&'static str, SqlValue)> {
    let kinds: Vec<&str> = TEMPLATES.iter().map(|(k, _)| *k).collect();
    let kind = {
        let value = input.get("type").cloned();
        input.enum_at(&key("type"), value.as_ref(), &kinds, true)
    };
    let subject = input.nullish_string("subject", Str::max(200)).flatten();
    let body = input.nullish_string("body", Str::max(100_000)).flatten();
    let Some(column) = kind.and_then(|k| TEMPLATES.iter().find(|(t, _)| *t == k)) else {
        return Vec::new();
    };
    let stored = match body {
        Some(body) if !body.trim().is_empty() => {
            let mut out = Map::new();
            if let Some(subject) = subject {
                out.insert("subject".into(), Value::String(subject));
            }
            out.insert("body".into(), Value::String(body));
            SqlValue::Text(Value::Object(out).to_string())
        }
        _ => SqlValue::Null,
    };
    vec![(column.1, stored)]
}

/// `smtpCredentialUpdateSchema`.
fn smtp(input: &mut Input) -> (Map<String, Value>, Option<String>) {
    const REQUIRED: Str = Str {
        trim: true,
        min: Some(1),
        max: None,
        pattern: None,
    };
    const TRIMMED: Str = Str {
        trim: true,
        min: None,
        max: None,
        pattern: None,
    };
    let mut config = Map::new();
    if let Some(host) = input.string("host", REQUIRED) {
        config.insert("host".into(), json!(host));
    }
    let port = {
        let value = input.get("port").cloned();
        input.coerce_number_at(&key("port"), value.as_ref(), Num::int(1.0, f64::MAX), true)
    };
    if let Some(port) = port {
        config.insert("port".into(), json!(port as i64));
    }
    if let Some(username) = input.string("username", REQUIRED) {
        config.insert("username".into(), json!(username));
    }
    let password = input.opt_string("password", TRIMMED);
    let from_email = input.string(
        "fromEmail",
        Str {
            pattern: Some(is_email),
            ..REQUIRED
        },
    );
    if let Some(from_email) = from_email {
        config.insert("fromEmail".into(), json!(from_email));
    }
    if let Some(from_name) = input.opt_string("fromName", TRIMMED) {
        config.insert("fromName".into(), json!(from_name));
    }
    (config, password)
}

/// Runs one action of the `platformSettingsActions` or `billingActions` key.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let user = context.user_id;
    if name == "reconcileTenantEntitlement" {
        if api.edition.is_cloud() {
            tenant::reconcile_owner_entitlement(&api.db, user).await?;
        }
        return Ok(None);
    }
    if name == "previewEmailTemplate" {
        return preview_email_template(api, user, raw).await;
    }
    // The client middleware runs before the input schema.
    let own_tenant = matches!(
        name,
        "updatePlatformBranding"
            | "updateEmailTemplate"
            | "updateDefaultTemplate"
            | "clearDefaultTemplate"
            | "updatePlatformChannels"
    );
    let allowed = if own_tenant {
        is_platform_admin(api, user).await?
    } else {
        is_super_admin(api, user).await?
    };
    if !allowed {
        return Err(unauthorized());
    }
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    match name {
        "updatePlatformBranding"
        | "updateRootPlatformBranding"
        | "updateEmailTemplate"
        | "updateRootEmailTemplate" => {
            let data = if name.contains("Branding") {
                branding(&mut input)
            } else {
                template(&mut input)
            };
            input.finish()?;
            assert_enterprise_features(api).await?;
            if own_tenant {
                tenant::upsert_by_owner(&api.db, user, data).await?;
            } else {
                tenant::upsert_by_id(&api.db, ROOT_TENANT_ID, data).await?;
            }
            Ok(None)
        }
        "updatePlatformSmtp" => {
            let (mut config, password) = smtp(&mut input);
            input.finish()?;
            assert_enterprise_features(api).await?;
            let password = match password.filter(|p| !p.is_empty()) {
                Some(p) => Some(p),
                None => stored(api, None, "smtp")
                    .await?
                    .and_then(|c| c.get("password").and_then(Value::as_str).map(str::to_owned))
                    .filter(|p| !p.is_empty()),
            };
            let Some(password) = password else {
                return Err(ActionError::ValidationErrors(json!({
                    "password": { "_errors": ["Enter the SMTP password."] }
                })));
            };
            // `verifySmtpTransport`: connect, (TLS,) authenticate, quit.
            let auth = chatbotx_business::email_marketing::smtp::SmtpAuth {
                host: config["host"].as_str().unwrap_or_default().to_string(),
                port: config["port"]
                    .as_f64()
                    .and_then(|p| u16::try_from(p as i64).ok())
                    .unwrap_or_default(),
                username: config["username"].as_str().unwrap_or_default().to_string(),
                password: password.clone(),
            };
            if chatbotx_business::email_marketing::smtp::verify(&auth)
                .await
                .is_err()
            {
                return Err(ActionError::Exception(
                    chatbotx_business::BusinessError::exception(
                        "Cannot connect to SMTP server. Please check your credentials and try again.",
                        "systemError",
                        400,
                    ),
                ));
            }
            config.insert("password".into(), json!(password));
            save(api, None, "smtp", config).await?;
            Ok(None)
        }
        "deletePlatformSmtp" => {
            assert_enterprise_features(api).await?;
            chatbotx_business::tx::write(&api.db, |conn| {
                Ok(chatbotx_business::platform_credential::remove(
                    conn, None, "smtp",
                )?)
            })
            .await?;
            Ok(None)
        }
        "updateDefaultTemplate" | "updateRootDefaultTemplate" => {
            let template_id = input.digits_id("templateId");
            let key = input.string("templateKey", Str::trimmed(1, usize::MAX));
            input.finish()?;
            let Some(encryption) = api.encryption.as_deref() else {
                return Err(ActionError::Internal(
                    "encryption keys are not configured".into(),
                ));
            };
            let scope = default_template_scope(own_tenant, user);
            let saved = default_template::set(
                &api.db,
                encryption,
                scope,
                template_id.unwrap_or(-1),
                key.unwrap_or_default(),
                user,
            )
            .await;
            // `saveDefaultTemplate`: the two expected refusals are outcomes.
            let reason = match &saved {
                Ok(()) => return Ok(Some(json!({ "status": "saved" }))),
                Err(chatbotx_business::BusinessError::Exception { code, .. }) => {
                    match code.as_str() {
                        "tenantDefaultTemplateUnavailable" => Some("unavailable"),
                        "tenantDefaultTemplateShareDisabled" => Some("shareDisabled"),
                        _ => None,
                    }
                }
                Err(_) => None,
            };
            match reason {
                Some(reason) => Ok(Some(json!({ "status": "rejected", "reason": reason }))),
                None => Err(saved.unwrap_err().into()),
            }
        }
        "updatePlatformChannels" | "updateRootPlatformChannels" => {
            let hidden = hidden_channels(&mut input);
            input.finish()?;
            let data = vec![(
                "hiddenChannels",
                SqlValue::Text(Value::from(hidden).to_string()),
            )];
            if own_tenant {
                tenant::upsert_by_owner(&api.db, user, data).await?;
            } else {
                tenant::upsert_by_id(&api.db, ROOT_TENANT_ID, data).await?;
            }
            Ok(None)
        }
        "clearDefaultTemplate" | "clearRootDefaultTemplate" => {
            default_template::clear(&api.db, default_template_scope(own_tenant, user), user)
                .await?;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!("unknown action {name}"))),
    }
}

/// `updatePlatformChannelsSchema`: `channelTypes` items, every one of them
/// creatable (`CREATABLE_CHANNELS`).
fn hidden_channels(input: &mut Input) -> Vec<String> {
    use chatbotx_core::utils::channel::{ChannelType, creatable_channels};
    let path = key("hiddenChannels");
    let Some(Value::Array(items)) = input.get("hiddenChannels").cloned() else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return Vec::new();
    };
    let all: Vec<&str> = ChannelType::ALL.iter().map(|c| c.as_str()).collect();
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut at = path.clone();
        at.push(Seg::Index(index));
        if let Some(channel) = input.enum_at(&at, Some(item), &all, true) {
            out.push(channel);
        }
    }
    let creatable = creatable_channels();
    if !out
        .iter()
        .all(|c| creatable.iter().any(|k| k.as_str() == *c))
    {
        input.issue(&path, "custom", "Only creatable channels can be hidden");
    }
    out
}

/// `previewEmailTemplateAction`: the input schema, the enterprise features,
/// then a super admin or a platform admin.
async fn preview_email_template(api: &ApiState, user: i64, raw: &Value) -> ActionResult {
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    let body = input.string("body", Str::max(100_000));
    input.finish()?;
    assert_enterprise_features(api).await?;
    if !(is_super_admin(api, user).await? || is_platform_admin(api, user).await?) {
        return Err(unauthorized());
    }
    Ok(Some(Value::String(compile_email_preview(
        &body.unwrap_or_default(),
    ))))
}

/// `SAMPLE_VARS` of `compileEmailPreview`.
const SAMPLE_VARS: &[(&str, &str)] = &[
    ("userName", "John Doe"),
    (
        "verificationUrl",
        "https://example.com/verify/sample-token-abc123",
    ),
    (
        "resetPasswordUrl",
        "https://example.com/reset/sample-token-abc123",
    ),
    ("magicUrl", "https://example.com/magic/sample-token-abc123"),
    ("brandName", "ChatbotX"),
    (
        "brandLogoUrl",
        "https://placehold.co/150x90/f3f4f6/374151?text=Logo",
    ),
    ("brandUrl", "https://example.com"),
];

/// `body.replace(/\{\{(\w+)\}\}/g, …)`: a known `{{name}}` becomes its
/// sample value; anything else stays as written.
fn substitute_sample_vars(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        if name_len > 0 && after[name_len..].starts_with("}}") {
            match SAMPLE_VARS.iter().find(|(k, _)| *k == name) {
                Some((_, value)) => out.push_str(value),
                None => {
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                }
            }
            rest = &after[name_len + 2..];
        } else {
            out.push_str("{{");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `compileEmailPreview`: an MJML body (the original starts with `<mjml`)
/// renders to HTML, an error to a red `<pre>`; plain HTML gets a wrapper.
pub fn compile_email_preview(body: &str) -> String {
    let substituted = substitute_sample_vars(body);
    if body.trim_start().starts_with("<mjml") {
        let rendered = mrml::parse(&substituted)
            .map_err(|e| e.to_string())
            .and_then(|parsed| {
                parsed
                    .element
                    .render(&mrml::prelude::render::RenderOptions::default())
                    .map_err(|e| e.to_string())
            });
        return match rendered {
            Ok(html) => html,
            // ponytail: mrml's message, not mjml's `formattedMessage` text.
            Err(message) => format!(
                "<html><body><pre style=\"color:red;padding:16px;font-size:13px\">{}</pre></body></html>",
                escape_html(&message)
            ),
        };
    }
    format!(
        "<!DOCTYPE html><html><body style=\"font-family:sans-serif;padding:16px\">{substituted}</body></html>"
    )
}

/// The reseller's own tenant, or the root tenant (`DefaultTemplateScope`).
fn default_template_scope(own_tenant: bool, user: i64) -> default_template::Scope {
    if own_tenant {
        default_template::Scope::Owner(user)
    } else {
        default_template::Scope::Root
    }
}
