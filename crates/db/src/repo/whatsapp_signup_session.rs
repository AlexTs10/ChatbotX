//! Port of `repositories/integration-whatsapp/signup-session.ts`
//! (`whatsappSignupSessionRepository`, P3.2b). The Postgres `text[]` columns are JSON
//! arrays of strings.

use rusqlite::Connection;

use super::models::WhatsappSignupSession;
use super::{Error, Result, json_text, new_id, now_ms};

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

const TABLE: &str = "\"WhatsappSignupSession\"";

fn first(rows: Vec<WhatsappSignupSession>) -> Option<WhatsappSignupSession> {
    rows.into_iter().next()
}

/// `createSignupSession`: expires `WHATSAPP_SIGNUP_SESSION_TTL_MS` after `now`.
pub fn create_signup_session(
    conn: &Connection,
    input: &CreateInput,
) -> Result<WhatsappSignupSession> {
    first(super::returning(
        conn,
        &format!(
            "INSERT INTO {TABLE} (\"id\", \"userId\", \"ownerId\", \"workspaceId\", \"wabaId\", \
             \"businessId\", \"encryptedAccessToken\", \"apiVersion\", \"candidatePhoneNumberIds\", \
             \"expiresAt\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        &[
            &new_id(),
            &input.user_id,
            &input.owner_id,
            &input.workspace_id,
            &input.waba_id,
            &input.business_id,
            &json_text(&input.encrypted_access_token)?,
            &input.api_version,
            &json_text(&input.candidate_phone_number_ids)?,
            &(input.now + WHATSAPP_SIGNUP_SESSION_TTL_MS),
        ],
    )?)
    .ok_or_else(|| Error::msg("Failed to create WhatsApp signup session"))
}

/// `findActiveSignupSessionForUser`: by id and user (never owner), not expired at `now`.
pub fn find_active_signup_session_for_user(
    conn: &Connection,
    id: i64,
    user_id: i64,
    now: i64,
) -> Result<Option<WhatsappSignupSession>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"userId\" = ?2 AND \"expiresAt\" > ?3",
        &[&id, &user_id, &now],
    )
}

/// `claimSignupSessionPhoneNumber`: appends a candidate, unclaimed phone number; the
/// session is consumed once every candidate is claimed.
pub fn claim_signup_session_phone_number(
    conn: &Connection,
    input: &ClaimInput,
) -> Result<Option<WhatsappSignupSession>> {
    let contains = |column: &str| {
        format!("EXISTS (SELECT 1 FROM json_each({TABLE}.\"{column}\") WHERE value = ?4)")
    };
    Ok(first(super::returning(
        conn,
        &format!(
            "UPDATE {TABLE} SET \"claimedPhoneNumberIds\" = json_insert(\"claimedPhoneNumberIds\", \
             '$[#]', ?4), \"consumedAt\" = CASE WHEN json_array_length(\"claimedPhoneNumberIds\") + 1 \
             = json_array_length(\"candidatePhoneNumberIds\") THEN ?6 ELSE \"consumedAt\" END, \
             \"updatedAt\" = ?6 WHERE \"id\" = ?1 AND \"userId\" = ?2 AND \"ownerId\" = ?3 \
             AND \"expiresAt\" > ?5 AND {} AND NOT {}",
            contains("candidatePhoneNumberIds"),
            contains("claimedPhoneNumberIds"),
        ),
        &[
            &input.id,
            &input.user_id,
            &input.owner_id,
            &input.phone_number_id,
            &input.now,
            &now_ms(),
        ],
    )?))
}

/// `bindSignupSessionWorkspace`: only while `workspaceId` is NULL.
pub fn bind_signup_session_workspace(conn: &Connection, id: i64, workspace_id: i64) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"workspaceId\" = ?1, \"updatedAt\" = ?2 WHERE \"id\" = ?3 \
             AND \"workspaceId\" IS NULL"
        ),
        [workspace_id, now_ms(), id],
    )?;
    Ok(())
}

/// `purgeFinishedSignupSessions`: deletes up to `batch_size` consumed or expired sessions.
pub fn purge_finished_signup_sessions(
    conn: &Connection,
    now: i64,
    batch_size: i64,
) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "DELETE FROM {TABLE} WHERE \"id\" IN (SELECT \"id\" FROM {TABLE} WHERE \
             \"consumedAt\" IS NOT NULL OR \"expiresAt\" <= ?1 LIMIT ?2)"
        ),
        [now, batch_size],
    )?)
}
