//! Port of `repositories/integration-whatsapp/signup-session.ts`
//! (`whatsappSignupSessionRepository`, P3.2b). The Postgres `text[]` columns are JSON
//! arrays of strings.

use rusqlite::Connection;

use super::Result;
use super::models::WhatsappSignupSession;

/// `WHATSAPP_SIGNUP_SESSION_TTL_MS`.
pub const WHATSAPP_SIGNUP_SESSION_TTL_MS: i64 = 10 * 60 * 1000;

/// `SIGNUP_SESSION_PURGE_BATCH_SIZE`.
pub const SIGNUP_SESSION_PURGE_BATCH_SIZE: i64 = 500;

/// `CreateWhatsappSignupSessionInput` (`now` = unix ms).
#[derive(Debug, Clone, PartialEq)]
pub struct CreateInput {
    pub user_id: i64,
    pub owner_id: i64,
    pub workspace_id: Option<i64>,
    pub waba_id: String,
    pub business_id: String,
    pub encrypted_access_token: serde_json::Value,
    pub api_version: String,
    pub candidate_phone_number_ids: Vec<String>,
    pub now: i64,
}

/// `ClaimSignupSessionPhoneNumberInput` (`now` = unix ms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInput {
    pub id: i64,
    pub user_id: i64,
    pub owner_id: i64,
    pub phone_number_id: String,
    pub now: i64,
}

/// `createSignupSession`: expires `WHATSAPP_SIGNUP_SESSION_TTL_MS` after `now`.
pub fn create_signup_session(
    _conn: &Connection,
    _input: &CreateInput,
) -> Result<WhatsappSignupSession> {
    todo!("P3.2b")
}

/// `findActiveSignupSessionForUser`: by id and user (never owner), not expired at `now`.
pub fn find_active_signup_session_for_user(
    _conn: &Connection,
    _id: i64,
    _user_id: i64,
    _now: i64,
) -> Result<Option<WhatsappSignupSession>> {
    todo!("P3.2b")
}

/// `claimSignupSessionPhoneNumber`: appends a candidate, unclaimed phone number; the
/// session is consumed once every candidate is claimed.
pub fn claim_signup_session_phone_number(
    _conn: &Connection,
    _input: &ClaimInput,
) -> Result<Option<WhatsappSignupSession>> {
    todo!("P3.2b")
}

/// `bindSignupSessionWorkspace`: only while `workspaceId` is NULL.
pub fn bind_signup_session_workspace(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
) -> Result<()> {
    todo!("P3.2b")
}

/// `purgeFinishedSignupSessions`: deletes up to `batch_size` consumed or expired sessions.
pub fn purge_finished_signup_sessions(
    _conn: &Connection,
    _now: i64,
    _batch_size: i64,
) -> Result<usize> {
    todo!("P3.2b")
}
