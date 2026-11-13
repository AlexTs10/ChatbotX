//! Port of `partials/credential.ts` (the AAD format and the WhatsApp / Messenger
//! credential schemas). A zod schema is a serde type; `parse_*` deserializes a JSON value
//! and runs the zod checks (`trim`, `min(1)`); the error lists the failing path.

use serde::{Deserialize, Serialize};

use crate::repo::{Error, Result};

const LEGACY_AAD_SUFFIX: &str = "false";

/// `userCredentialAad`.
pub fn user_credential_aad(user_id: &str, credential_type: &str) -> String {
    let _ = (user_id, credential_type, LEGACY_AAD_SUFFIX);
    todo!("P3.2b")
}

/// `platformCredentialAad`.
pub fn platform_credential_aad(credential_type: &str) -> String {
    let _ = credential_type;
    todo!("P3.2b")
}

/// `credentialAadFor`: the platform AAD without a `user_id`, else the user AAD.
pub fn credential_aad_for(user_id: Option<&str>, credential_type: &str) -> String {
    let _ = (user_id, credential_type);
    todo!("P3.2b")
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

/// `whatsappCredentialSchema.parse`.
pub fn parse_whatsapp_credential(value: &serde_json::Value) -> Result<WhatsappCredential> {
    let _ = (value, Error::msg(""));
    todo!("P3.2b")
}

/// `whatsappCredentialPublicSchema.parse` (strips the secrets).
pub fn parse_whatsapp_credential_public(
    value: &serde_json::Value,
) -> Result<WhatsappCredentialPublic> {
    let _ = value;
    todo!("P3.2b")
}

/// `whatsappCredentialUpdateSchema.parse`.
pub fn parse_whatsapp_credential_update(
    value: &serde_json::Value,
) -> Result<WhatsappCredentialUpdate> {
    let _ = value;
    todo!("P3.2b")
}

/// `messengerCredentialSchema.parse`.
pub fn parse_messenger_credential(value: &serde_json::Value) -> Result<MessengerCredential> {
    let _ = value;
    todo!("P3.2b")
}

/// `messengerCredentialPublicSchema.parse` (strips the secret).
pub fn parse_messenger_credential_public(
    value: &serde_json::Value,
) -> Result<MessengerCredentialPublic> {
    let _ = value;
    todo!("P3.2b")
}

/// `messengerCredentialUpdateSchema.parse`.
pub fn parse_messenger_credential_update(
    value: &serde_json::Value,
) -> Result<MessengerCredentialUpdate> {
    let _ = value;
    todo!("P3.2b")
}
