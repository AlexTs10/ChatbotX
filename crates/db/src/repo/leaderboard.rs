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
    let p = get_pagination_with_defaults(page);
    let mut q = Sql::raw(select_with());
    q.append(filter_where(filter));
    q.push(" ORDER BY l.\"createdAt\" DESC LIMIT ");
    q.push_bind(p.limit);
    q.push(" OFFSET ");
    q.push_bind(p.offset);
    map_with(&q, conn)
}

/// `count`.
pub fn count(conn: &Connection, filter: &LeaderboardFilter) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Leaderboard\" l WHERE ");
    q.append(filter_where(filter));
    Ok(q.query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `countByStatus`: `(status, total)` of the live leaderboards.
pub fn count_by_status(conn: &Connection, workspace_id: i64) -> Result<Vec<(String, i64)>> {
    Sql::bind(
        "SELECT \"status\", count(*) FROM \"Leaderboard\" WHERE \"deletedAt\" IS NULL AND \"workspaceId\" = ",
        workspace_id,
    )
    .closed(" GROUP BY \"status\"")
    .query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))
}

/// `findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<LeaderboardWith>> {
    let mut q = Sql::raw(select_with());
    q.append(Sql::and([
        Some(Sql::bind("l.\"id\" = ", id)),
        Some(Sql::bind("l.\"workspaceId\" = ", workspace_id)),
        Some(Sql::raw("l.\"deletedAt\" IS NULL")),
    ]));
    q.push(" LIMIT 1");
    Ok(map_with(&q, conn)?.into_iter().next())
}

/// `findById` (live rows only).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<Leaderboard>> {
    super::find_one(conn, "\"id\" = ?1 AND \"deletedAt\" IS NULL", &[&id])
}

/// `lockById`: `(id, status)` of a live leaderboard.
pub fn lock_by_id(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"status\" FROM \"Leaderboard\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"deletedAt\" IS NULL",
    )?;
    let mut rows = stmt.query(rusqlite::params![id, workspace_id])?;
    Ok(match rows.next()? {
        Some(r) => Some((r.get(0)?, r.get(1)?)),
        None => None,
    })
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
    find_scorable(conn, "inboxId", inbox_id, finished_since)
}

/// `findScorableForMinigame`.
pub fn find_scorable_for_minigame(
    conn: &Connection,
    minigame_id: i64,
    finished_since: i64,
) -> Result<Vec<Leaderboard>> {
    find_scorable(conn, "minigameId", minigame_id, finished_since)
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
    update_where(
        conn,
        values,
        Sql::and([
            Some(Sql::bind("\"id\" = ", id)),
            Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
            Some(Sql::bind("\"status\" = ", from.to_owned())),
            Some(Sql::raw("\"deletedAt\" IS NULL")),
        ]),
    )
}

/// `claimRewards`: sets `rewardsTriggeredAt` once (keeps the first value) and reports whether
/// it was set before this call.
pub fn claim_rewards(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    now: i64,
) -> Result<Option<ClaimedRewards>> {
    let before: Option<(Option<i64>, Option<Json>)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT \"rewardsTriggeredAt\", \"rewardRecipients\" FROM \"Leaderboard\" \
             WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'finished' AND \"deletedAt\" IS NULL",
        )?;
        let mut rows = stmt.query(rusqlite::params![id, workspace_id])?;
        match rows.next()? {
            Some(r) => Some((r.get(0)?, r.get(1)?)),
            None => None,
        }
    };
    let Some((triggered_at, recipients)) = before else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE \"Leaderboard\" SET \"rewardsTriggeredAt\" = COALESCE(\"rewardsTriggeredAt\", ?2) WHERE \"id\" = ?1",
        rusqlite::params![id, now],
    )?;
    let recipients = match recipients {
        Some(Json(v)) if !v.is_null() => v,
        _ => serde_json::Value::Object(Default::default()),
    };
    Ok(Some(ClaimedRewards {
        recipients,
        previously_triggered: triggered_at.is_some(),
    }))
}

/// `claimRewardRecipient`: marks `contactId` `pending` unless it already has an entry.
pub fn claim_reward_recipient(conn: &Connection, id: i64, contact_id: i64) -> Result<bool> {
    let key = format!("$.\"{contact_id}\"");
    let n = conn.execute(
        "UPDATE \"Leaderboard\" SET \"rewardRecipients\" = json_set(COALESCE(\"rewardRecipients\", '{}'), ?2, 'pending') \
         WHERE \"id\" = ?1 AND json_type(COALESCE(\"rewardRecipients\", '{}'), ?2) IS NULL",
        rusqlite::params![id, key],
    )?;
    Ok(n > 0)
}

