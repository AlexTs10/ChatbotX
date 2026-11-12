//! Port of `repositories/resource-deletion-log/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::{Result, new_id, now_ms};

/// `ResourceDeletionLogInsert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDeletionLogInsert {
    pub workspace_id: i64,
    /// `workspace` | `channel`.
    pub resource_type: String,
    pub resource_id: String,
    pub resource_subtype: Option<String>,
    pub resource_name: Option<String>,
    /// `pending` | `completed` | `canceled`.
    pub status: String,
    pub requested_by_user_id: Option<i64>,
    pub request_source: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub requested_at: i64,
    pub completed_at: Option<i64>,
    pub canceled_at: Option<i64>,
}

/// `ResourceDeletionLogResourceKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceKey {
    pub workspace_id: i64,
    pub resource_type: String,
    pub resource_id: String,
}

/// `insert`.
pub fn insert(conn: &Connection, row: &ResourceDeletionLogInsert) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"ResourceDeletionLog\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"resourceType\", \
         \"resourceId\", \"resourceSubtype\", \"resourceName\", \"status\", \"requestedByUserId\", \"requestSource\", \
         \"ipAddress\", \"userAgent\", \"requestedAt\", \"completedAt\", \"canceledAt\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            new_id(),
            now,
            row.workspace_id,
            row.resource_type,
            row.resource_id,
            row.resource_subtype,
            row.resource_name,
            row.status,
            row.requested_by_user_id,
            row.request_source,
            row.ip_address,
            row.user_agent,
            row.requested_at,
            row.completed_at,
            row.canceled_at
        ],
    )?;
    Ok(())
}

/// `findPendingIdByResource`: the newest `pending` row of a resource.
pub fn find_pending_id_by_resource(conn: &Connection, key: &ResourceKey) -> Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"ResourceDeletionLog\" WHERE \"workspaceId\" = ?1 AND \"resourceType\" = ?2 \
         AND \"resourceId\" = ?3 AND \"status\" = 'pending' ORDER BY \"id\" DESC LIMIT 1",
    )?;
    Ok(stmt
        .query_row(
            rusqlite::params![key.workspace_id, key.resource_type, key.resource_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// `completePendingById`: `pending` -> `completed` for one still-pending row; returns the
/// updated count.
pub fn complete_pending_by_id(conn: &Connection, id: i64, completed_at: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE \"ResourceDeletionLog\" SET \"status\" = 'completed', \"completedAt\" = ?2, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"status\" = 'pending'",
        rusqlite::params![id, completed_at, now_ms()],
    )?)
}

/// `cancelPendingByResource`: `pending` -> `canceled` for every pending row of a
/// resource of the workspace; returns the updated count.
pub fn cancel_pending_by_resource(
    conn: &Connection,
    key: &ResourceKey,
    canceled_at: i64,
) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE \"ResourceDeletionLog\" SET \"status\" = 'canceled', \"canceledAt\" = ?4, \"updatedAt\" = ?5 \
         WHERE \"workspaceId\" = ?1 AND \"resourceType\" = ?2 AND \"resourceId\" = ?3 AND \"status\" = 'pending'",
        rusqlite::params![key.workspace_id, key.resource_type, key.resource_id, canceled_at, now_ms()],
    )?)
}
