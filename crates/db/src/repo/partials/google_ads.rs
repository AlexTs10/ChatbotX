//! Port of the parts of `partials/google-ads.ts` that the credential and settings
//! partials use.

use serde::{Deserialize, Serialize};

use super::shared::ZodError;

/// `googleAdsUploadMethodSchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsUploadMethod {
    DataManager,
    Legacy,
}

/// `googleAdsConsentSourceSchema` (a `variable` source keeps its `{{...}}` template).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GoogleAdsConsentSource {
    NotProvided,
    Granted,
    Denied,
    Variable { template: String },
}

/// `googleAdsConsentSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsConsent {
    pub ad_user_data: GoogleAdsConsentSource,
    pub ad_personalization: GoogleAdsConsentSource,
}

/// `NOT_PROVIDED_CONSENT`.
pub const NOT_PROVIDED_CONSENT: GoogleAdsConsent = GoogleAdsConsent {
    ad_user_data: GoogleAdsConsentSource::NotProvided,
    ad_personalization: GoogleAdsConsentSource::NotProvided,
};

/// `googleAdsSettingsDocumentSchema` (only `version: 1` exists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleAdsSettingsDocument {
    pub version: i64,
    pub consent: GoogleAdsConsent,
}

/// `googleAdsConsentSchema.safeParse`.
pub fn parse_google_ads_consent(value: &serde_json::Value) -> Result<GoogleAdsConsent, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `googleAdsSettingsDocumentSchema.safeParse`.
pub fn parse_google_ads_settings_document(
    value: &serde_json::Value,
) -> Result<GoogleAdsSettingsDocument, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `googleAdsIdentityPolicyValues`.
pub const GOOGLE_ADS_IDENTITY_POLICY_VALUES: &[&str] = &["click", "id", "event"];

/// `googleAdsIdentityPolicyValues` as a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsIdentityPolicy {
    Click,
    Id,
    Event,
}

/// The `keySource` of an identity snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsKeySource {
    Click,
    Explicit,
    Occurrence,
}

/// `identitySnapshotSchema` (`id` is required and nullable, 1 to 64 characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsIdentitySnapshot {
    pub version: i64,
    pub configured_policy: GoogleAdsIdentityPolicy,
    pub effective_policy: GoogleAdsIdentityPolicy,
    pub key_source: GoogleAdsKeySource,
    #[serde(deserialize_with = "Option::deserialize")]
    pub id: Option<String>,
}

/// `consentSnapshotEntrySchema.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsConsentStatus {
    Granted,
    Denied,
}

/// `consentSnapshotEntrySchema.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsConsentSnapshotSource {
    NotProvided,
    Fixed,
    Variable,
}

/// `consentSnapshotEntrySchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleAdsConsentSnapshotEntry {
    #[serde(deserialize_with = "Option::deserialize")]
    pub status: Option<GoogleAdsConsentStatus>,
    pub source: GoogleAdsConsentSnapshotSource,
}

/// The consent block of the event options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsConsentSnapshot {
    pub ad_user_data: GoogleAdsConsentSnapshotEntry,
    pub ad_personalization: GoogleAdsConsentSnapshotEntry,
}

/// `timeSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsTimeSource {
    Recorded,
    Provided,
}

/// The `status` of a matching / customer-properties snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAdsSnapshotStatus {
    Enabled,
    WithheldConsent,
    UnsupportedTransport,
}

/// `googleAdsMatchingSnapshotSchema` (`email` / `phone` required and nullable, 1 to 200
/// characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleAdsMatchingSnapshot {
    pub status: GoogleAdsSnapshotStatus,
    #[serde(deserialize_with = "Option::deserialize")]
    pub email: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub phone: Option<String>,
}

/// `googleAdsCustomerTypes` (`@chatbotx.io/utils/google-click`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GoogleAdsCustomerType {
    New,
    Returning,
}

/// `googleAdsCustomerValueBuckets` (`@chatbotx.io/utils/google-click`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GoogleAdsCustomerValueBucket {
    Low,
    Medium,
    High,
}

/// `googleAdsCustomerPropertiesSnapshotSchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsCustomerPropertiesSnapshot {
    pub status: GoogleAdsSnapshotStatus,
    #[serde(deserialize_with = "Option::deserialize")]
    pub customer_type: Option<GoogleAdsCustomerType>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub customer_value_bucket: Option<GoogleAdsCustomerValueBucket>,
}

/// `googleAdsEventOptionsSchema` (v1, or v2 with the optional `matching` and
/// `customerProperties`; a v1 document drops them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAdsEventOptions {
    pub version: i64,
    pub identity: GoogleAdsIdentitySnapshot,
    pub time_source: GoogleAdsTimeSource,
    pub consent: GoogleAdsConsentSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching: Option<GoogleAdsMatchingSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_properties: Option<GoogleAdsCustomerPropertiesSnapshot>,
}

/// `googleAdsEventOptionsSchema.safeParse`.
pub fn parse_google_ads_event_options(
    value: &serde_json::Value,
) -> Result<GoogleAdsEventOptions, ZodError> {
    let _ = value;
    todo!("P3.2b")
}
