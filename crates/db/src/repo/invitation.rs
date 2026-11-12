//! Port of `repositories/invitation/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::Invitation;
use super::{Error, Json, Result, new_id, now_ms};

/// `InvitationInsert` (the columns without a default).
#[derive(Debug, Clone, PartialEq)]
pub struct NewInvitation {
    pub code: String,
    pub permissions: Json,
    pub expires_at: i64,
    pub workspace_id: Option<i64>,
    pub invited_by: i64,
}

/// `findByCode`.
pub fn find_by_code(conn: &Connection, code: &str) -> Result<Option<Invitation>> {
    super::find_one(conn, "\"code\" = ?1", &[&code])
}

/// `create`: inserts and returns the row.
pub fn create(conn: &Connection, values: &NewInvitation) -> Result<Invitation> {
    let now = now_ms();
    super::returning::<Invitation>(
        conn,
        "INSERT INTO \"Invitation\" (\"id\", \"createdAt\", \"updatedAt\", \"code\", \"permissions\", \
         \"expiresAt\", \"workspaceId\", \"invitedBy\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)",
        &[
            &new_id(),
            &now,
            &values.code,
            &values.permissions,
            &values.expires_at,
            &values.workspace_id,
            &values.invited_by,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to create invitation"))
}
