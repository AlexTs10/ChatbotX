//! Port of `repositories/inbox/repository.ts`. `findByIdForUpdate` needs no row lock
//! (one SQLite writer); callers run it inside the write transaction.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::Inbox;
use super::partials::thread_control::THREAD_CONTROL_SEEN_REFRESH_MS;
use super::patch::{Patch, set_clause};
use super::sql::Sql;
use super::{Error, Model, Result, id_list, now_ms, text_list};

/// `touchThreadControlSeen`: writes `threadControlSeenAt` only when missing or older than
/// the refresh interval (workspace-scoped). `true` when written.
pub fn touch_thread_control_seen(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    seen_at: i64,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE \"Inbox\" SET \"threadControlSeenAt\" = ?3 WHERE \"id\" = ?2 AND \"workspaceId\" = ?1 \
         AND (\"threadControlSeenAt\" IS NULL OR \"threadControlSeenAt\" < ?4)",
        rusqlite::params![workspace_id, inbox_id, seen_at, seen_at - THREAD_CONTROL_SEEN_REFRESH_MS],
    )?;
    Ok(n > 0)
}

/// `InboxIdentity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxIdentity {
    pub id: i64,
    pub name: String,
    pub channel: String,
    pub status: String,
}

/// `InboxIdentity & { workspaceId }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceInboxIdentity {
    pub identity: InboxIdentity,
    pub workspace_id: i64,
}

/// `InboxAnalyticsOption`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxAnalyticsOption {
    pub id: i64,
    pub name: String,
    pub channel: String,
}

fn identity(r: &rusqlite::Row<'_>) -> rusqlite::Result<InboxIdentity> {
    Ok(InboxIdentity {
        id: r.get(0)?,
        name: r.get(1)?,
        channel: r.get(2)?,
        status: r.get(3)?,
    })
}

/// `listIdentitiesByIds`: the workspace's inboxes of `ids` (by id).
pub fn list_identities_by_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<InboxIdentity>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\", \"channel\", \"status\" FROM \"Inbox\" WHERE \"workspaceId\" = ?1 \
         AND \"id\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(ids)], identity)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listConnectedIdentitiesByChannels`: connected inboxes of the workspaces and channels,
/// by name.
pub fn list_connected_identities_by_channels(
    conn: &Connection,
    workspace_ids: &[i64],
    channels: &[&str],
) -> Result<Vec<WorkspaceInboxIdentity>> {
    if channels.is_empty() || workspace_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\", \"channel\", \"status\", \"workspaceId\" FROM \"Inbox\" \
         WHERE \"workspaceId\" IN (SELECT value FROM json_each(?1)) AND \"status\" = 'connected' \
         AND \"channel\" IN (SELECT value FROM json_each(?2)) ORDER BY \"name\" ASC",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![id_list(workspace_ids), text_list(channels)],
            |r| {
                Ok(WorkspaceInboxIdentity {
                    identity: identity(r)?,
                    workspace_id: r.get(4)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `findById` (not workspace-scoped, as the TS).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<Inbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Inbox\" WHERE \"id\" = ",
        Inbox::columns_sql(None)
    ));
    q.push_bind(id);
    q.model(conn)
}

/// `findByIdForUpdate`: the workspace's inbox identity (call inside the write job).
pub fn find_by_id_for_update(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<InboxIdentity>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\", \"channel\", \"status\" FROM \"Inbox\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
    )?;
    let mut rows = stmt.query_map(rusqlite::params![id, workspace_id], identity)?;
    Ok(rows.next().transpose()?)
}

/// `listOptionsByWorkspaceAndChannel`: `(id, name)` of the workspace's inboxes of the channel.
pub fn list_options_by_workspace_and_channel(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\" FROM \"Inbox\" WHERE \"workspaceId\" = ?1 AND \"channel\" = ?2 ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, channel], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listAnalyticsOptionsByWorkspace`: connected, non-SMTP inboxes by name.
pub fn list_analytics_options_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<InboxAnalyticsOption>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\", \"channel\" FROM \"Inbox\" WHERE \"workspaceId\" = ?1 \
         AND \"status\" = 'connected' AND \"channel\" <> 'smtp' ORDER BY \"name\" ASC",
    )?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            Ok(InboxAnalyticsOption {
                id: r.get(0)?,
                name: r.get(1)?,
                channel: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listTenantSuspendedWithoutConnectionByOwner`: inboxes of the owner's workspaces that
/// are `disconnected` for `tenant_suspended` and have no `Connection` row.
pub fn list_tenant_suspended_without_connection_by_owner(
    conn: &Connection,
    owner_id: i64,
) -> Result<Vec<Inbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Inbox\" AS i JOIN \"Workspace\" AS w ON w.\"id\" = i.\"workspaceId\" \
         LEFT JOIN \"Connection\" AS c ON c.\"inboxId\" = i.\"id\" WHERE w.\"ownerId\" = ",
        Inbox::columns_sql(Some("i"))
    ));
    q.push_bind(owner_id);
    q.push(
        " AND i.\"status\" = 'disconnected' AND i.\"disconnectReason\" = 'tenant_suspended' AND c.\"id\" IS NULL ORDER BY i.\"id\"",
    );
    q.models(conn)
}

/// The columns `updateConnectionMirror` may set.
const CONNECTION_MIRROR_COLUMNS: &[&str] = &["status", "disconnectedAt", "disconnectReason"];

/// `updateConnectionMirror`: sets `status` / `disconnectedAt` / `disconnectReason` (the
/// patch keys; others are refused) on the workspace's inbox; `updatedAt` = now.
pub fn update_connection_mirror(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    values: &Patch,
) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    if let Some((col, _)) = values
        .iter()
        .find(|(c, _)| !CONNECTION_MIRROR_COLUMNS.contains(c))
    {
        return Err(Error::msg(format!(
            "updateConnectionMirror cannot set {col}"
        )));
    }
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"Inbox\" SET ");
    q.append(set_clause(Inbox::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(inbox_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    conn.execute(&q.text, rusqlite::params_from_iter(q.params.iter()))?;
    Ok(())
}
