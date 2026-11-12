//! Port of `repositories/leaderboard/participant.ts` (P3.3). The score upsert of
//! `applyChangeInTransaction` runs as read + insert/update inside the caller's write job
//! (one SQLite writer; `xmax = 0` becomes "no row before"). `pg_advisory_xact_lock` and
//! `FOR UPDATE` / `FOR SHARE` are no-ops. "now" is passed in (`now: i64`).

use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value as J};

use super::models::LeaderboardParticipant;
use super::sql::Sql;
use super::utils::like_contains;
use super::{Error, Json, Result};

/// The `LeaderboardChangeRejected` error (`kind` of [`Error::Coded`]).
pub const CHANGE_REJECTED: &str = "LeaderboardChangeRejected";

/// `limits` of a score change.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreLimits {
    pub day_bucket: String,
    pub max_per_day: Option<i64>,
    pub cooldown_seconds: i64,
}

/// `LeaderboardScoreChangeInput` (+ `now`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreChangeInput {
    pub leaderboard_id: i64,
    pub workspace_id: i64,
    pub contact_id: i64,
    pub delta: i64,
    pub source: String,
    pub rule: Option<String>,
    pub breakdown_key: Option<String>,
    /// `1` or `-1` (default `1`).
    pub breakdown_step: Option<i64>,
    pub source_ref: Option<String>,
    pub event_id: i64,
    pub event_at: i64,
    pub limits: Option<ScoreLimits>,
    pub referrer_contact_id: Option<i64>,
    pub max_score: i64,
    pub allowed_statuses: Vec<String>,
    pub dedup_since: Option<i64>,
    pub now: i64,
}

/// `LeaderboardScoreChangeResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreChangeResult {
    Duplicate,
    Rejected,
    Closed,
    Applied {
        score: i64,
        scored_at: i64,
        is_new: bool,
        leaderboard_status: String,
    },
}

/// `LeaderboardRankedRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedRow {
    pub contact_id: i64,
    pub score: i64,
    pub scored_at: i64,
}

/// `LeaderboardRankingPageRow` (`cursor` = the `scoredAt` value).
#[derive(Debug, Clone, PartialEq)]
pub struct RankingPageRow {
    pub row: RankedRow,
    pub cursor: i64,
}

/// `LeaderboardChangedRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedRow {
    pub row: RankedRow,
    pub is_banned: bool,
}

/// `LeaderboardParticipantListInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParticipantListInput {
    pub leaderboard_id: i64,
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub banned_only: bool,
    pub contact_ids: Option<Vec<i64>>,
    pub page: i64,
    pub per_page: i64,
}

/// One row of [`list_paginated`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParticipantListRow {
    pub contact_id: i64,
    pub score: i64,
    pub is_banned: bool,
    pub scored_at: i64,
    pub breakdown: Json,
    pub last_activity_at: i64,
    pub created_at: i64,
    pub referrer_contact_id: Option<i64>,
    pub full_name: Option<String>,
    pub avatar: Option<String>,
    pub phone_number: Option<String>,
}

/// Result of [`list_paginated`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParticipantPage {
    pub rows: Vec<ParticipantListRow>,
    pub total: i64,
}

/// One row of [`export_page`].
#[derive(Debug, Clone, PartialEq)]
pub struct ExportRow {
    pub contact_id: i64,
    pub score: i64,
    pub is_banned: bool,
    pub breakdown: Json,
    pub last_activity_at: i64,
    pub created_at: i64,
    pub full_name: Option<String>,
    pub phone_number: Option<String>,
    pub email: Option<String>,
}

/// `{ id, fullName, avatar }` of [`find_contact_profiles`].
#[derive(Debug, Clone, PartialEq)]
pub struct ContactProfile {
    pub id: i64,
    pub full_name: Option<String>,
    pub avatar: Option<String>,
}

const MAX_PER_PAGE: i64 = 100;

fn json_int(v: &J, key: &str) -> i64 {
    v.get(key).and_then(J::as_i64).unwrap_or(0)
}

fn rejected() -> Error {
    Error::Coded {
        kind: CHANGE_REJECTED,
        code: CHANGE_REJECTED.to_owned(),
        message: "Leaderboard score change rejected".to_owned(),
    }
}

