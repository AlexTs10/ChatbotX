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
    let p = get_pagination_with_defaults(page);
    let mut q = Sql::raw(format!("{} WHERE ", Poll::select_sql()));
    q.append(list_where(workspace_id, name));
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(p.limit);
    q.push(" OFFSET ");
    q.push_bind(p.offset);
    let rows: Vec<Poll> = q.models(conn)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"Poll\" WHERE ");
    c.append(list_where(workspace_id, name));
    let total: i64 = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let mut votes: Vec<(i64, i64)> = Vec::new();
    if !rows.is_empty() {
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let mut stmt = conn.prepare_cached(
            "SELECT \"pollId\", count(*) FROM \"PollVote\" WHERE \"workspaceId\" = ?1 \
             AND \"pollId\" IN (SELECT value FROM json_each(?2)) GROUP BY \"pollId\"",
        )?;
        votes = stmt
            .query_map(rusqlite::params![workspace_id, id_list(&ids)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(PollPage {
        data: rows
            .into_iter()
            .map(|poll| {
                let votes_count = votes
                    .iter()
                    .find(|(id, _)| *id == poll.id)
                    .map_or(0, |(_, n)| *n);
                PollWithVotes { poll, votes_count }
            })
            .collect(),
        page_count: (total + p.limit - 1) / p.limit,
    })
}

/// `findWithCandidates` (`forUpdate` is a no-op).
pub fn find_with_candidates(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PollWithCandidates>> {
    load_with_candidates(conn, id, Some(workspace_id))
}

/// `findUnscopedWithCandidates`.
pub fn find_unscoped_with_candidates(
    conn: &Connection,
    id: i64,
) -> Result<Option<PollWithCandidates>> {
    load_with_candidates(conn, id, None)
}

/// `listRecent`: newest first; `running_at` keeps polls with `startsAt <= t < endsAt`.
pub fn list_recent(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    running_at: Option<i64>,
) -> Result<Vec<Poll>> {
    let mut q = Sql::raw(format!("{} WHERE ", Poll::select_sql()));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        running_at.map(|t| {
            let mut s = Sql::bind("\"startsAt\" <= ", t);
            s.push(" AND \"endsAt\" > ");
            s.push_bind(t);
            s
        }),
    ]));
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(limit);
    q.models(conn)
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
    let now = now_ms();
    let poll = super::returning::<Poll>(
        conn,
        "INSERT INTO \"Poll\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\", \"imageUrl\", \"startsAt\", \
         \"endsAt\", \"afterVoteFlowId\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.name,
            &input.image_url,
            &input.starts_at,
            &input.ends_at,
            &input.after_vote_flow_id,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to insert poll"))?;
    let mut inserted = Vec::with_capacity(candidates.len());
    for (i, c) in candidates.iter().enumerate() {
        inserted.push(insert_candidate(
            conn,
            input.workspace_id,
            poll.id,
            i as i64,
            c,
        )?);
    }
    inserted.sort_by_key(|c| c.position);
    Ok(PollWithCandidates {
        poll,
        candidates: inserted,
    })
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
    conn.execute(
        "UPDATE \"PollCandidate\" SET \"position\" = ?4, \"name\" = ?5, \"description\" = ?6, \"imageUrl\" = ?7, \
         \"updatedAt\" = ?8 WHERE \"id\" = ?1 AND \"pollId\" = ?2 AND \"workspaceId\" = ?3",
        rusqlite::params![
            id,
            poll_id,
            workspace_id,
            position,
            candidate.name,
            candidate.description,
            candidate.image_url,
            now_ms()
        ],
    )?;
    Ok(())
}

/// `insertCandidates` (`(position, candidate)` pairs).
pub fn insert_candidates(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    candidates: &[(i64, PollCandidateInput)],
) -> Result<Vec<PollCandidate>> {
    candidates
        .iter()
        .map(|(pos, c)| insert_candidate(conn, workspace_id, poll_id, *pos, c))
        .collect()
}

/// `deleteCandidates`.
pub fn delete_candidates(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"PollCandidate\" WHERE \"pollId\" = ?1 AND \"workspaceId\" = ?2 AND \"id\" IN (SELECT value FROM json_each(?3))",
        rusqlite::params![poll_id, workspace_id, id_list(ids)],
    )?;
    Ok(())
}

/// `update` (`data` columns: `name`, `imageUrl`, `startsAt`, `endsAt`, `afterVoteFlowId`).
pub fn update(conn: &Connection, workspace_id: i64, id: i64, data: &Patch) -> Result<Option<Poll>> {
    let mut all = data.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"Poll\" SET ");
    q.append(set_clause(Poll::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", Poll::columns_sql(None)));
    q.model(conn)
}

/// `deleteMany`: returns the deleted count.
pub fn delete_many(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(conn.execute(
        "DELETE FROM \"Poll\" WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, id_list(ids)],
    )?)
}

/// `upsertVote`: one vote per (poll, contact); a re-vote for the same candidate is
/// `unchanged` and reports when the current vote was cast.
pub fn upsert_vote(conn: &Connection, input: &VoteInput) -> Result<PollVoteOutcome> {
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT \"candidateId\", \"updatedAt\" FROM \"PollVote\" WHERE \"pollId\" = ?1 AND \"contactId\" = ?2",
            rusqlite::params![input.poll_id, input.contact_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let now = now_ms();
    match existing {
        None => {
            conn.execute(
                "INSERT INTO \"PollVote\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"pollId\", \"candidateId\", \
                 \"contactId\", \"contactInboxId\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    new_id(),
                    now,
                    input.workspace_id,
                    input.poll_id,
                    input.candidate_id,
                    input.contact_id,
                    input.contact_inbox_id
                ],
            )?;
            Ok(PollVoteOutcome {
                result: PollVoteResult::New,
                voted_at: Some(now),
            })
        }
        Some((candidate, voted_at)) if candidate == input.candidate_id => Ok(PollVoteOutcome {
            result: PollVoteResult::Unchanged,
            voted_at: Some(voted_at),
        }),
        Some(_) => {
            conn.execute(
                "UPDATE \"PollVote\" SET \"candidateId\" = ?3, \"contactInboxId\" = ?4, \"updatedAt\" = ?5 \
                 WHERE \"pollId\" = ?1 AND \"contactId\" = ?2",
                rusqlite::params![input.poll_id, input.contact_id, input.candidate_id, input.contact_inbox_id, now],
            )?;
            Ok(PollVoteOutcome {
                result: PollVoteResult::Changed,
                voted_at: Some(now),
            })
        }
    }
}

/// `voteCounts`: `(candidateId, votes)`.
pub fn vote_counts(conn: &Connection, workspace_id: i64, poll_id: i64) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"candidateId\", count(*) FROM \"PollVote\" WHERE \"workspaceId\" = ?1 AND \"pollId\" = ?2 GROUP BY \"candidateId\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, poll_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `voteBreakdown`: votes per candidate and contact gender (`unknown` for none).
pub fn vote_breakdown(
    conn: &Connection,
    workspace_id: i64,
    poll_id: i64,
) -> Result<Vec<VoteBreakdownRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT v.\"candidateId\", coalesce(c.\"gender\", 'unknown') AS g, count(*) FROM \"PollVote\" v \
         JOIN \"Contact\" c ON c.\"id\" = v.\"contactId\" WHERE v.\"workspaceId\" = ?1 AND v.\"pollId\" = ?2 \
         GROUP BY v.\"candidateId\", g",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, poll_id], |r| {
            Ok(VoteBreakdownRow {
                candidate_id: r.get(0)?,
                gender: r.get(1)?,
                votes: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
