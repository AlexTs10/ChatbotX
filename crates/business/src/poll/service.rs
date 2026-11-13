//! `poll/service.ts` (`pollService`).

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Poll, PollCandidate};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::poll::{
    self, PollCandidateInput, PollInsertInput, PollPage, PollWithCandidates,
};
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::rusqlite::{Connection, OptionalExtension};

use super::analytics::{AnalyticsCandidate, PollAnalytics, compute_poll_analytics};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `POLL_MIN_CANDIDATES`.
pub const POLL_MIN_CANDIDATES: usize = 2;
/// `POLL_MAX_CANDIDATES`.
pub const POLL_MAX_CANDIDATES: usize = 10;

/// Input of [`list`] (`page` 1 and `per_page` 10 when not sent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPolls {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// A candidate with its vote count.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateWithVotes {
    pub candidate: PollCandidate,
    pub votes_count: i64,
}

/// A poll whose candidates carry vote counts.
#[derive(Debug, Clone, PartialEq)]
pub struct PollWithVoteCounts {
    pub poll: Poll,
    pub candidates: Vec<CandidateWithVotes>,
}

/// Input of [`create`] (`PollCreateInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePoll {
    pub workspace_id: i64,
    pub name: String,
    pub image_url: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub after_vote_flow_id: Option<i64>,
    pub candidates: Vec<PollCandidateInput>,
}

/// A candidate of [`UpdatePoll`]: `id` keeps an existing one, `None` adds one.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdatePollCandidate {
    pub id: Option<i64>,
    pub candidate: PollCandidateInput,
}

/// Input of [`update`] (`PollUpdateData`; `None` = not sent,
/// `after_vote_flow_id: Some(None)` clears the flow).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdatePoll {
    pub workspace_id: i64,
    pub id: i64,
    pub name: Option<String>,
    pub image_url: Option<String>,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub after_vote_flow_id: Option<Option<i64>>,
    pub candidates: Option<Vec<UpdatePollCandidate>>,
}

fn poll_not_found() -> BusinessError {
    BusinessError::not_found("Poll not found")
}

fn assert_date_range(starts_at: i64, ends_at: i64) -> Result<()> {
    if ends_at <= starts_at {
        return Err(BusinessError::exception(
            "Poll end must be after its start",
            "pollInvalidDateRange",
            400,
        ));
    }
    Ok(())
}

fn assert_candidate_count(count: usize) -> Result<()> {
    if !(POLL_MIN_CANDIDATES..=POLL_MAX_CANDIDATES).contains(&count) {
        return Err(BusinessError::exception(
            format!(
                "A poll needs between {POLL_MIN_CANDIDATES} and {POLL_MAX_CANDIDATES} candidates"
            ),
            "pollCandidateCount",
            400,
        ));
    }
    Ok(())
}

/// `flowRepository.existsInWorkspace` (private query; moves to the flow port).
pub(crate) fn flow_exists_in_workspace(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM \"Flow\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 LIMIT 1",
            [flow_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok(found.is_some())
}

fn assert_flow_in_workspace(
    conn: &Connection,
    workspace_id: i64,
    flow_id: Option<i64>,
) -> Result<()> {
    let Some(flow_id) = flow_id else {
        return Ok(());
    };
    if !flow_exists_in_workspace(conn, workspace_id, flow_id)? {
        return Err(BusinessError::exception(
            "Flow not found",
            "pollFlowNotFound",
            400,
        ));
    }
    Ok(())
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<PollWithCandidates> {
    poll::find_with_candidates(conn, workspace_id, id)?.ok_or_else(poll_not_found)
}

fn attach_vote_counts(conn: &Connection, found: PollWithCandidates) -> Result<PollWithVoteCounts> {
    let counts = poll::vote_counts(conn, found.poll.workspace_id, found.poll.id)?;
    let candidates = found
        .candidates
        .into_iter()
        .map(|candidate| {
            let votes_count = counts
                .iter()
                .find(|(id, _)| *id == candidate.id)
                .map_or(0, |(_, votes)| *votes);
            CandidateWithVotes {
                candidate,
                votes_count,
            }
        })
        .collect();
    Ok(PollWithVoteCounts {
        poll: found.poll,
        candidates,
    })
}

/// `pollService.list`: newest first with vote counts.
pub async fn list(db: &Db, input: ListPolls) -> Result<PollPage> {
    todo!("P3.6-D1")
}

/// `pollService.find`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<PollWithCandidates> {
    todo!("P3.6-D1")
}

/// `pollService.findWithVoteCounts`.
pub async fn find_with_vote_counts(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<PollWithVoteCounts> {
    todo!("P3.6-D1")
}

/// `pollService.withVoteCounts`: each candidate's vote count (0 when none).
pub async fn with_vote_counts(db: &Db, found: PollWithCandidates) -> Result<PollWithVoteCounts> {
    todo!("P3.6-D1")
}

/// `pollService.create`: 2–10 candidates, `endsAt` after `startsAt`, an
/// after-vote flow of this workspace.
pub async fn create(db: &Db, input: CreatePoll) -> Result<PollWithCandidates> {
    todo!("P3.6-D1")
}

/// `pollService.update`: sent candidates with an id are kept (in the sent
/// order), new ones are added, the rest are deleted; only sent fields change.
pub async fn update(db: &Db, input: UpdatePoll) -> Result<PollWithCandidates> {
    todo!("P3.6-D1")
}

/// `pollService.delete`.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    todo!("P3.6-D1")
}

/// `pollService.bulkDelete`: how many of `ids` were this workspace's polls.
pub async fn bulk_delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<usize> {
    todo!("P3.6-D1")
}

/// `pollService.getAnalytics`: the poll and its vote analytics.
pub async fn get_analytics(db: &Db, workspace_id: i64, id: i64) -> Result<(Poll, PollAnalytics)> {
    todo!("P3.6-D1")
}