/// `applyChangeInTransaction` (run inside one write job). A banned participant, a full daily
/// cap or a running cooldown returns the `LeaderboardChangeRejected` error.
pub fn apply_change_in_transaction(
    conn: &Connection,
    input: &ScoreChangeInput,
) -> Result<ScoreChangeResult> {
    let statuses =
        serde_json::to_string(&input.allowed_statuses).map_err(|e| Error::msg(e.to_string()))?;
    let status: Option<String> = conn
        .query_row(
            "SELECT \"status\" FROM \"Leaderboard\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"deletedAt\" IS NULL \
             AND \"status\" IN (SELECT value FROM json_each(?3)) AND (\"status\" <> 'finished' OR \"rewardsTriggeredAt\" IS NULL)",
            rusqlite::params![input.leaderboard_id, input.workspace_id, statuses],
            |r| r.get(0),
        )
        .optional()?;
    let Some(leaderboard_status) = status else {
        return Ok(ScoreChangeResult::Closed);
    };
    if let Some(since) = input.dedup_since {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM \"LeaderboardScoreEvent\" WHERE \"leaderboardId\" = ?1 AND \"contactId\" = ?2 \
                 AND \"createdAt\" >= ?3 AND \"id\" = ?4 LIMIT 1",
                rusqlite::params![input.leaderboard_id, input.contact_id, since, input.event_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(ScoreChangeResult::Duplicate);
        }
    }
    let inserted = conn.execute(
        "INSERT INTO \"LeaderboardScoreEvent\" (\"id\", \"createdAt\", \"leaderboardId\", \"workspaceId\", \"contactId\", \
         \"delta\", \"source\", \"rule\", \"sourceRef\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT (\"createdAt\", \"id\") DO NOTHING",
        rusqlite::params![
            input.event_id,
            input.event_at,
            input.leaderboard_id,
            input.workspace_id,
            input.contact_id,
            input.delta,
            input.source,
            input.rule,
            input.source_ref
        ],
    )?;
    if inserted == 0 {
        return Ok(ScoreChangeResult::Duplicate);
    }
    let counter_key = input.rule.clone().or_else(|| input.breakdown_key.clone());
    let step = input.breakdown_step.unwrap_or(1);
    let limits = input.limits.as_ref();
    let day_bucket = limits.map(|l| l.day_bucket.clone());
    let rule_for_daily = if limits.is_some() {
        input.rule.clone()
    } else {
        None
    };
    let daily_rule = rule_for_daily.clone().filter(|_| step > 0);
    let now = input.now;

    let existing = super::find_one::<LeaderboardParticipant>(
        conn,
        "\"leaderboardId\" = ?1 AND \"contactId\" = ?2",
        &[&input.leaderboard_id, &input.contact_id],
    )?;
    let Some(p) = existing else {
        let mut breakdown = Map::new();
        if let Some(k) = &counter_key {
            breakdown.insert(k.clone(), J::from(step.max(0)));
        }
        let mut daily = Map::new();
        if let Some(r) = &daily_rule {
            daily.insert(r.clone(), J::from(1));
        }
        let score = input.max_score.min(input.delta.max(0));
        conn.execute(
            "INSERT INTO \"LeaderboardParticipant\" (\"leaderboardId\", \"contactId\", \"workspaceId\", \"score\", \
             \"referrerContactId\", \"scoredAt\", \"dailyBucket\", \"dailyCounts\", \"breakdown\", \"lastActivityAt\", \"createdAt\") \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?6, ?6)",
            rusqlite::params![
                input.leaderboard_id,
                input.contact_id,
                input.workspace_id,
                score,
                input.referrer_contact_id,
                now,
                daily_rule.as_ref().and(day_bucket.clone()),
                J::Object(daily).to_string(),
                J::Object(breakdown).to_string()
            ],
        )?;
        return Ok(ScoreChangeResult::Applied {
            score,
            scored_at: now,
            is_new: true,
            leaderboard_status,
        });
    };
    if p.is_banned {
        return Err(rejected());
    }
    let same_bucket = p.daily_bucket.is_some() && p.daily_bucket == day_bucket;
    if let (Some(l), Some(r)) = (limits, &daily_rule)
        && let Some(max) = l.max_per_day
        && same_bucket
        && json_int(&p.daily_counts.0, r) >= max
    {
        return Err(rejected());
    }
    if let Some(l) = limits
        && l.cooldown_seconds > 0
        && p.scored_at > now - l.cooldown_seconds * 1000
    {
        return Err(rejected());
    }
    let mut daily = p.daily_counts.0.as_object().cloned().unwrap_or_default();
    if let Some(r) = &daily_rule {
        if same_bucket {
            let n = json_int(&p.daily_counts.0, r) + 1;
            daily.insert(r.clone(), J::from(n));
        } else {
            daily = Map::new();
            daily.insert(r.clone(), J::from(1));
        }
    }
    let mut breakdown = p.breakdown.0.as_object().cloned().unwrap_or_default();
    if let Some(k) = &counter_key {
        let n = (json_int(&p.breakdown.0, k) + step).max(0);
        breakdown.insert(k.clone(), J::from(n));
    }
    let score = input.max_score.min((p.score + input.delta).max(0));
    let scored_at = if input.delta != 0 { now } else { p.scored_at };
    let bucket = if daily_rule.is_some() {
        day_bucket
    } else {
        p.daily_bucket.clone()
    };
    conn.execute(
        "UPDATE \"LeaderboardParticipant\" SET \"score\" = ?3, \"dailyCounts\" = ?4, \"breakdown\" = ?5, \
         \"scoredAt\" = ?6, \"lastActivityAt\" = ?7, \"dailyBucket\" = ?8 WHERE \"leaderboardId\" = ?1 AND \"contactId\" = ?2",
        rusqlite::params![
            input.leaderboard_id,
            input.contact_id,
            score,
            J::Object(daily).to_string(),
            J::Object(breakdown).to_string(),
            scored_at,
            now,
            bucket
        ],
    )?;
    Ok(ScoreChangeResult::Applied {
        score,
        scored_at,
        is_new: false,
        leaderboard_status,
    })
}

