//! Port of `repositories/oauth-consent/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::{Json, Result, now_ms};

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
    let mut stmt = conn.prepare_cached(
        "SELECT \"activeTokens\".\"activeUntil\", c.\"icon\", oc.\"clientId\", c.\"name\", oc.\"createdAt\", \
                \"activeTokens\".\"lastRefreshedAt\", oc.\"scopes\" \
         FROM \"OAuthConsent\" oc \
         INNER JOIN \"OAuthClient\" c ON oc.\"clientId\" = c.\"clientId\" \
         LEFT JOIN (SELECT max(\"expiresAt\") AS \"activeUntil\", \"clientId\", max(\"createdAt\") AS \"lastRefreshedAt\" \
                    FROM \"OAuthRefreshToken\" \
                    WHERE \"userId\" = ?1 AND \"expiresAt\" > ?2 AND \"revoked\" IS NULL \
                    GROUP BY \"clientId\") AS \"activeTokens\" \
           ON oc.\"clientId\" = \"activeTokens\".\"clientId\" \
         WHERE oc.\"userId\" = ?1 \
         ORDER BY oc.\"createdAt\" DESC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![user_id, now], |row| {
            Ok(GivenConsent {
                active_until: row.get(0)?,
                client_icon: row.get(1)?,
                client_id: row.get(2)?,
                client_name: row.get(3)?,
                first_authorized_at: row.get(4)?,
                last_refreshed_at: row.get(5)?,
                scopes: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `findLatestGivenByUserAndClient`: `Some(workspaceId)` of the newest consent (rows with
/// a workspace first); `None` when there is no consent.
pub fn find_latest_given_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
) -> Result<Option<Option<i64>>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"workspaceId\" FROM \"OAuthConsent\" WHERE \"userId\" = ?1 AND \"clientId\" = ?2 \
         ORDER BY \"workspaceId\" IS NULL, \"createdAt\" DESC, \"id\" DESC LIMIT 1",
    )?;
    Ok(stmt
        .query_row(rusqlite::params![user_id, client_id], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .optional()?)
}

/// `setWorkspaceByUserAndClient`: returns the updated count.
pub fn set_workspace_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
    workspace_id: i64,
) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE \"OAuthConsent\" SET \"workspaceId\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"userId\" = ?1 AND \"clientId\" = ?2",
        rusqlite::params![user_id, client_id, workspace_id, now_ms()],
    )?)
}

/// `deleteByUserAndClient`: returns the deleted count.
pub fn delete_by_user_and_client(
    conn: &Connection,
    user_id: i64,
    client_id: &str,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"OAuthConsent\" WHERE \"userId\" = ?1 AND \"clientId\" = ?2",
        rusqlite::params![user_id, client_id],
    )?)
}
