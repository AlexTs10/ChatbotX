//! Port of `repositories/leaderboard/history.ts` (P3.3): `leaderboardScoreEventRepository`
//! and `leaderboardAuditLogRepository`.

use rusqlite::Connection;

use super::models::LeaderboardAuditLog;
use super::utils::{PaginationInput, get_pagination_with_defaults};
use super::{Result, new_id, now_ms};

/// One row of [`list_events_for_contact`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreEventRow {
    pub id: i64,
    pub created_at: i64,
    pub delta: i64,
    pub source: String,
    pub rule: Option<String>,
    pub source_ref: Option<String>,
}

/// `leaderboardScoreEventRepository.listForContact`: `from <= createdAt < to`, newest first.
pub fn list_events_for_contact(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
    from: i64,
    to: i64,
    limit: i64,
) -> Result<Vec<ScoreEventRow>> {
    todo!("P3.3")
}

/// `leaderboardScoreEventRepository.deleteBatchByWorkspace`: deletes up to `limit` events.
pub fn delete_events_batch_by_workspace(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// Values of [`insert_audit_log`].
#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogInput {
    pub leaderboard_id: i64,
    pub workspace_id: i64,
    pub contact_id: Option<i64>,
    pub action: String,
    pub delta: i64,
    pub actor_user_id: Option<i64>,
    pub reason: String,
}

/// `leaderboardAuditLogRepository.insert`.
pub fn insert_audit_log(conn: &Connection, v: &AuditLogInput) -> Result<LeaderboardAuditLog> {
    todo!("P3.3")
}

/// One audit log row `with: { contact, actor }`.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditLogWith {
    pub log: LeaderboardAuditLog,
    /// `(id, fullName, avatar)`.
    pub contact: Option<(i64, Option<String>, Option<String>)>,
    /// `(id, name, email)`.
    pub actor: Option<(i64, Option<String>, String)>,
}

/// `leaderboardAuditLogRepository.listPaginated`: newest first, with the total.
pub fn list_audit_logs(
    conn: &Connection,
    leaderboard_id: i64,
    workspace_id: i64,
    page: PaginationInput,
) -> Result<(Vec<AuditLogWith>, i64)> {
    todo!("P3.3")
}
