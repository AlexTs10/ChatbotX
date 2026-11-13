//! Port of `partials/credential.ts` (the AAD format and the WhatsApp / Messenger
//! credential schemas). A zod schema is a serde type; `parse_*` deserializes a JSON value
//! and runs the zod checks (`trim`, `min(1)`); the error lists the failing path.

use serde::{Deserialize, Serialize};

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
