//! Port of `repositories/leaderboard/leaderboard.ts` (P3.3). Row locks (`FOR UPDATE`) are
//! plain reads under the one SQLite writer (P3.2 rule).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{File, Leaderboard};
use super::patch::{Patch, insert_clause, set_clause};
use super::sql::Sql;
use super::utils::{PaginationInput, get_pagination_with_defaults, like_contains};
use super::{Json, Model, Result, new_id, now_ms};

/// `LeaderboardListInput` (without the page fields for [`count`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeaderboardFilter {
    pub workspace_id: i64,
    pub status: Option<String>,
    pub platform: Option<String>,
    pub keyword: Option<String>,
}

/// `{ id, name, channel }` of the leaderboard inbox.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxRef {
    pub id: i64,
    pub name: String,
    pub channel: String,
}

/// `{ id, name, type }` of the leaderboard minigame.
#[derive(Debug, Clone, PartialEq)]
pub struct MinigameRef {
    pub id: i64,
    pub name: String,
    pub r#type: String,
}

/// A leaderboard `with: { inbox, minigame }`.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderboardWith {
    pub leaderboard: Leaderboard,
    pub inbox: Option<InboxRef>,
    pub minigame: Option<MinigameRef>,
}

/// Result of [`claim_rewards`].
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedRewards {
    pub recipients: serde_json::Value,
    pub previously_triggered: bool,
}

/// One row of [`find_all_live`].
#[derive(Debug, Clone, PartialEq)]
pub struct LiveLeaderboard {
    pub id: i64,
    pub workspace_id: i64,
    pub starts_at: i64,
    pub status: String,
    pub reset_time: String,
}

fn filter_where(f: &LeaderboardFilter) -> Sql {
    Sql::and([
        Some(Sql::bind("l.\"workspaceId\" = ", f.workspace_id)),
        Some(Sql::raw("l.\"deletedAt\" IS NULL")),
        f.status
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Sql::bind("l.\"status\" = ", s.to_owned())),
        f.platform
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Sql::bind("l.\"platform\" = ", s.to_owned())),
        f.keyword
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Sql::bind("ilike(l.\"name\", ", like_contains(s)).closed(")")),
    ])
}

fn select_with() -> String {
    format!(
        "SELECT {}, i.\"id\", i.\"name\", i.\"channel\", m.\"id\", m.\"name\", m.\"type\" \
         FROM \"Leaderboard\" l LEFT JOIN \"Inbox\" i ON i.\"id\" = l.\"inboxId\" \
         LEFT JOIN \"Minigame\" m ON m.\"id\" = l.\"minigameId\" WHERE ",
        Leaderboard::columns_sql(Some("l"))
    )
}

fn map_with(q: &Sql, conn: &Connection) -> Result<Vec<LeaderboardWith>> {
    let n = Leaderboard::COLUMNS.len();
    q.query_map(conn, |r| {
        let inbox = match r.get::<_, Option<i64>>(n)? {
            Some(id) => Some(InboxRef {
                id,
                name: r.get(n + 1)?,
                channel: r.get(n + 2)?,
            }),
            None => None,
        };
        let minigame = match r.get::<_, Option<i64>>(n + 3)? {
            Some(id) => Some(MinigameRef {
                id,
                name: r.get(n + 4)?,
                r#type: r.get(n + 5)?,
            }),
            None => None,
        };
        Ok(LeaderboardWith {
            leaderboard: Leaderboard::from_row(r)?,
            inbox,
            minigame,
        })
    })
}

/// `listPaginated`: newest first.
pub fn list_paginated(
    conn: &Connection,
    filter: &LeaderboardFilter,
    page: PaginationInput,
) -> Result<Vec<LeaderboardWith>> {
    todo!("P3.3")
}

/// `count`.
pub fn count(conn: &Connection, filter: &LeaderboardFilter) -> Result<i64> {
    todo!("P3.3")
}

/// `countByStatus`: `(status, total)` of the live leaderboards.
pub fn count_by_status(conn: &Connection, workspace_id: i64) -> Result<Vec<(String, i64)>> {
    todo!("P3.3")
}

/// `findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<LeaderboardWith>> {
    todo!("P3.3")
}

