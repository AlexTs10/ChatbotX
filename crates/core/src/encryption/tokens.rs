//! The signed tokens of `packages/encryption` (`appointment-token-utils.ts`
//! and the token files built on it). A token is the base64url JSON of an
//! [`EncryptedData`] blob whose AAD names the token type; the payload carries
//! `expiresAt` (epoch ms).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{EncryptedData, Encryption, EncryptionError};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `appointmentCancel` / `appointmentSchedule` default TTL: 30 days.
pub const APPOINTMENT_TOKEN_TTL_MS: i64 = 30 * DAY_MS;
/// `appointmentWebview` default TTL: 15 minutes.
pub const APPOINTMENT_WEBVIEW_TOKEN_TTL_MS: i64 = 15 * 60 * 1000;
/// Booking links an agent sends from the inbox: 24 hours.
pub const APPOINTMENT_INBOX_LINK_TTL_MS: i64 = DAY_MS;
/// Leaderboard view links: 30 days.
pub const LEADERBOARD_VIEW_TOKEN_TTL_MS: i64 = 30 * DAY_MS;
/// Media proxy tokens: 1 hour.
pub const MEDIA_TOKEN_TTL_MS: i64 = 60 * 60 * 1000;
/// Mini App links: 7 days.
pub const MINI_APP_TOKEN_TTL_MS: i64 = 7 * DAY_MS;
/// Minigame play links: 24 hours.
pub const MINIGAME_PLAY_TOKEN_TTL_MS: i64 = DAY_MS;
/// Customer shop links: 7 days.
pub const SHOP_TOKEN_TTL_MS: i64 = 7 * DAY_MS;
/// Merchant shop previews: 24 hours.
pub const SHOP_PREVIEW_TOKEN_TTL_MS: i64 = DAY_MS;
/// User-data webview links: 24 hours.
pub const USER_DATA_WEBVIEW_TOKEN_TTL_MS: i64 = DAY_MS;
/// Largest valid ECMAScript time value (the `messageCreatedAt` cap).
pub const MAX_TIME_VALUE_MS: i64 = 8_640_000_000_000_000;

const APPOINTMENT_CANCEL_AAD: &str = "appointment-cancel-token";
const APPOINTMENT_SCHEDULE_AAD: &str = "appointment-schedule-token";
const APPOINTMENT_WEBVIEW_AAD: &str = "appointment-webview-token";
const LEADERBOARD_VIEW_AAD: &str = "leaderboard-view-token";
const MEDIA_AAD: &str = "media";
const MINI_APP_AAD: &str = "mini-app-token";
const MINI_APP_RESUME_AAD: &str = "mini-app-resume-token";
const MINIGAME_PLAY_AAD: &str = "minigame-play-token";
const SHOP_AAD: &str = "shop-token";
const USER_DATA_WEBVIEW_AAD: &str = "user-data-webview-token";
const POSTBACK_PREFIX: &str = "appointment_cancel:";

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(char::from(BASE64URL[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// `Buffer.from(text, "base64url")`: lenient like Node — both alphabets,
/// padding and other characters skipped, a dangling sextet dropped.
fn base64url_decode_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for b in text.bytes() {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

/// The token text of a blob: base64url of its JSON.
pub fn encode_token_blob(blob: &EncryptedData) -> Result<String, EncryptionError> {
    let json = serde_json::to_string(blob).map_err(|e| EncryptionError::Json(e.to_string()))?;
    Ok(base64url_encode(json.as_bytes()))
}

/// The blob of a token: base64url-decoded, `JSON.parse`d and checked
/// against `encryptedDataSchema`.
pub fn decode_token_blob(token: &str) -> Result<EncryptedData, EncryptionError> {
    let bytes = base64url_decode_lenient(token);
    let json = String::from_utf8_lossy(&bytes);
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| EncryptionError::Json(e.to_string()))?;
    let blob: EncryptedData =
        serde_json::from_value(value).map_err(|e| EncryptionError::Schema(e.to_string()))?;
    blob.validate()?;
    Ok(blob)
}

// ---------------------------------------------------------------------------
// Claims and payloads
// ---------------------------------------------------------------------------

/// Validation of a token payload beyond its serde shape (the zod `min(1)`
/// and refinement rules).
pub trait TokenClaims {
    /// `Err` with the first issue message.
    fn validate(&self) -> Result<(), String>;
}

fn non_empty(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(format!(
            "{name}: Too small: expected string to have >=1 characters"
        ))
    } else {
        Ok(())
    }
}

fn non_empty_opt(name: &str, value: Option<&str>) -> Result<(), String> {
    value.map_or(Ok(()), |v| non_empty(name, v))
}

/// `z.iso.datetime()`: `YYYY-MM-DDTHH:mm[:ss[.f+]]Z` on a real date.
fn is_iso_datetime(value: &str) -> bool {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"^(\d{4})-(\d{2})-(\d{2})T([01]\d|2[0-3]):[0-5]\d(?::[0-5]\d(?:\.\d+)?)?Z$",
        )
        .expect("valid regex")
    });
    let Some(caps) = pattern.captures(value) else {
        return false;
    };
    let year: i64 = caps[1].parse().unwrap_or(0);
    let month: u32 = caps[2].parse().unwrap_or(0);
    let day: u32 = caps[3].parse().unwrap_or(0);
    (1..=12).contains(&month) && day >= 1 && day <= crate::js::days_in_month(year, month)
}

