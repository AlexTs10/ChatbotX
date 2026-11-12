//! Port of `repositories/invitation/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::Invitation;
use super::{Json, Result};

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
    let _ = (conn, code);
    todo!("P3.3")
}

/// `create`: inserts and returns the row.
pub fn create(conn: &Connection, values: &NewInvitation) -> Result<Invitation> {
    let _ = (conn, values);
    todo!("P3.3")
}
