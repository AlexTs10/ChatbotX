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
    todo!("P3.3")
}

/// `find`.
pub fn find(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<LeaderboardParticipant>> {
    todo!("P3.3")
}

/// `lock`: `(score, isBanned)`.
pub fn lock(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<(i64, bool)>> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `resetAll`: returns the changed row count.
pub fn reset_all(conn: &Connection, leaderboard_id: i64, now: i64) -> Result<usize> {
    todo!("P3.3")
}

const RANK_ORDER: &str = "ORDER BY \"score\" DESC, \"scoredAt\" ASC, \"contactId\" ASC";

/// `topRanked`.
pub fn top_ranked(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    offset: Option<i64>,
) -> Result<Vec<RankedRow>> {
    todo!("P3.3")
}

/// `rankOf`: `(rank, score)` of an unbanned participant.
pub fn rank_of(
    conn: &Connection,
    leaderboard_id: i64,
    contact_id: i64,
) -> Result<Option<(i64, i64)>> {
    todo!("P3.3")
}

/// `rankingPage`: keyset page after `after`.
pub fn ranking_page(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    after: Option<&RankingPageRow>,
) -> Result<Vec<RankingPageRow>> {
    todo!("P3.3")
}

/// `lockChangedSince`: rows with `lastActivityAt >= since`, by contact id.
pub fn lock_changed_since(
    conn: &Connection,
    leaderboard_id: i64,
    since: i64,
) -> Result<Vec<ChangedRow>> {
    todo!("P3.3")
}

/// `stats`: `(participants, totalPoints, newToday)`.
pub fn stats(conn: &Connection, leaderboard_id: i64, new_since: i64) -> Result<(i64, i64, i64)> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `findContactProfiles`.
pub fn find_contact_profiles(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<ContactProfile>> {
    todo!("P3.3")
}

/// `contactHasAnyTag` (`true` for an empty tag list).
pub fn contact_has_any_tag(conn: &Connection, contact_id: i64, tag_ids: &[i64]) -> Result<bool> {
    todo!("P3.3")
}

/// `exportPage`.
pub fn export_page(
    conn: &Connection,
    leaderboard_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<ExportRow>> {
    todo!("P3.3")
}

/// `findLatestReferrerName`.
pub fn find_latest_referrer_name(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<String>> {
    todo!("P3.3")
}

/// `countReferredBy`.
pub fn count_referred_by(
    conn: &Connection,
    leaderboard_id: i64,
    referrer_contact_id: i64,
) -> Result<i64> {
    todo!("P3.3")
}
