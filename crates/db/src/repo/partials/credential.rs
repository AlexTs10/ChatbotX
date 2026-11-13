//! Port of `partials/credential.ts` (the AAD format and the WhatsApp / Messenger
//! credential schemas). A zod schema is a serde type; `parse_*` deserializes a JSON value
//! and runs the zod checks (`trim`, `min(1)`); the error lists the failing path.

use serde::{Deserialize, Serialize};

use super::google_ads::GoogleAdsUploadMethod;
use crate::repo::{Error, Result};

const LEGACY_AAD_SUFFIX: &str = "false";

/// `userCredentialAad`.
pub fn user_credential_aad(user_id: &str, credential_type: &str) -> String {
    format!("user:{user_id}:{credential_type}:{LEGACY_AAD_SUFFIX}")
}

/// `platformCredentialAad`.
pub fn platform_credential_aad(credential_type: &str) -> String {
    format!("platform:{credential_type}:{LEGACY_AAD_SUFFIX}")
}

/// `credentialAadFor`: the platform AAD without a `user_id`, else the user AAD.
pub fn credential_aad_for(user_id: Option<&str>, credential_type: &str) -> String {
    match user_id {
        None => platform_credential_aad(credential_type),
        Some(user_id) => user_credential_aad(user_id, credential_type),
    }
}

/// `whatsappCredentialSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappCredential {
    pub client_id: String,
    pub version: String,
    pub config_id: String,
    pub system_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id: Option<String>,
    pub business_name: String,
    pub verify_token: String,
    pub client_secret: String,
    pub system_user_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capi_config_id: Option<String>,
}

/// `whatsappCredentialPublicSchema` (no secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappCredentialPublic {
    pub client_id: String,
    pub version: String,
    pub config_id: String,
    pub system_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id: Option<String>,
    pub business_name: String,
    pub verify_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capi_config_id: Option<String>,
}

/// `whatsappCredentialUpdateSchema` (trimmed; the secrets are optional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappCredentialUpdate {
    pub client_id: String,
    pub version: String,
    pub config_id: String,
    pub system_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id: Option<String>,
    pub business_name: String,
    pub verify_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_user_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capi_config_id: Option<String>,
}

/// `messengerCredentialSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessengerCredential {
    pub client_id: String,
    pub version: String,
    pub verify_token: String,
    pub client_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketing_messages_config_id: Option<String>,
}

/// `messengerCredentialPublicSchema` (no secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessengerCredentialPublic {
    pub client_id: String,
    pub version: String,
    pub verify_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketing_messages_config_id: Option<String>,
}

/// `messengerCredentialUpdateSchema` (trimmed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessengerCredentialUpdate {
    pub client_id: String,
    pub version: String,
    pub verify_token: String,
    pub client_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketing_messages_config_id: Option<String>,
}

/// A zod object parse: unknown keys are dropped, a wrong or missing field is an error.
fn deserialize<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|e| Error::msg(e.to_string()))
}

/// `z.string().trim().min(1)`.
fn trim_min1(path: &str, field: &mut String) -> Result<()> {
    *field = field.trim().to_owned();
    if field.is_empty() {
        return Err(Error::msg(format!(
            "{path}: Too small: expected string to have >=1 characters"
        )));
    }
    Ok(())
}

/// `z.string().trim().optional()`.
fn trim_opt(field: &mut Option<String>) {
    if let Some(value) = field {
        *value = value.trim().to_owned();
    }
}

/// `whatsappCredentialSchema.parse`.
pub fn parse_whatsapp_credential(value: &serde_json::Value) -> Result<WhatsappCredential> {
    deserialize(value)
}

/// `whatsappCredentialPublicSchema.parse` (strips the secrets).
pub fn parse_whatsapp_credential_public(
    value: &serde_json::Value,
) -> Result<WhatsappCredentialPublic> {
    deserialize(value)
}

/// `whatsappCredentialUpdateSchema.parse`.
pub fn parse_whatsapp_credential_update(
    value: &serde_json::Value,
) -> Result<WhatsappCredentialUpdate> {
    let mut v: WhatsappCredentialUpdate = deserialize(value)?;
    for (path, field) in [
        ("clientId", &mut v.client_id),
        ("version", &mut v.version),
        ("configId", &mut v.config_id),
        ("systemUserId", &mut v.system_user_id),
        ("businessName", &mut v.business_name),
        ("verifyToken", &mut v.verify_token),
    ] {
        trim_min1(path, field)?;
    }
    for field in [
        &mut v.business_id,
        &mut v.client_secret,
        &mut v.system_user_token,
        &mut v.capi_config_id,
    ] {
        trim_opt(field);
    }
    Ok(v)
}

/// `messengerCredentialSchema.parse`.
pub fn parse_messenger_credential(value: &serde_json::Value) -> Result<MessengerCredential> {
    deserialize(value)
}

/// `messengerCredentialPublicSchema.parse` (strips the secret).
pub fn parse_messenger_credential_public(
    value: &serde_json::Value,
) -> Result<MessengerCredentialPublic> {
    deserialize(value)
}

