//! Port of the parts of `partials/google-ads.ts` that the credential and settings
//! partials use.

use serde::{Deserialize, Serialize};

use super::shared::{Issues, ZodError, deserialize, js_length};

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
    let object = value
        .as_object()
        .ok_or_else(|| ZodError::one("", "Invalid input: expected object"))?;
    let mut issues = Issues::default();
    let mut sources = Vec::with_capacity(2);
    for key in ["adUserData", "adPersonalization"] {
        match parse_consent_source(object.get(key).unwrap_or(&serde_json::Value::Null)) {
            Ok(source) => sources.push(source),
            Err(error) => {
                for issue in error.issues {
                    let path = if issue.path.is_empty() {
                        key.to_owned()
                    } else {
                        format!("{key}.{}", issue.path)
                    };
                    issues.add(path, issue.message);
                }
            }
        }
    }
    if !issues.0.is_empty() {
        return Err(ZodError { issues: issues.0 });
    }
    let ad_personalization = sources.pop().unwrap_or(GoogleAdsConsentSource::NotProvided);
    let ad_user_data = sources.pop().unwrap_or(GoogleAdsConsentSource::NotProvided);
    Ok(GoogleAdsConsent {
        ad_user_data,
        ad_personalization,
    })
}

/// `googleAdsSettingsDocumentSchema.safeParse`.
pub fn parse_google_ads_settings_document(
    value: &serde_json::Value,
) -> Result<GoogleAdsSettingsDocument, ZodError> {
    let version = value.get("version").and_then(serde_json::Value::as_i64);
    if version != Some(1) {
        return Err(ZodError::one("version", "Invalid input"));
    }
    let consent =
        parse_google_ads_consent(value.get("consent").unwrap_or(&serde_json::Value::Null))
            .map_err(|error| ZodError {
                issues: error
                    .issues
                    .into_iter()
                    .map(|issue| super::shared::ZodIssue {
                        path: format!("consent.{}", issue.path),
                        message: issue.message,
                    })
                    .collect(),
            })?;
    Ok(GoogleAdsSettingsDocument {
        version: 1,
        consent,
    })
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
    let version = value.get("version").and_then(serde_json::Value::as_i64);
    if !matches!(version, Some(1 | 2)) {
        return Err(ZodError::one("version", "Invalid input"));
    }
    let mut options: GoogleAdsEventOptions = deserialize(value)?;
    if options.version == 1 {
        // zod strips the v2-only keys from a v1 document.
        options.matching = None;
        options.customer_properties = None;
    }
    let mut issues = Issues::default();
    if options.identity.version != 1 {
        issues.add("identity.version", "Invalid input: expected 1");
    }
    if let Some(id) = &options.identity.id {
        check_length(&mut issues, "identity.id", id, 64);
    }
    if let Some(matching) = &options.matching {
        for (path, template) in [
            ("matching.email", &matching.email),
            ("matching.phone", &matching.phone),
        ] {
            if let Some(template) = template {
                check_length(&mut issues, path, template, 200);
            }
        }
    }
    issues.finish(options)
}

/// `z.string().min(1).max(max)`.
fn check_length(issues: &mut Issues, path: &str, text: &str, max: usize) {
    let length = js_length(text);
    if length < 1 {
        issues.add(path, "Too small: expected string to have >=1 characters");
    } else if length > max {
        issues.add(
            path,
            format!("Too big: expected string to have <={max} characters"),
        );
    }
}

/// `containsVariablePlaceholder` (`@chatbotx.io/utils/variables`; the same regex as
/// `chatbotx_core::utils::variables`, which crates/db cannot depend on).
fn contains_variable_placeholder(value: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{\{(coupon:[^{}\n]+|raw:[^{}\n]+|[^{}\n]+)\}\}")
            .expect("placeholder pattern")
    });
    RE.is_match(value)
}

/// `googleAdsConsentSourceSchema.safeParse` (a non-variable source drops a leftover
/// `template`).
fn parse_consent_source(value: &serde_json::Value) -> Result<GoogleAdsConsentSource, ZodError> {
    let source: GoogleAdsConsentSource = deserialize(value)?;
    let GoogleAdsConsentSource::Variable { template } = source else {
        return Ok(source);
    };
    let template = template.trim().to_owned();
    let length = js_length(&template);
    let mut issues = Issues::default();
    if length < 1 {
        issues.add("template", "googleAds.consent.validation.templateRequired");
    }
    if length > 200 {
        issues.add("template", "googleAds.consent.validation.templateTooLong");
    }
    if !contains_variable_placeholder(&template) {
        issues.add("template", "googleAds.consent.validation.templateRequired");
    }
    issues.finish(GoogleAdsConsentSource::Variable { template })
}