/// A token payload: the claims plus `expiresAt`, in that key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPayload<C> {
    #[serde(flatten)]
    pub claims: C,
    pub expires_at: i64,
}

impl<C: TokenClaims> TokenClaims for TokenPayload<C> {
    fn validate(&self) -> Result<(), String> {
        self.claims.validate()
    }
}

/// `appointmentCancelPayloadSchema` / `appointmentSchedulePayloadSchema`
/// without `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentClaims {
    pub appointment_id: String,
    pub workspace_id: String,
    pub contact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_inbox_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_version_id: Option<String>,
}

impl TokenClaims for AppointmentClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("appointmentId", &self.appointment_id)?;
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("contactId", &self.contact_id)?;
        non_empty_opt("conversationId", self.conversation_id.as_deref())?;
        non_empty_opt("contactInboxId", self.contact_inbox_id.as_deref())?;
        non_empty_opt("flowVersionId", self.flow_version_id.as_deref())
    }
}

/// Who signed an appointment webview token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppointmentTokenSource {
    /// Signed by the appointmentScheduling flow step (the default for
    /// tokens signed before the field existed).
    #[default]
    Flow,
    /// A booking link an agent sent from the inbox.
    Inbox,
}

/// What the appointment webview lets the contact do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppointmentWebviewMode {
    #[default]
    Book,
    SelectAvailability,
    SelectAvailabilityRange,
}

/// `appointmentWebviewPayloadSchema` without `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppointmentWebviewClaims {
    #[serde(default)]
    pub source: AppointmentTokenSource,
    #[serde(default)]
    pub mode: AppointmentWebviewMode,
    pub workspace_id: String,
    pub calendar_id: String,
    pub contact_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_date_custom_field_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date_custom_field_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date_custom_field_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_custom_field_id: Option<String>,
    #[serde(
        rename = "resultUsedByAI",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub result_used_by_ai: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_start_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_end_at: Option<String>,
}