/// `setRewardRecipient`: `None` removes the entry.
pub fn set_reward_recipient(
    conn: &Connection,
    id: i64,
    contact_id: i64,
    delivery: Option<&str>,
) -> Result<()> {
    let key = format!("$.\"{contact_id}\"");
    match delivery {
        None => conn.execute(
            "UPDATE \"Leaderboard\" SET \"rewardRecipients\" = json_remove(COALESCE(\"rewardRecipients\", '{}'), ?2) WHERE \"id\" = ?1",
            rusqlite::params![id, key],
        )?,
        Some(d) => conn.execute(
            "UPDATE \"Leaderboard\" SET \"rewardRecipients\" = json_set(COALESCE(\"rewardRecipients\", '{}'), ?2, ?3) WHERE \"id\" = ?1",
            rusqlite::params![id, key, d],
        )?,
    };
    Ok(())
}

/// `findLiveForWorkspace`: active or paused.
pub fn find_live_for_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<Leaderboard>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"status\" IN ('active', 'paused') AND \"deletedAt\" IS NULL",
        &[&workspace_id],
    )
}

/// `findAllLive`.
pub fn find_all_live(conn: &Connection) -> Result<Vec<LiveLeaderboard>> {
    Sql::raw(
        "SELECT \"id\", \"workspaceId\", \"startsAt\", \"status\", \"resetTime\" FROM \"Leaderboard\" \
         WHERE \"status\" IN ('active', 'paused') AND \"deletedAt\" IS NULL",
    )
    .query_map(conn, |r| {
        Ok(LiveLeaderboard {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            starts_at: r.get(2)?,
            status: r.get(3)?,
            reset_time: r.get(4)?,
        })
    })
}

/// `findOverdue`: `(id, workspaceId)` of live boards with `endsAt <= now`.
pub fn find_overdue(conn: &Connection, now: i64) -> Result<Vec<(i64, i64)>> {
    Sql::bind(
        "SELECT \"id\", \"workspaceId\" FROM \"Leaderboard\" WHERE \"status\" IN ('active', 'paused') \
         AND \"deletedAt\" IS NULL AND \"endsAt\" <= ",
        now,
    )
    .query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))
}

/// `findEarliestLiveStart`.
pub fn find_earliest_live_start(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT min(\"startsAt\") FROM \"Leaderboard\" WHERE \"status\" IN ('active', 'paused') AND \"deletedAt\" IS NULL",
        [],
        |r| r.get(0),
    )?)
}

/// `insert` (`id` / timestamps are filled when the patch has none).
pub fn insert(conn: &Connection, values: &Patch) -> Result<Leaderboard> {
    let now = now_ms();
    let mut all: Patch = Vec::new();
    for (col, v) in [
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
    ] {
        if !values.iter().any(|(c, _)| *c == col) {
            all.push((col, v));
        }
    }
    all.extend(values.iter().cloned());
    let mut q = Sql::raw("INSERT INTO \"Leaderboard\" ");
    q.append(insert_clause(Leaderboard::COLUMNS, &all)?);
    q.push(&format!(" RETURNING {}", Leaderboard::columns_sql(None)));
    q.model(conn)?
        .ok_or_else(|| super::Error::msg("Failed to insert leaderboard"))
}

/// `update` (live rows of the workspace).
pub fn update(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<Leaderboard>> {
    update_where(
        conn,
        values,
        Sql::and([
            Some(Sql::bind("\"id\" = ", id)),
            Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
            Some(Sql::raw("\"deletedAt\" IS NULL")),
        ]),
    )
}

/// `createExportFile`: a pending CSV export file row.
pub fn create_export_file(
    conn: &Connection,
    workspace_id: i64,
    user_id: Option<i64>,
    file_name: &str,
    path: &str,
) -> Result<File> {
    let now = now_ms();
    super::returning::<File>(
        conn,
        "INSERT INTO \"File\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"userId\", \"contextType\", \
         \"subType\", \"path\", \"fileName\", \"mimeType\", \"status\") \
         VALUES (?1, ?2, ?2, ?3, ?4, 'export', 'leaderboardParticipants', ?5, ?6, 'text/csv', 'pending')",
        &[&new_id(), &now, &workspace_id, &user_id, &path, &file_name],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to create export file"))
}

/// `findExportFile`.
pub fn find_export_file(
    conn: &Connection,
    workspace_id: i64,
    file_id: i64,
) -> Result<Option<File>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"contextType\" = 'export' AND \"subType\" = 'leaderboardParticipants'",
        &[&file_id, &workspace_id],
    )
}
