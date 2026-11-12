//! Port of `repositories/resource-deletion-log/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `findPendingIdByResource`: the newest `pending` row of a resource.
pub fn find_pending_id_by_resource(conn: &Connection, key: &ResourceKey) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `completePendingById`: `pending` -> `completed` for one still-pending row; returns the
/// updated count.
pub fn complete_pending_by_id(conn: &Connection, id: i64, completed_at: i64) -> Result<usize> {
    todo!("P3.3")
}

/// `cancelPendingByResource`: `pending` -> `canceled` for every pending row of a
/// resource of the workspace; returns the updated count.
pub fn cancel_pending_by_resource(
    conn: &Connection,
    key: &ResourceKey,
    canceled_at: i64,
) -> Result<usize> {
    todo!("P3.3")
}