impl TokenClaims for AppointmentWebviewClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("calendarId", &self.calendar_id)?;
        non_empty("contactId", &self.contact_id)?;
        non_empty("conversationId", &self.conversation_id)?;
        non_empty("contactInboxId", &self.contact_inbox_id)?;
        for (name, value) in [
            ("channel", &self.channel),
            ("flowId", &self.flow_id),
            ("flowVersionId", &self.flow_version_id),
            ("stepId", &self.step_id),
            ("nodeId", &self.node_id),
            (
                "selectedDateCustomFieldId",
                &self.selected_date_custom_field_id,
            ),
            ("startDateCustomFieldId", &self.start_date_custom_field_id),
            ("endDateCustomFieldId", &self.end_date_custom_field_id),
            ("resultCustomFieldId", &self.result_custom_field_id),
        ] {
            non_empty_opt(name, value.as_deref())?;
        }
        for (name, value) in [
            ("availabilityStartAt", &self.availability_start_at),
            ("availabilityEndAt", &self.availability_end_at),
        ] {
            if let Some(v) = value {
                if !is_iso_datetime(v) {
                    return Err(format!("{name}: Invalid ISO datetime"));
                }
            }
        }
        if self.source == AppointmentTokenSource::Flow {
            for (name, value) in [
                ("channel", &self.channel),
                ("flowId", &self.flow_id),
                ("flowVersionId", &self.flow_version_id),
                ("stepId", &self.step_id),
            ] {
                if value.as_deref().is_none_or(str::is_empty) {
                    return Err(format!("{name} is required for flow tokens"));
                }
            }
            return Ok(());
        }
        if self.mode != AppointmentWebviewMode::Book {
            return Err("Inbox tokens only support the book mode".to_owned());
        }
        if self.node_id.as_deref().is_some_and(|n| !n.is_empty()) {
            return Err("Inbox tokens cannot resume a flow".to_owned());
        }
        Ok(())
    }
}

/// `leaderboardViewPayloadSchema` / `minigamePlayPayloadSchema` without
/// `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInboxClaims {
    pub workspace_id: String,
    pub contact_id: String,
    pub contact_inbox_id: String,
}

impl TokenClaims for ContactInboxClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("contactId", &self.contact_id)?;
        non_empty("contactInboxId", &self.contact_inbox_id)
    }
}

/// What a media token points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaTokenKind {
    Attachment,
    Avatar,
}

/// `mediaTokenClaimsSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTokenClaims {
    pub workspace_id: String,
    pub kind: MediaTokenKind,
    pub ref_id: String,
    /// Parent message `createdAt` (epoch ms), the shard hint of attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_created_at: Option<i64>,
}

impl TokenClaims for MediaTokenClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("refId", &self.ref_id)?;
        if let Some(at) = self.message_created_at {
            if !(0..=MAX_TIME_VALUE_MS).contains(&at) {
                return Err("messageCreatedAt: out of range".to_owned());
            }
        }
        Ok(())
    }
}

/// `miniAppTokenPayloadSchema` without `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppClaims {
    pub workspace_id: String,
    pub contact_id: String,
}

impl TokenClaims for MiniAppClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("contactId", &self.contact_id)
    }
}

/// `miniAppResumeTokenPayloadSchema` without `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppResumeClaims {
    pub workspace_id: String,
    pub contact_id: String,
    pub mini_app_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    /// The flow button the link was sent on (`encodeButtonPayload`).
    pub button_payload: String,
}

impl TokenClaims for MiniAppResumeClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("contactId", &self.contact_id)?;
        non_empty("miniAppId", &self.mini_app_id)?;
        non_empty("conversationId", &self.conversation_id)?;
        non_empty("contactInboxId", &self.contact_inbox_id)?;
        non_empty("buttonPayload", &self.button_payload)
    }
}

/// The customer fields of a shop link.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopCustomerClaims {
    pub workspace_id: String,
    pub contact_id: String,
    pub contact_inbox_id: Option<String>,
    pub conversation_id: Option<String>,
}

/// `shopTokenPayloadSchema` (discriminated by `mode`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ShopTokenPayload {
    /// A link sent to a contact: the cart and order are theirs.
    Customer {
        workspace_id: String,
        contact_id: String,
        contact_inbox_id: Option<String>,
        conversation_id: Option<String>,
        expires_at: i64,
    },
    /// A merchant's preview: browsable, never orderable.
    Preview {
        workspace_id: String,
        expires_at: i64,
    },
}

