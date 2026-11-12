//! Port of `repositories/response-sla/repository.ts` (P3.3). Row locks
//! (`FOR NO KEY UPDATE`, `FOR SHARE SKIP LOCKED`) are not needed under the one writer.

use rusqlite::Connection;

use super::models::ResponseSlaPolicy;
use super::{Result, id_list, new_id, now_ms};

/// `ResponseSlaPolicyInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseSlaPolicyInput {
    pub enabled: bool,
    pub first_response_target_seconds: i64,
    pub next_response_target_seconds: Option<i64>,
    pub breaching_threshold_percent: i64,
}

/// `ResponseSlaWindowState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowState {
    pub id: i64,
    pub source_id: Option<String>,
    pub awaiting_human_reply_since: Option<i64>,
    pub assigned_user_id: Option<i64>,
    pub assigned_user_at: Option<i64>,
    pub admin_replied_at: Option<i64>,
    pub response_due_at: Option<i64>,
    pub response_sla_target_seconds: Option<i64>,
}

/// The snapshot [`set_snapshot_if_window_unchanged`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlaSnapshot {
    pub response_due_at: i64,
    pub response_sla_breaching_at: i64,
    pub response_sla_target_seconds: i64,
}

/// `findPolicy`.
pub fn find_policy(conn: &Connection, workspace_id: i64) -> Result<Option<ResponseSlaPolicy>> {
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `upsertPolicy`.
pub fn upsert_policy(
    conn: &Connection,
    workspace_id: i64,
    data: ResponseSlaPolicyInput,
) -> Result<ResponseSlaPolicy> {
    let now = now_ms();
    super::returning::<ResponseSlaPolicy>(
        conn,
        "INSERT INTO \"ResponseSlaPolicy\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"enabled\", \
         \"firstResponseTargetSeconds\", \"nextResponseTargetSeconds\", \"breachingThresholdPercent\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (\"workspaceId\") DO UPDATE SET \
         \"enabled\" = excluded.\"enabled\", \"firstResponseTargetSeconds\" = excluded.\"firstResponseTargetSeconds\", \
         \"nextResponseTargetSeconds\" = excluded.\"nextResponseTargetSeconds\", \
         \"breachingThresholdPercent\" = excluded.\"breachingThresholdPercent\", \"updatedAt\" = excluded.\"updatedAt\"",
        &[
            &new_id(),
            &now,
            &workspace_id,
            &data.enabled,
            &data.first_response_target_seconds,
            &data.next_response_target_seconds,
            &data.breaching_threshold_percent,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to upsert response SLA policy"))
}

/// `deletePolicy`: `true` when a policy was deleted.
pub fn delete_policy(conn: &Connection, workspace_id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"ResponseSlaPolicy\" WHERE \"workspaceId\" = ?1",
        [workspace_id],
    )? > 0)
}

/// `readWindowStates`: by id (`lock` has no effect under the single writer).
pub fn read_window_states(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    lock: bool,
) -> Result<Vec<WindowState>> {
    let _ = lock;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"sourceId\", \"awaitingHumanReplySince\", \"assignedUserId\", \"assignedUserAt\", \
         \"adminRepliedAt\", \"responseDueAt\", \"responseSlaTargetSeconds\" FROM \"Conversation\" \
         WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(ids)], |r| {
            Ok(WindowState {
                id: r.get(0)?,
                source_id: r.get(1)?,
                awaiting_human_reply_since: r.get(2)?,
                assigned_user_id: r.get(3)?,
                assigned_user_at: r.get(4)?,
                admin_replied_at: r.get(5)?,
                response_due_at: r.get(6)?,
                response_sla_target_seconds: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `areWindowsAlreadyOpen`: every id is a thread (sourceId) or already awaits a reply.
pub fn are_windows_already_open(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<bool> {
    if ids.is_empty() {
        return Ok(false);
    }
    let settled: i64 = conn.query_row(
        "SELECT count(*) FROM \"Conversation\" WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) \
         AND (\"sourceId\" IS NOT NULL OR \"awaitingHumanReplySince\" IS NOT NULL)",
        rusqlite::params![workspace_id, id_list(ids)],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(settled).ok() == Some(ids.len()))
}

/// `readAssigneeIds`: the distinct assignees (NULL dropped).
pub fn read_assignee_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT \"assignedUserId\" FROM \"Conversation\" WHERE \"workspaceId\" = ?1 \
         AND \"id\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(ids)], |r| {
            r.get::<_, Option<i64>>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().flatten().collect())
}

/// `setSnapshotIfWindowUnchanged`: writes the snapshot when the policy is enabled and
/// the DM's window still started at `expected_since`.
pub fn set_snapshot_if_window_unchanged(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    expected_since: i64,
    snapshot: SlaSnapshot,
) -> Result<bool> {
    let enabled = find_policy(conn, workspace_id)?.is_some_and(|p| p.enabled);
    if !enabled {
        return Ok(false);
    }
    let changed = conn.execute(
        "UPDATE \"Conversation\" SET \"responseDueAt\" = ?4, \"responseSlaBreachingAt\" = ?5, \
         \"responseSlaTargetSeconds\" = ?6, \"updatedAt\" = ?7 \
         WHERE \"workspaceId\" = ?1 AND \"id\" = ?2 AND \"awaitingHumanReplySince\" = ?3 AND \"sourceId\" IS NULL",
        rusqlite::params![
            workspace_id,
            id,
            expected_since,
            snapshot.response_due_at,
            snapshot.response_sla_breaching_at,
            snapshot.response_sla_target_seconds,
            now_ms()
        ],
    )?;
    Ok(changed > 0)
}

/// `clearSnapshotIfWindowUnchanged`.
pub fn clear_snapshot_if_window_unchanged(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    expected_since: i64,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE \"Conversation\" SET \"responseDueAt\" = NULL, \"responseSlaBreachingAt\" = NULL, \
         \"responseSlaTargetSeconds\" = NULL, \"updatedAt\" = ?4 \
         WHERE \"workspaceId\" = ?1 AND \"id\" = ?2 AND \"awaitingHumanReplySince\" = ?3",
        rusqlite::params![workspace_id, id, expected_since, now_ms()],
    )?;
    Ok(changed > 0)
}

/// `clearWorkspaceSnapshots`.
pub fn clear_workspace_snapshots(conn: &Connection, workspace_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"Conversation\" SET \"responseDueAt\" = NULL, \"responseSlaBreachingAt\" = NULL, \
         \"responseSlaTargetSeconds\" = NULL, \"updatedAt\" = ?2 WHERE \"workspaceId\" = ?1 AND \"responseDueAt\" IS NOT NULL",
        rusqlite::params![workspace_id, now_ms()],
    )?;
    Ok(())
}