/// `messengerCredentialUpdateSchema.parse`.
pub fn parse_messenger_credential_update(
    value: &serde_json::Value,
) -> Result<MessengerCredentialUpdate> {
    let mut v: MessengerCredentialUpdate = deserialize(value)?;
    for (path, field) in [
        ("clientId", &mut v.client_id),
        ("version", &mut v.version),
        ("verifyToken", &mut v.verify_token),
        ("clientSecret", &mut v.client_secret),
    ] {
        trim_min1(path, field)?;
    }
    trim_opt(&mut v.marketing_messages_config_id);
    Ok(v)
}

/// `googleCredentialSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCredential {
    pub client_id: String,
    pub client_secret: String,
    pub verify_token: String,
}

/// `googleCredentialPublicSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCredentialPublic {
    pub client_id: String,
}

/// `googleCredentialUpdateSchema` (trimmed, all required).
pub type GoogleCredentialUpdate = GoogleCredential;

/// `googleAdsCredentialSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsCredential {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_method: Option<GoogleAdsUploadMethod>,
}

/// `googleAdsCredentialPublicSchema` (the developer token is only `hasDeveloperToken`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsCredentialPublic {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_method: Option<GoogleAdsUploadMethod>,
    pub has_developer_token: bool,
}

/// `googleAdsCredentialUpdateSchema` (trimmed; the secrets are optional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsCredentialUpdate {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_method: Option<GoogleAdsUploadMethod>,
}

/// `instagramCredentialUpdateSchema` / `zaloCredentialUpdateSchema` /
/// `threadsCredentialUpdateSchema` (trimmed, all required).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCredentialUpdate {
    pub client_id: String,
    pub version: String,
    pub verify_token: String,
    pub client_secret: String,
}

/// `giphyCredentialUpdateSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiphyCredentialUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// `stripeCredentialUpdateSchema` (`connectClientId` matches `^ca_[A-Za-z0-9]+$`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripeCredentialUpdate {
    pub publishable_key: String,
    pub verify_token: String,
    pub secret_key: String,
    pub connect_client_id: String,
}

/// `smtpCredentialUpdateSchema` (`port` is `z.coerce.number().int().positive()`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpCredentialUpdate {
    pub host: String,
    pub port: i64,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub from_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
}

/// `tiktokChannelFeaturesSchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TiktokChannelFeatures {
    pub messaging: bool,
    pub comments: bool,
}

/// `TIKTOK_DEFAULT_CHANNEL_FEATURES`.
pub const TIKTOK_DEFAULT_CHANNEL_FEATURES: TiktokChannelFeatures = TiktokChannelFeatures {
    messaging: true,
    comments: true,
};

/// `tiktokCredentialUpdateSchema` (`features` defaults; one feature must be on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiktokCredentialUpdate {
    pub client_id: String,
    pub client_secret: String,
    pub features: TiktokChannelFeatures,
}

/// `googleCredentialSchema.parse`.
pub fn parse_google_credential(value: &serde_json::Value) -> Result<GoogleCredential> {
    deserialize(value)
}

/// `googleCredentialPublicSchema.parse`.
pub fn parse_google_credential_public(value: &serde_json::Value) -> Result<GoogleCredentialPublic> {
    deserialize(value)
}

/// `googleCredentialUpdateSchema.parse`.
pub fn parse_google_credential_update(value: &serde_json::Value) -> Result<GoogleCredentialUpdate> {
    let mut parsed: GoogleCredentialUpdate = deserialize(value)?;
    trim_min1("clientId", &mut parsed.client_id)?;
    trim_min1("clientSecret", &mut parsed.client_secret)?;
    trim_min1("verifyToken", &mut parsed.verify_token)?;
    Ok(parsed)
}

/// `googleAdsCredentialSchema.parse`.
pub fn parse_google_ads_credential(value: &serde_json::Value) -> Result<GoogleAdsCredential> {
    deserialize(value)
}

/// `googleAdsCredentialPublicSchema.parse`.
pub fn parse_google_ads_credential_public(
    value: &serde_json::Value,
) -> Result<GoogleAdsCredentialPublic> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Picked {
        client_id: String,
        #[serde(default)]
        developer_token: Option<String>,
        #[serde(default)]
        upload_method: Option<GoogleAdsUploadMethod>,
    }
    let picked: Picked = deserialize(value)?;
    Ok(GoogleAdsCredentialPublic {
        client_id: picked.client_id,
        upload_method: picked.upload_method,
        // `Boolean(developerToken)`: an empty token is no token.
        has_developer_token: picked.developer_token.is_some_and(|t| !t.is_empty()),
    })
}

/// `googleAdsCredentialUpdateSchema.parse`.
pub fn parse_google_ads_credential_update(
    value: &serde_json::Value,
) -> Result<GoogleAdsCredentialUpdate> {
    let mut parsed: GoogleAdsCredentialUpdate = deserialize(value)?;
    trim_min1("clientId", &mut parsed.client_id)?;
    trim_opt(&mut parsed.client_secret);
    trim_opt(&mut parsed.developer_token);
    Ok(parsed)
}