impl TokenClaims for ShopTokenPayload {
    fn validate(&self) -> Result<(), String> {
        match self {
            ShopTokenPayload::Customer {
                workspace_id,
                contact_id,
                contact_inbox_id,
                conversation_id,
                ..
            } => {
                non_empty("workspaceId", workspace_id)?;
                non_empty("contactId", contact_id)?;
                non_empty_opt("contactInboxId", contact_inbox_id.as_deref())?;
                non_empty_opt("conversationId", conversation_id.as_deref())
            }
            ShopTokenPayload::Preview { workspace_id, .. } => {
                non_empty("workspaceId", workspace_id)
            }
        }
    }
}

/// The reply a user-data webview collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserDataReplyFormat {
    Date,
    Datetime,
}

/// `userDataWebviewPayloadSchema` without `expiresAt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDataWebviewClaims {
    pub workspace_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub contact_id: String,
    pub channel: String,
    pub flow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_version_id: Option<String>,
    pub step_id: String,
    pub node_id: String,
    pub challenge_id: String,
    pub output_field_id: String,
    pub reply_format: UserDataReplyFormat,
}

impl TokenClaims for UserDataWebviewClaims {
    fn validate(&self) -> Result<(), String> {
        non_empty("workspaceId", &self.workspace_id)?;
        non_empty("conversationId", &self.conversation_id)?;
        non_empty("contactInboxId", &self.contact_inbox_id)?;
        non_empty("contactId", &self.contact_id)?;
        non_empty("channel", &self.channel)?;
        non_empty("flowId", &self.flow_id)?;
        non_empty_opt("flowVersionId", self.flow_version_id.as_deref())?;
        non_empty("stepId", &self.step_id)?;
        non_empty("nodeId", &self.node_id)?;
        non_empty("challengeId", &self.challenge_id)?;
        non_empty("outputFieldId", &self.output_field_id)
    }
}

/// `buildAppointmentCancelPostback`.
pub fn build_appointment_cancel_postback(token: &str) -> String {
    format!("{POSTBACK_PREFIX}{token}")
}

/// `parseAppointmentCancelPostback`: the token, or `None` when the payload
/// is not a cancel postback (or carries no token).
pub fn parse_appointment_cancel_postback(payload: Option<&str>) -> Option<String> {
    let token = payload?.strip_prefix(POSTBACK_PREFIX)?;
    (!token.is_empty()).then(|| token.to_owned())
}

// ---------------------------------------------------------------------------
// Signing and verifying
// ---------------------------------------------------------------------------

impl Encryption {
    /// `signAppointmentToken`: encrypts `payload` with `aad` and encodes it.
    pub fn sign_appointment_token<T: Serialize + ?Sized>(
        &self,
        payload: &T,
        aad: &str,
    ) -> Result<String, EncryptionError> {
        let blob = self.encrypt_object(payload, Some(aad))?;
        encode_token_blob(&blob)
    }