/// `find`.
pub fn find(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<LeaderboardParticipant>> {
    super::find_one(
        conn,
        "\"leaderboardId\" = ?1 AND \"contactId\" = ?2",
        &[&leaderboard_id, &contact_id],
    )
}

/// `lock`: `(score, isBanned)`.
pub fn lock(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<(i64, bool)>> {
    Ok(conn
        .query_row(
            "SELECT \"score\", \"isBanned\" FROM \"LeaderboardParticipant\" WHERE \"leaderboardId\" = ?1 AND \"contactId\" = ?2",
            rusqlite::params![leaderboard_id, contact_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn ranked(r: &rusqlite::Row<'_>) -> rusqlite::Result<RankedRow> {
    Ok(RankedRow {
        contact_id: r.get(0)?,
        score: r.get(1)?,
        scored_at: r.get(2)?,
    })
}

/// `setBanned`.
pub fn set_banned(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
    is_banned: bool,
) -> Result<Option<RankedRow>> {
    Ok(conn
        .query_row(
            "UPDATE \"LeaderboardParticipant\" SET \"isBanned\" = ?3 WHERE \"leaderboardId\" = ?1 AND \"contactId\" = ?2 \
             RETURNING \"contactId\", \"score\", \"scoredAt\"",
            rusqlite::params![leaderboard_id, contact_id, is_banned],
            ranked,
        )
        .optional()?)
}

/// `resetAll`: returns the changed row count.
pub fn reset_all(conn: &Connection, leaderboard_id: i64, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE \"LeaderboardParticipant\" SET \"score\" = 0, \"scoredAt\" = ?2, \"breakdown\" = '{}', \
         \"dailyCounts\" = '{}', \"dailyBucket\" = NULL WHERE \"leaderboardId\" = ?1",
        rusqlite::params![leaderboard_id, now],
    )?)
}

const RANK_ORDER: &str = "ORDER BY \"score\" DESC, \"scoredAt\" ASC, \"contactId\" ASC";

/// `topRanked`.
pub fn top_ranked(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    offset: Option<i64>,
) -> Result<Vec<RankedRow>> {
    let sql = format!(
        "SELECT \"contactId\", \"score\", \"scoredAt\" FROM \"LeaderboardParticipant\" \
         WHERE \"leaderboardId\" = ?1 AND NOT \"isBanned\" {RANK_ORDER} LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![leaderboard_id, limit, offset.unwrap_or(0)],
            ranked,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `rankOf`: `(rank, score)` of an unbanned participant.
pub fn rank_of(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<(i64, i64)>> {
    Ok(conn
        .query_row(
            "SELECT (SELECT count(*) + 1 FROM \"LeaderboardParticipant\" o WHERE o.\"leaderboardId\" = me.\"leaderboardId\" \
             AND NOT o.\"isBanned\" AND (o.\"score\" > me.\"score\" OR (o.\"score\" = me.\"score\" AND o.\"scoredAt\" < me.\"scoredAt\"))), \
             me.\"score\" FROM \"LeaderboardParticipant\" me WHERE me.\"leaderboardId\" = ?1 AND me.\"contactId\" = ?2 AND NOT me.\"isBanned\"",
            rusqlite::params![leaderboard_id, contact_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// `rankingPage`: keyset page after `after`.
pub fn ranking_page(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    after: Option<&RankingPageRow>,
) -> Result<Vec<RankingPageRow>> {
    let mut q = Sql::bind(
        "SELECT \"contactId\", \"score\", \"scoredAt\" FROM \"LeaderboardParticipant\" WHERE NOT \"isBanned\" AND \"leaderboardId\" = ",
        leaderboard_id,
    );
    if let Some(a) = after {
        q.push(" AND (\"score\" < ");
        q.push_bind(a.row.score);
        q.push(" OR (\"score\" = ");
        q.push_bind(a.row.score);
        q.push(" AND \"scoredAt\" > ");
        q.push_bind(a.cursor);
        q.push(") OR (\"score\" = ");
        q.push_bind(a.row.score);
        q.push(" AND \"scoredAt\" = ");
        q.push_bind(a.cursor);
        q.push(" AND \"contactId\" > ");
        q.push_bind(a.row.contact_id);
        q.push("))");
    }
    q.push(&format!(" {RANK_ORDER} LIMIT "));
    q.push_bind(limit);
    q.query_map(conn, |r| {
        let row = ranked(r)?;
        Ok(RankingPageRow {
            cursor: row.scored_at,
            row,
        })
    })
}

/// `lockChangedSince`: rows with `lastActivityAt >= since`, by contact id.
pub fn lock_changed_since(
    conn: &Connection,
    leaderboard_id: i64,
    since: i64,
) -> Result<Vec<ChangedRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\", \"score\", \"scoredAt\", \"isBanned\" FROM \"LeaderboardParticipant\" \
         WHERE \"leaderboardId\" = ?1 AND \"lastActivityAt\" >= ?2 ORDER BY \"contactId\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![leaderboard_id, since], |r| {
            Ok(ChangedRow {
                row: ranked(r)?,
                is_banned: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `stats`: `(participants, totalPoints, newToday)`.
pub fn stats(conn: &Connection, leaderboard_id: i64, new_since: i64) -> Result<(i64, i64, i64)> {
    Ok(conn.query_row(
        "SELECT count(*), COALESCE(sum(\"score\"), 0), count(*) FILTER (WHERE \"createdAt\" >= ?2) \
         FROM \"LeaderboardParticipant\" WHERE \"leaderboardId\" = ?1",
        rusqlite::params![leaderboard_id, new_since],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}

fn is_contact_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit())
}

fn list_where(input: &ParticipantListInput) -> Sql {
    let ids = input.contact_ids.as_ref().map(|ids| {
        if ids.is_empty() {
            Sql::raw("0")
        } else {
            Sql::bind(
                "p.\"contactId\" IN (SELECT value FROM json_each(",
                super::id_list(ids),
            )
            .closed("))")
        }
    });
    let keyword = input
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            let pattern = like_contains(k);
            let mut s = Sql::bind("ilike(c.\"fullName\", ", pattern.clone());
            s.push(") OR ilike(c.\"phoneNumber\", ");
            s.push_bind(pattern);
            s.push(")");
            if is_contact_id(k)
                && let Ok(id) = k.parse::<i64>()
            {
                s.push(" OR p.\"contactId\" = ");
                s.push_bind(id);
            }
            s
        });
    Sql::and([
        Some(Sql::bind("p.\"leaderboardId\" = ", input.leaderboard_id)),
        Some(Sql::bind("p.\"workspaceId\" = ", input.workspace_id)),
        input.banned_only.then(|| Sql::raw("p.\"isBanned\"")),
        ids,
        keyword,
    ])
}

const PARTICIPANT_JOIN: &str = "FROM \"LeaderboardParticipant\" p JOIN \"Contact\" c ON c.\"id\" = p.\"contactId\" \
                                AND c.\"workspaceId\" = p.\"workspaceId\" WHERE ";
const PARTICIPANT_ORDER: &str =
    " ORDER BY p.\"isBanned\" ASC, p.\"score\" DESC, p.\"scoredAt\" ASC, p.\"contactId\" ASC";

/// `listPaginated` (`perPage` clamped to 1..=100).
pub fn list_paginated(conn: &Connection, input: &ParticipantListInput) -> Result<ParticipantPage> {
    let per_page = input.per_page.clamp(1, MAX_PER_PAGE);
    let offset = (input.page.max(1) - 1) * per_page;
    let mut q = Sql::raw(format!(
        "SELECT p.\"contactId\", p.\"score\", p.\"isBanned\", p.\"scoredAt\", p.\"breakdown\", p.\"lastActivityAt\", \
         p.\"createdAt\", p.\"referrerContactId\", c.\"fullName\", c.\"avatar\", c.\"phoneNumber\" {PARTICIPANT_JOIN}"
    ));
    q.append(list_where(input));
    q.push(PARTICIPANT_ORDER);
    q.push(" LIMIT ");
    q.push_bind(per_page);
    q.push(" OFFSET ");
    q.push_bind(offset);
    let rows = q.query_map(conn, |r| {
        Ok(ParticipantListRow {
            contact_id: r.get(0)?,
            score: r.get(1)?,
            is_banned: r.get(2)?,
            scored_at: r.get(3)?,
            breakdown: r.get(4)?,
            last_activity_at: r.get(5)?,
            created_at: r.get(6)?,
            referrer_contact_id: r.get(7)?,
            full_name: r.get(8)?,
            avatar: r.get(9)?,
            phone_number: r.get(10)?,
        })
    })?;
    let mut c = Sql::raw(format!("SELECT count(*) {PARTICIPANT_JOIN}"));
    c.append(list_where(input));
    let total = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok(ParticipantPage { rows, total })
}

/// `findContactProfiles`.
pub fn find_contact_profiles(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<ContactProfile>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"fullName\", \"avatar\" FROM \"Contact\" WHERE \"workspaceId\" = ?1 \
         AND \"id\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, super::id_list(contact_ids)],
            |r| {
                Ok(ContactProfile {
                    id: r.get(0)?,
                    full_name: r.get(1)?,
                    avatar: r.get(2)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `contactHasAnyTag` (`true` for an empty tag list).
pub fn contact_has_any_tag(conn: &Connection, contact_id: i64, tag_ids: &[i64]) -> Result<bool> {
    if tag_ids.is_empty() {
        return Ok(true);
    }
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM \"ContactToTag\" WHERE \"contactId\" = ?1 AND \"tagId\" IN (SELECT value FROM json_each(?2)) LIMIT 1",
            rusqlite::params![contact_id, super::id_list(tag_ids)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// `exportPage`.
pub fn export_page(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<ExportRow>> {
    let sql = format!(
        "SELECT p.\"contactId\", p.\"score\", p.\"isBanned\", p.\"breakdown\", p.\"lastActivityAt\", p.\"createdAt\", \
         c.\"fullName\", c.\"phoneNumber\", c.\"email\" {PARTICIPANT_JOIN} p.\"leaderboardId\" = ?1{PARTICIPANT_ORDER} LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![leaderboard_id, limit, offset], |r| {
            Ok(ExportRow {
                contact_id: r.get(0)?,
                score: r.get(1)?,
                is_banned: r.get(2)?,
                breakdown: r.get(3)?,
                last_activity_at: r.get(4)?,
                created_at: r.get(5)?,
                full_name: r.get(6)?,
                phone_number: r.get(7)?,
                email: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `findLatestReferrerName`.
pub fn find_latest_referrer_name(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT c.\"fullName\" FROM \"LeaderboardParticipant\" p JOIN \"Contact\" c ON c.\"id\" = p.\"referrerContactId\" \
             AND c.\"workspaceId\" = p.\"workspaceId\" WHERE p.\"contactId\" = ?1 AND p.\"workspaceId\" = ?2 \
             AND p.\"referrerContactId\" IS NOT NULL ORDER BY p.\"createdAt\" DESC LIMIT 1",
            rusqlite::params![contact_id, workspace_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// `countReferredBy`.
pub fn count_referred_by(
    conn: &Connection,
    leaderboard_id: i64,
    referrer_contact_id: i64,
) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"LeaderboardParticipant\" WHERE \"leaderboardId\" = ?1 AND \"referrerContactId\" = ?2",
        rusqlite::params![leaderboard_id, referrer_contact_id],
        |r| r.get(0),
    )?)
}
