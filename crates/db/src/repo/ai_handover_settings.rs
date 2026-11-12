//! Port of `repositories/ai-handover-settings/repository.ts`.
//!
//! `FOR UPDATE` row locks do not exist in SQLite: every write runs inside the one
//! `BEGIN IMMEDIATE` writer, which already serializes the "lock, read, write" sequences.

use rusqlite::Connection;

use super::models::AiHandoverSettings;
use super::{Error, Json, Result, new_id, now_ms};

/// `UpsertAiHandoverSettingsInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertAiHandoverSettingsInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub channel: String,
    pub enabled: bool,
    pub schedule_enabled: bool,
    pub time_ranges: Json,
    pub goto_flow_id: Option<i64>,
    pub return_message: Option<String>,
    pub pause_bot_waiting_for_staff: bool,
}

/// `SetApplyToAllInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetApplyToAllInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub apply_to_all_customers: bool,
    pub apply_to_all_message: Option<String>,
    pub requested_by_user_id: Option<i64>,
}

/// `AiHandoverSettingsWithInbox`.
#[derive(Debug, Clone, PartialEq)]
pub struct AiHandoverSettingsWithInbox {
    pub settings: AiHandoverSettings,
    pub inbox_status: String,
}

/// `findByInbox`: the Page's row, scoped by workspace AND inbox.
pub fn find_by_inbox(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<AiHandoverSettings>> {
    todo!("P3.2");
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"inboxId\" = ?2",
        &[&workspace_id, &inbox_id],
    )
}

/// `upsert`: inserts every setting with the apply-to-all state off; on an existing row of
/// the inbox it replaces only the settings.
pub fn upsert(conn: &Connection, input: &UpsertAiHandoverSettingsInput) -> Result<AiHandoverSettings> {
    todo!("P3.2");
    let now = now_ms();
    let rows = super::returning::<AiHandoverSettings>(
        conn,
        "INSERT INTO \"AIHandoverSettings\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"inboxId\", \"channel\", \
         \"enabled\", \"scheduleEnabled\", \"timeRanges\", \"gotoFlowId\", \"returnMessage\", \"pauseBotWaitingForStaff\", \
         \"applyToAllCustomers\", \"applyToAllRevision\", \"applyToAllMessage\", \"applyToAllRequestedByUserId\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0, NULL, NULL) \
         ON CONFLICT (\"inboxId\") DO UPDATE SET \"enabled\" = excluded.\"enabled\", \"scheduleEnabled\" = excluded.\"scheduleEnabled\", \
         \"timeRanges\" = excluded.\"timeRanges\", \"gotoFlowId\" = excluded.\"gotoFlowId\", \"returnMessage\" = excluded.\"returnMessage\", \
         \"pauseBotWaitingForStaff\" = excluded.\"pauseBotWaitingForStaff\", \"updatedAt\" = excluded.\"updatedAt\"",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.inbox_id,
            &input.channel,
            &input.enabled,
            &input.schedule_enabled,
            &input.time_ranges,
            &input.goto_flow_id,
            &input.return_message,
            &input.pause_bot_waiting_for_staff,
        ],
    )?;
    rows.into_iter()
        .next()
        .ok_or_else(|| Error::msg("Failed to upsert business AI settings"))
}

/// `lockExisting`: the settings row and the Page's inbox status (`disconnected` when the
/// inbox row is gone); `None` without a settings row (no status read, no insert).
pub fn lock_existing(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<AiHandoverSettingsWithInbox>> {
    todo!("P3.2");
    let Some(settings) = super::find_one::<AiHandoverSettings>(
        conn,
        "\"workspaceId\" = ?1 AND \"inboxId\" = ?2",
        &[&workspace_id, &inbox_id],
    )?
    else {
        return Ok(None);
    };
    let mut stmt = conn.prepare_cached("SELECT \"status\" FROM \"Inbox\" WHERE \"id\" = ?1 LIMIT 1")?;
    let mut rows = stmt.query([inbox_id])?;
    let inbox_status = match rows.next()? {
        Some(row) => row.get(0)?,
        None => "disconnected".to_owned(),
    };
    Ok(Some(AiHandoverSettingsWithInbox {
        settings,
        inbox_status,
    }))
}

/// `lockForApplyToAll`: creates a switched-off row when the Page has none, then
/// [`lock_existing`]. Fails with `Failed to lock business AI settings` when the row cannot
/// be read back (the inbox's row belongs to another workspace).
pub fn lock_for_apply_to_all(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    channel: &str,
) -> Result<AiHandoverSettingsWithInbox> {
    todo!("P3.2");
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"AIHandoverSettings\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"inboxId\", \"channel\", \
         \"enabled\", \"scheduleEnabled\", \"timeRanges\", \"gotoFlowId\", \"returnMessage\", \"pauseBotWaitingForStaff\", \
         \"applyToAllCustomers\", \"applyToAllRevision\", \"applyToAllMessage\", \"applyToAllRequestedByUserId\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, 0, 0, '[]', NULL, NULL, 0, 0, 0, NULL, NULL) ON CONFLICT (\"inboxId\") DO NOTHING",
        rusqlite::params![new_id(), now, workspace_id, inbox_id, channel],
    )?;
    lock_existing(conn, workspace_id, inbox_id)?
        .ok_or_else(|| Error::msg("Failed to lock business AI settings"))
}

/// `setApplyToAll`: records the new apply-to-all state with the next revision. Fails with
/// `Failed to record the apply-to-all change` when the Page has no row.
pub fn set_apply_to_all(conn: &Connection, input: &SetApplyToAllInput) -> Result<AiHandoverSettings> {
    todo!("P3.2");
    let current = find_by_inbox(conn, input.workspace_id, input.inbox_id)?;
    let revision = current.map_or(0, |c| c.apply_to_all_revision) + 1;
    let rows = super::returning::<AiHandoverSettings>(
        conn,
        "UPDATE \"AIHandoverSettings\" SET \"applyToAllCustomers\" = ?1, \"applyToAllRevision\" = ?2, \
         \"applyToAllMessage\" = ?3, \"applyToAllRequestedByUserId\" = ?4, \"updatedAt\" = ?7 \
         WHERE \"workspaceId\" = ?5 AND \"inboxId\" = ?6",
        &[
            &input.apply_to_all_customers,
            &revision,
            &input.apply_to_all_message,
            &input.requested_by_user_id,
            &input.workspace_id,
            &input.inbox_id,
            &now_ms(),
        ],
    )?;
    rows.into_iter()
        .next()
        .ok_or_else(|| Error::msg("Failed to record the apply-to-all change"))
}
