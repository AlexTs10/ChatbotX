//! Port of `repositories/messenger-welcome-message-flow/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `listCurrentByFlow`: oldest first.
pub fn list_current_by_flow(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<CurrentPageFlow>> {
    todo!("P3.3")
}

/// `listAllByFlow`: current and superseded rows of a flow.
pub fn list_all_by_flow(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<MessengerWelcomeMessageFlow>> {
    todo!("P3.3")
}

/// `insertCurrent`.
pub fn insert_current(conn: &Connection, input: &CurrentRowInput) -> Result<()> {
    todo!("P3.3")
}

/// `setFlowVersion`.
pub fn set_flow_version(conn: &Connection, id: i64, flow_version_id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `markSuperseded`.
pub fn mark_superseded(conn: &Connection, id: i64) -> Result<()> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `deleteByIds` (no statement for an empty list).
pub fn delete_by_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    todo!("P3.3")
}
