//! Port of `repositories/poll/repository.ts` (P3.3). `upsertVote` runs as read +
//! insert/update in one write job (one SQLite writer; `xmax = 0` = "no row before").

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};

use super::models::{Poll, PollCandidate};
use super::patch::{Patch, set_clause};
use super::sql::Sql;
use super::utils::{PaginationInput, get_pagination_with_defaults, like_contains};
use super::{Model, Result, id_list, new_id, now_ms};

/// `PollCandidateInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct PollCandidateInput {
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
}

/// `PollInsertInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct PollInsertInput {
    pub workspace_id: i64,
    pub name: String,
    pub image_url: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub after_vote_flow_id: Option<i64>,
}

/// `PollWithCandidates`.
#[derive(Debug, Clone, PartialEq)]
pub struct PollWithCandidates {
    pub poll: Poll,
    pub candidates: Vec<PollCandidate>,
}

/// A poll with its vote count.
#[derive(Debug, Clone, PartialEq)]
pub struct PollWithVotes {
    pub poll: Poll,
    pub votes_count: i64,
}

/// Result of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct PollPage {
    pub data: Vec<PollWithVotes>,
    pub page_count: i64,
}

/// `PollVoteResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollVoteResult {
    New,
    Changed,
    Unchanged,
}

impl PollVoteResult {
    /// `"new"` / `"changed"` / `"unchanged"`.
    pub fn as_str(self) -> &'static str {
        match self {
            PollVoteResult::New => "new",
            PollVoteResult::Changed => "changed",
            PollVoteResult::Unchanged => "unchanged",
        }
    }
}

/// `PollVoteOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollVoteOutcome {
    pub result: PollVoteResult,
    pub voted_at: Option<i64>,
}

/// Input of [`upsert_vote`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteInput {
    pub workspace_id: i64,
    pub poll_id: i64,
    pub candidate_id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: Option<i64>,
}

/// `PollVoteBreakdownRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoteBreakdownRow {
    pub candidate_id: i64,
    pub gender: String,
    pub votes: i64,
}

fn load_with_candidates(
    conn: &Connection,
    id: i64,
    workspace_id: Option<i64>,
) -> Result<Option<PollWithCandidates>> {
    let poll: Option<Poll> = match workspace_id {
        Some(ws) => super::find_one(conn, "\"id\" = ?1 AND \"workspaceId\" = ?2", &[&id, &ws])?,
        None => super::find_one(conn, "\"id\" = ?1", &[&id])?,
    };
    let Some(poll) = poll else {
        return Ok(None);
    };
    let candidates = super::find_many(
        conn,
        "WHERE \"pollId\" = ?1 ORDER BY \"position\" ASC",
        &[&poll.id],
    )?;
    Ok(Some(PollWithCandidates { poll, candidates }))
}

fn list_where(workspace_id: i64, name: Option<&str>) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        name.filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(\"name\", ", like_contains(n)).closed(")")),
    ])
}

/// `list`: newest first, with vote counts.
pub fn list(
    conn: &Connection,
    workspace_id: i64,
    name: Option<&str>,
    page: PaginationInput,
) -> Result<PollPage> {
    todo!("P3.3")
}

/// `findWithCandidates` (`forUpdate` is a no-op).
pub fn find_with_candidates(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PollWithCandidates>> {
    todo!("P3.3")
}

/// `findUnscopedWithCandidates`.
pub fn find_unscoped_with_candidates(
    conn: &Connection,
    id: i64,
) -> Result<Option<PollWithCandidates>> {
    todo!("P3.3")
}

/// `listRecent`: newest first; `running_at` keeps polls with `startsAt <= t < endsAt`.
pub fn list_recent(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    running_at: Option<i64>,
) -> Result<Vec<Poll>> {
    todo!("P3.3")
}

fn insert_candidate(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    position: i64,
    c: &PollCandidateInput,
) -> Result<PollCandidate> {
    let now = now_ms();
    super::returning::<PollCandidate>(
        conn,
        "INSERT INTO \"PollCandidate\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"pollId\", \"position\", \
         \"name\", \"description\", \"imageUrl\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        &[&new_id(), &now, &workspace_id, &poll_id, &position, &c.name, &c.description, &c.image_url],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to insert poll candidate"))
}

/// `insertWithCandidates` (candidates get positions 0..n).
pub fn insert_with_candidates(
    conn: &Connection,
    input: &PollInsertInput,
    candidates: &[PollCandidateInput],
) -> Result<PollWithCandidates> {
    todo!("P3.3")
}

/// `updateCandidate`.
pub fn update_candidate(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    id: i64,
    position: i64,
    candidate: &PollCandidateInput,
) -> Result<()> {
    todo!("P3.3")
}

/// `insertCandidates` (`(position, candidate)` pairs).
pub fn insert_candidates(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    candidates: &[(i64, PollCandidateInput)],
) -> Result<Vec<PollCandidate>> {
    todo!("P3.3")
}

/// `deleteCandidates`.
pub fn delete_candidates(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    ids: &[i64],
) -> Result<()> {
    todo!("P3.3")
}

/// `update` (`data` columns: `name`, `imageUrl`, `startsAt`, `endsAt`, `afterVoteFlowId`).
pub fn update(conn: &Connection, workspace_id: i64, id: i64, data: &Patch) -> Result<Option<Poll>> {
    todo!("P3.3")
}

/// `deleteMany`: returns the deleted count.
pub fn delete_many(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<usize> {
    todo!("P3.3")
}

/// `upsertVote`: one vote per (poll, contact); a re-vote for the same candidate is
/// `unchanged` and reports when the current vote was cast.
pub fn upsert_vote(conn: &Connection, input: &VoteInput) -> Result<PollVoteOutcome> {
    todo!("P3.3")
}

/// `voteCounts`: `(candidateId, votes)`.
pub fn vote_counts(conn: &Connection, workspace_id: i64, poll_id: i64) -> Result<Vec<(i64, i64)>> {
    todo!("P3.3")
}

/// `voteBreakdown`: votes per candidate and contact gender (`unknown` for none).
pub fn vote_breakdown(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
) -> Result<Vec<VoteBreakdownRow>> {
    todo!("P3.3")
}