fn parse_app_credential_update(value: &serde_json::Value) -> Result<AppCredentialUpdate> {
    let mut parsed: AppCredentialUpdate = deserialize(value)?;
    trim_min1("clientId", &mut parsed.client_id)?;
    trim_min1("version", &mut parsed.version)?;
    trim_min1("verifyToken", &mut parsed.verify_token)?;
    trim_min1("clientSecret", &mut parsed.client_secret)?;
    Ok(parsed)
}

/// `instagramCredentialUpdateSchema.parse`.
pub fn parse_instagram_credential_update(value: &serde_json::Value) -> Result<AppCredentialUpdate> {
    parse_app_credential_update(value)
}

/// `zaloCredentialUpdateSchema.parse`.
pub fn parse_zalo_credential_update(value: &serde_json::Value) -> Result<AppCredentialUpdate> {
    parse_app_credential_update(value)
}

/// `giphyCredentialUpdateSchema.parse`.
pub fn parse_giphy_credential_update(value: &serde_json::Value) -> Result<GiphyCredentialUpdate> {
    let mut parsed: GiphyCredentialUpdate = deserialize(value)?;
    trim_opt(&mut parsed.api_key);
    Ok(parsed)
}

/// `stripeCredentialUpdateSchema.parse`.
pub fn parse_stripe_credential_update(value: &serde_json::Value) -> Result<StripeCredentialUpdate> {
    let mut parsed: StripeCredentialUpdate = deserialize(value)?;
    trim_min1("publishableKey", &mut parsed.publishable_key)?;
    trim_min1("verifyToken", &mut parsed.verify_token)?;
    trim_min1("secretKey", &mut parsed.secret_key)?;
    parsed.connect_client_id = parsed.connect_client_id.trim().to_owned();
    let id = &parsed.connect_client_id;
    let valid = id
        .strip_prefix("ca_")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric()));
    if !valid {
        return Err(Error::msg(
            "connectClientId: Invalid string: must match pattern /^ca_[A-Za-z0-9]+$/",
        ));
    }
    Ok(parsed)
}

/// `z.coerce.number()` (JS `Number(value)`; `None` is `NaN`).
fn coerce_number(value: Option<&serde_json::Value>) -> Option<f64> {
    use serde_json::Value;
    match value {
        None => None,
        Some(Value::Null) => Some(0.0),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Some(0.0)
            } else {
                text.parse::<f64>().ok().filter(|n| n.is_finite())
            }
        }
        Some(_) => None,
    }
}

/// zod `.email()`: `^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$`.
fn is_zod_email(text: &str) -> bool {
    static EMAIL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^[A-Za-z0-9_'+\-.]*[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$",
        )
        .expect("email pattern")
    });
    !text.starts_with('.') && !text.contains("..") && EMAIL.is_match(text)
}

/// `smtpCredentialUpdateSchema.parse`.
pub fn parse_smtp_credential_update(value: &serde_json::Value) -> Result<SmtpCredentialUpdate> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| Error::msg("Invalid input: expected object"))?;
    let port = coerce_number(object.get("port"))
        .ok_or_else(|| Error::msg("port: Invalid input: expected number, received NaN"))?;
    if port.fract() != 0.0 || port <= 0.0 || port > i64::MAX as f64 {
        return Err(Error::msg("port: expected a positive integer"));
    }
    object.insert("port".to_owned(), serde_json::json!(port as i64));
    let mut parsed: SmtpCredentialUpdate = deserialize(&serde_json::Value::Object(object))?;
    trim_min1("host", &mut parsed.host)?;
    trim_min1("username", &mut parsed.username)?;
    trim_opt(&mut parsed.password);
    trim_min1("fromEmail", &mut parsed.from_email)?;
    if !is_zod_email(&parsed.from_email) {
        return Err(Error::msg("fromEmail: Invalid email address"));
    }
    trim_opt(&mut parsed.from_name);
    Ok(parsed)
}

/// `tiktokCredentialUpdateSchema.parse`.
pub fn parse_tiktok_credential_update(value: &serde_json::Value) -> Result<TiktokCredentialUpdate> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| Error::msg("Invalid input: expected object"))?;
    // `.default(...)`: only an absent (`undefined`) value takes the default.
    if !object.contains_key("features") {
        object.insert(
            "features".to_owned(),
            serde_json::to_value(TIKTOK_DEFAULT_CHANNEL_FEATURES)
                .map_err(|e| Error::msg(e.to_string()))?,
        );
    }
    let mut parsed: TiktokCredentialUpdate = deserialize(&serde_json::Value::Object(object))?;
    trim_min1("clientId", &mut parsed.client_id)?;
    trim_min1("clientSecret", &mut parsed.client_secret)?;
    if !(parsed.features.messaging || parsed.features.comments) {
        return Err(Error::msg("features.messaging: Invalid input"));
    }
    Ok(parsed)
}
