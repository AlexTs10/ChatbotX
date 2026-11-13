//! Port of `whatsappRegistrationErrorSchema` of `partials/integration-whatsapp.ts`.

use serde::{Deserialize, Serialize};

use super::shared::ZodError;

/// `z.union([z.string(), z.number()])`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextOrNumber {
    Text(String),
    Number(serde_json::Number),
}

/// `whatsappRegistrationErrorSchema` (`IntegrationWhatsapp.registrationError`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappRegistrationError {
    pub code: TextOrNumber,
    /// `.nullable()`: the key is required, the value may be `null`.
    #[serde(deserialize_with = "Option::deserialize")]
    pub sub_code: Option<TextOrNumber>,
    pub message: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fbtrace_id: Option<String>,
    pub at: String,
}

/// `whatsappRegistrationErrorSchema.parse`.
pub fn parse_whatsapp_registration_error(
    value: &serde_json::Value,
) -> Result<WhatsappRegistrationError, ZodError> {
    let _ = value;
    todo!("P3.2b")
}