    /// `verifyAppointmentToken`: decodes, checks the token type (AAD),
    /// decrypts, checks the payload shape, then the expiry.
    pub fn verify_appointment_token<T: DeserializeOwned + TokenClaims>(
        &self,
        token: &str,
        aad: &str,
    ) -> Result<T, EncryptionError> {
        let blob = decode_token_blob(token)?;
        if blob.aad.as_deref() != Some(aad) {
            return Err(EncryptionError::TypeMismatch);
        }
        let text = self.decrypt_text(&blob, Some(aad))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| EncryptionError::Json(e.to_string()))?;
        let payload: T = serde_json::from_value(value.clone())
            .map_err(|e| EncryptionError::Schema(e.to_string()))?;
        payload.validate().map_err(EncryptionError::Schema)?;
        let expires_at = value.get("expiresAt").and_then(serde_json::Value::as_f64);
        match expires_at {
            Some(at) if at >= self.now_ms() as f64 => Ok(payload),
            _ => Err(EncryptionError::Expired),
        }
    }

    fn sign_claims<C: Serialize>(
        &self,
        claims: &C,
        ttl_ms: i64,
        aad: &str,
    ) -> Result<String, EncryptionError> {
        let payload = TokenPayload {
            claims,
            expires_at: self.now_ms() + ttl_ms,
        };
        self.sign_appointment_token(&payload, aad)
    }

    /// `signAppointmentCancelToken` (default TTL [`APPOINTMENT_TOKEN_TTL_MS`]).
    pub fn sign_appointment_cancel_token(
        &self,
        claims: &AppointmentClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, APPOINTMENT_CANCEL_AAD)
    }

    /// `verifyAppointmentCancelToken`.
    pub fn verify_appointment_cancel_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<AppointmentClaims>, EncryptionError> {
        self.verify_appointment_token(token, APPOINTMENT_CANCEL_AAD)
    }

    /// `verifyAppointmentCancelPostback`.
    pub fn verify_appointment_cancel_postback(
        &self,
        payload: &str,
    ) -> Result<TokenPayload<AppointmentClaims>, EncryptionError> {
        let token = parse_appointment_cancel_postback(Some(payload))
            .ok_or(EncryptionError::InvalidPostback)?;
        self.verify_appointment_cancel_token(&token)
    }

    /// `signAppointmentScheduleToken` (default TTL [`APPOINTMENT_TOKEN_TTL_MS`]).
    pub fn sign_appointment_schedule_token(
        &self,
        claims: &AppointmentClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, APPOINTMENT_SCHEDULE_AAD)
    }

    /// `verifyAppointmentScheduleToken`.
    pub fn verify_appointment_schedule_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<AppointmentClaims>, EncryptionError> {
        self.verify_appointment_token(token, APPOINTMENT_SCHEDULE_AAD)
    }

    /// `signAppointmentWebviewTokenWithExpiry`: the token and the exact
    /// `expiresAt` (epoch ms) written into it.
    pub fn sign_appointment_webview_token_with_expiry(
        &self,
        claims: &AppointmentWebviewClaims,
        ttl_ms: i64,
    ) -> Result<(String, i64), EncryptionError> {
        let expires_at = self.now_ms() + ttl_ms;
        let payload = TokenPayload { claims, expires_at };
        let token = self.sign_appointment_token(&payload, APPOINTMENT_WEBVIEW_AAD)?;
        Ok((token, expires_at))
    }

    /// `signAppointmentWebviewToken` (default TTL
    /// [`APPOINTMENT_WEBVIEW_TOKEN_TTL_MS`]).
    pub fn sign_appointment_webview_token(
        &self,
        claims: &AppointmentWebviewClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        Ok(self
            .sign_appointment_webview_token_with_expiry(claims, ttl_ms)?
            .0)
    }

    /// `verifyAppointmentWebviewToken`.
    pub fn verify_appointment_webview_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<AppointmentWebviewClaims>, EncryptionError> {
        self.verify_appointment_token(token, APPOINTMENT_WEBVIEW_AAD)
    }

    /// `signLeaderboardViewToken` (default TTL [`LEADERBOARD_VIEW_TOKEN_TTL_MS`]).
    pub fn sign_leaderboard_view_token(
        &self,
        claims: &ContactInboxClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, LEADERBOARD_VIEW_AAD)
    }

    /// `verifyLeaderboardViewToken`.
    pub fn verify_leaderboard_view_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<ContactInboxClaims>, EncryptionError> {
        self.verify_appointment_token(token, LEADERBOARD_VIEW_AAD)
    }

    /// `signMediaToken` (default TTL [`MEDIA_TOKEN_TTL_MS`]).
    pub fn sign_media_token(
        &self,
        claims: &MediaTokenClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, MEDIA_AAD)
    }

    /// `verifyMediaToken`.
    pub fn verify_media_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<MediaTokenClaims>, EncryptionError> {
        self.verify_appointment_token(token, MEDIA_AAD)
    }

    /// `signMiniAppToken` (default TTL [`MINI_APP_TOKEN_TTL_MS`]).
    pub fn sign_mini_app_token(
        &self,
        claims: &MiniAppClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, MINI_APP_AAD)
    }

    /// `verifyMiniAppToken`.
    pub fn verify_mini_app_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<MiniAppClaims>, EncryptionError> {
        self.verify_appointment_token(token, MINI_APP_AAD)
    }

    /// `signMiniAppResumeToken` (default TTL [`MINI_APP_TOKEN_TTL_MS`]).
    pub fn sign_mini_app_resume_token(
        &self,
        claims: &MiniAppResumeClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, MINI_APP_RESUME_AAD)
    }

    /// `verifyMiniAppResumeToken`.
    pub fn verify_mini_app_resume_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<MiniAppResumeClaims>, EncryptionError> {
        self.verify_appointment_token(token, MINI_APP_RESUME_AAD)
    }

    /// `signMinigamePlayToken` (default TTL [`MINIGAME_PLAY_TOKEN_TTL_MS`]).
    pub fn sign_minigame_play_token(
        &self,
        claims: &ContactInboxClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, MINIGAME_PLAY_AAD)
    }

    /// `verifyMinigamePlayToken`.
    pub fn verify_minigame_play_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<ContactInboxClaims>, EncryptionError> {
        self.verify_appointment_token(token, MINIGAME_PLAY_AAD)
    }

    /// `signShopToken` (default TTL [`SHOP_TOKEN_TTL_MS`]).
    pub fn sign_shop_token(
        &self,
        claims: &ShopCustomerClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        let payload = ShopTokenPayload::Customer {
            workspace_id: claims.workspace_id.clone(),
            contact_id: claims.contact_id.clone(),
            contact_inbox_id: claims.contact_inbox_id.clone(),
            conversation_id: claims.conversation_id.clone(),
            expires_at: self.now_ms() + ttl_ms,
        };
        self.sign_appointment_token(&payload, SHOP_AAD)
    }

    /// `signShopPreviewToken`.
    pub fn sign_shop_preview_token(&self, workspace_id: &str) -> Result<String, EncryptionError> {
        let payload = ShopTokenPayload::Preview {
            workspace_id: workspace_id.to_owned(),
            expires_at: self.now_ms() + SHOP_PREVIEW_TOKEN_TTL_MS,
        };
        self.sign_appointment_token(&payload, SHOP_AAD)
    }

    /// `verifyShopToken`.
    pub fn verify_shop_token(&self, token: &str) -> Result<ShopTokenPayload, EncryptionError> {
        self.verify_appointment_token(token, SHOP_AAD)
    }

    /// `signUserDataWebviewToken` (default TTL
    /// [`USER_DATA_WEBVIEW_TOKEN_TTL_MS`]).
    pub fn sign_user_data_webview_token(
        &self,
        claims: &UserDataWebviewClaims,
        ttl_ms: i64,
    ) -> Result<String, EncryptionError> {
        self.sign_claims(claims, ttl_ms, USER_DATA_WEBVIEW_AAD)
    }

    /// `verifyUserDataWebviewToken`.
    pub fn verify_user_data_webview_token(
        &self,
        token: &str,
    ) -> Result<TokenPayload<UserDataWebviewClaims>, EncryptionError> {
        self.verify_appointment_token(token, USER_DATA_WEBVIEW_AAD)
    }
}

/// `serde_json::Value` payloads (raw `signAppointmentToken` objects) have no
/// extra rules.
impl TokenClaims for serde_json::Value {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_node() {
        assert_eq!(base64url_encode(b"hello"), "aGVsbG8");
        assert_eq!(base64url_encode(b"\xfb\xff"), "-_8");
        assert_eq!(base64url_decode_lenient("aGVsbG8"), b"hello");
        assert_eq!(base64url_decode_lenient("aGVsbG8="), b"hello");
        assert_eq!(base64url_decode_lenient("-_8"), b"\xfb\xff");
    }

    #[test]
    fn iso_datetime_check_matches_zod() {
        assert!(is_iso_datetime("2026-07-22T08:30:00.000Z"));
        assert!(is_iso_datetime("2026-07-22T08:30Z"));
        assert!(!is_iso_datetime("2026-07-22T08:30:00+07:00"));
        assert!(!is_iso_datetime("2026-02-30T08:30:00Z"));
    }
}