/// `findById` (live rows only).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<Leaderboard>> {
    todo!("P3.3")
}

/// `lockById`: `(id, status)` of a live leaderboard.
pub fn lock_by_id(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<(i64, String)>> {
    todo!("P3.3")
}

fn find_scorable(
    conn: &Connection,
    col: &str,
    id: i64,
    finished_since: i64,
) -> Result<Vec<Leaderboard>> {
    super::find_many(
        conn,
        &format!(
            "WHERE \"{col}\" = ?1 AND \"deletedAt\" IS NULL AND (\"status\" = 'active' OR \
             (\"status\" = 'finished' AND \"rewardsTriggeredAt\" IS NULL AND \"finishedAt\" >= ?2))"
        ),
        &[&id, &finished_since],
    )
}

/// `findScorableForInbox`.
pub fn find_scorable_for_inbox(
    conn: &Connection,
    inbox_id: i64,
    finished_since: i64,
) -> Result<Vec<Leaderboard>> {
    todo!("P3.3")
}

/// `findScorableForMinigame`.
pub fn find_scorable_for_minigame(
    conn: &Connection,
    minigame_id: i64,
    finished_since: i64,
) -> Result<Vec<Leaderboard>> {
    todo!("P3.3")
}

fn update_where(conn: &Connection, values: &Patch, cond: Sql) -> Result<Option<Leaderboard>> {
    let mut all = values.clone();
    if !all.iter().any(|(c, _)| *c == "updatedAt") {
        all.push(("updatedAt", Value::Integer(now_ms())));
    }
    let mut q = Sql::raw("UPDATE \"Leaderboard\" SET ");
    q.append(set_clause(Leaderboard::COLUMNS, &all)?);
    q.push(" WHERE ");
    q.append(cond);
    q.push(&format!(" RETURNING {}", Leaderboard::columns_sql(None)));
    q.model(conn)
}

/// `transitionStatus`: compare-and-set of `status` (`values` = `status` / `finishedAt`).
pub fn transition_status(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    from: &str,
    values: &Patch,
) -> Result<Option<Leaderboard>> {
    todo!("P3.3")
}

/// `claimRewards`: sets `rewardsTriggeredAt` once (keeps the first value) and reports whether
/// it was set before this call.
pub fn claim_rewards(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    now: i64,
) -> Result<Option<ClaimedRewards>> {
    todo!("P3.3")
}

/// `claimRewardRecipient`: marks `contactId` `pending` unless it already has an entry.
pub fn claim_reward_recipient(conn: &Connection, id: i64, contact_id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `setRewardRecipient`: `None` removes the entry.
pub fn set_reward_recipient(
    conn: &Connection,
    id: i64,
    contact_id: i64,
    delivery: Option<&str>,
) -> Result<()> {
    todo!("P3.3")
}

/// `findLiveForWorkspace`: active or paused.
pub fn find_live_for_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<Leaderboard>> {
    todo!("P3.3")
}

/// `findAllLive`.
pub fn find_all_live(conn: &Connection) -> Result<Vec<LiveLeaderboard>> {
    todo!("P3.3")
}

/// `findOverdue`: `(id, workspaceId)` of live boards with `endsAt <= now`.
pub fn find_overdue(conn: &Connection, now: i64) -> Result<Vec<(i64, i64)>> {
    todo!("P3.3")
}

/// `findEarliestLiveStart`.
pub fn find_earliest_live_start(conn: &Connection) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `insert` (`id` / timestamps are filled when the patch has none).
pub fn insert(conn: &Connection, values: &Patch) -> Result<Leaderboard> {
    todo!("P3.3")
}

/// `update` (live rows of the workspace).
pub fn update(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<Leaderboard>> {
    todo!("P3.3")
}

/// `createExportFile`: a pending CSV export file row.
pub fn create_export_file(
    conn: &Connection,
    workspace_id: i64,
    user_id: Option<i64>,
    file_name: &str,
    path: &str,
) -> Result<File> {
    todo!("P3.3")
}

/// `findExportFile`.
pub fn find_export_file(
    conn: &Connection,
    workspace_id: i64,
    file_id: i64,
) -> Result<Option<File>> {
    todo!("P3.3")
}
