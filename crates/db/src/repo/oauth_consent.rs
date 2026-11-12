//! Port of `repositories/oauth-consent/repository.ts` (P3.3).

use rusqlite::Connection;

use super::{Json, Result};

/// One row of [`list_given_by_user`].
#[derive(Debug, Clone, PartialEq)]
pub struct GivenConsent {
    /// `max(OAuthRefreshToken.expiresAt)` of the live tokens of the client (`None` when
    /// there is no live token).
    pub active_until: Option<i64>,
    pub client_icon: Option<String>,
    pub client_id: String,
    pub client_name: Option<String>,
    pub first_authorized_at: i64,
    /// `max(OAuthRefreshToken.createdAt)` of the live tokens of the client.
    pub last_refreshed_at: Option<i64>,
    pub scopes: Json,
}

/// `listGivenByUser`: the user's consents with their client, newest first, joined with
/// the aggregate of the live (not expired at `now`, not revoked) refresh tokens.
pub fn list_given_by_user(conn: &Connection, user_id: i64, now: i64) -> Result<Vec<GivenConsent>> {
    let _ = (conn, user_id, now);
    todo!("P3.3")
}

/// `findLatestGivenByUserAndClient`: `Some(workspaceId)` of the newest consent (rows with
/// a workspace first); `None` when there is no consent.
pub fn find_latest_given_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
) -> Result<Option<Option<i64>>> {
    let _ = (conn, user_id, client_id);
    todo!("P3.3")
}

/// `setWorkspaceByUserAndClient`: returns the updated count.
pub fn set_workspace_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
    workspace_id: i64,
) -> Result<usize> {
    let _ = (conn, user_id, client_id, workspace_id);
    todo!("P3.3")
}

/// `deleteByUserAndClient`: returns the deleted count.
pub fn delete_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
) -> Result<usize> {
    let _ = (conn, user_id, client_id);
    todo!("P3.3")
}
