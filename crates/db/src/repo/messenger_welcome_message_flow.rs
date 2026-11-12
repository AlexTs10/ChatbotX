//! Port of `repositories/messenger-welcome-message-flow/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::MessengerWelcomeMessageFlow;
use super::{Result, id_list, new_id, now_ms};

/// `CurrentRowInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentRowInput {
    pub workspace_id: i64,
    pub flow_id: i64,
    pub integration_messenger_id: i64,
    pub page_id: String,
    pub meta_flow_id: String,
    pub flow_version_id: i64,
}

/// The columns `listCurrentByFlow` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentPageFlow {
    pub integration_messenger_id: i64,
    pub page_id: String,
    pub meta_flow_id: String,
}

/// What [`replace_current`] does with the previous row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousAction {
    /// Still used in an ad: mark it superseded.
    Supersede,
    /// Gone from Meta: delete it.
    Delete,
}

/// `findCurrent`: the current row of a flow on one page.
pub fn find_current(
    conn: &Connection,
    flow_id: i64,
    integration_messenger_id: i64,
) -> Result<Option<MessengerWelcomeMessageFlow>> {
    super::find_one(
        conn,
        "\"flowId\" = ?1 AND \"integrationMessengerId\" = ?2 AND \"status\" = 'current'",
        &[&flow_id, &integration_messenger_id],
    )
}

/// `listCurrentByFlow`: oldest first.
pub fn list_current_by_flow(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<CurrentPageFlow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"integrationMessengerId\", \"pageId\", \"metaFlowId\" FROM \"MessengerWelcomeMessageFlow\" \
         WHERE \"workspaceId\" = ?1 AND \"flowId\" = ?2 AND \"status\" = 'current' ORDER BY \"createdAt\" ASC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, flow_id], |row| {
            Ok(CurrentPageFlow {
                integration_messenger_id: row.get(0)?,
                page_id: row.get(1)?,
                meta_flow_id: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listAllByFlow`: current and superseded rows of a flow.
pub fn list_all_by_flow(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<MessengerWelcomeMessageFlow>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"flowId\" = ?2",
        &[&workspace_id, &flow_id],
    )
}

/// `insertCurrent`.
pub fn insert_current(conn: &Connection, input: &CurrentRowInput) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"MessengerWelcomeMessageFlow\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"flowId\", \
         \"integrationMessengerId\", \"pageId\", \"metaFlowId\", \"flowVersionId\", \"status\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'current')",
        rusqlite::params![
            new_id(),
            now,
            input.workspace_id,
            input.flow_id,
            input.integration_messenger_id,
            input.page_id,
            input.meta_flow_id,
            input.flow_version_id
        ],
    )?;
    Ok(())
}

/// `setFlowVersion`.
pub fn set_flow_version(conn: &Connection, id: i64, flow_version_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"MessengerWelcomeMessageFlow\" SET \"flowVersionId\" = ?2, \"updatedAt\" = ?3 WHERE \"id\" = ?1",
        rusqlite::params![id, flow_version_id, now_ms()],
    )?;
    Ok(())
}

/// `markSuperseded`.
pub fn mark_superseded(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"MessengerWelcomeMessageFlow\" SET \"status\" = 'superseded', \"updatedAt\" = ?2 WHERE \"id\" = ?1",
        rusqlite::params![id, now_ms()],
    )?;
    Ok(())
}

/// `replaceCurrent`: supersedes or deletes the previous row, then inserts the new current
/// row. Run it inside one write job (the TS nested transaction) so a page is never left
/// without a current row.
pub fn replace_current(
    conn: &Connection,
    previous_id: i64,
    action: PreviousAction,
    next: &CurrentRowInput,
) -> Result<()> {
    match action {
        PreviousAction::Supersede => mark_superseded(conn, previous_id)?,
        PreviousAction::Delete => delete_by_ids(conn, &[previous_id])?,
    }
    insert_current(conn, next)
}

/// `deleteByIds` (no statement for an empty list).
pub fn delete_by_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"MessengerWelcomeMessageFlow\" WHERE \"id\" IN (SELECT value FROM json_each(?1))",
        [id_list(ids)],
    )?;
    Ok(())
}
