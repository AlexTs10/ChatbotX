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
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"createdAt\", \"delta\", \"source\", \"rule\", \"sourceRef\" FROM \"LeaderboardScoreEvent\" \
         WHERE \"leaderboardId\" = ?1 AND \"contactId\" = ?2 AND \"createdAt\" >= ?3 AND \"createdAt\" < ?4 \
         ORDER BY \"createdAt\" DESC LIMIT ?5",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![leaderboard_id, contact_id, from, to, limit],
            |r| {
                Ok(ScoreEventRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    delta: r.get(2)?,
                    source: r.get(3)?,
                    rule: r.get(4)?,
                    source_ref: r.get(5)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `leaderboardScoreEventRepository.deleteBatchByWorkspace`: deletes up to `limit` events.
pub fn delete_events_batch_by_workspace(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"LeaderboardScoreEvent\" WHERE (\"createdAt\", \"id\") IN (SELECT \"createdAt\", \"id\" \
         FROM \"LeaderboardScoreEvent\" WHERE \"workspaceId\" = ?1 LIMIT ?2)",
        rusqlite::params![workspace_id, limit],
    )?)
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
    super::returning::<LeaderboardAuditLog>(
        conn,
        "INSERT INTO \"LeaderboardAuditLog\" (\"id\", \"createdAt\", \"leaderboardId\", \"workspaceId\", \"contactId\", \
         \"action\", \"delta\", \"actorUserId\", \"reason\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        &[
            &new_id(),
            &now_ms(),
            &v.leaderboard_id,
            &v.workspace_id,
            &v.contact_id,
            &v.action,
            &v.delta,
            &v.actor_user_id,
            &v.reason,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to insert leaderboard audit log"))
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
    use super::Model;
    let p = get_pagination_with_defaults(page);
    let n = LeaderboardAuditLog::COLUMNS.len();
    let sql = format!(
        "SELECT {}, c.\"id\", c.\"fullName\", c.\"avatar\", u.\"id\", u.\"name\", u.\"email\" FROM \"LeaderboardAuditLog\" l \
         LEFT JOIN \"Contact\" c ON c.\"id\" = l.\"contactId\" LEFT JOIN \"User\" u ON u.\"id\" = l.\"actorUserId\" \
         WHERE l.\"leaderboardId\" = ?1 AND l.\"workspaceId\" = ?2 ORDER BY l.\"createdAt\" DESC LIMIT ?3 OFFSET ?4",
        LeaderboardAuditLog::columns_sql(Some("l"))
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![leaderboard_id, workspace_id, p.limit, p.offset],
            |r| {
                let contact = match r.get::<_, Option<i64>>(n)? {
                    Some(id) => Some((id, r.get(n + 1)?, r.get(n + 2)?)),
                    None => None,
                };
                let actor = match r.get::<_, Option<i64>>(n + 3)? {
                    Some(id) => Some((id, r.get(n + 4)?, r.get(n + 5)?)),
                    None => None,
                };
                Ok(AuditLogWith {
                    log: LeaderboardAuditLog::from_row(r)?,
                    contact,
                    actor,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total = conn.query_row(
        "SELECT count(*) FROM \"LeaderboardAuditLog\" WHERE \"leaderboardId\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![leaderboard_id, workspace_id],
        |r| r.get(0),
    )?;
    Ok((rows, total))
}
