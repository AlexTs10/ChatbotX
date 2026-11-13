//! `poll/vote.service.ts` (`pollVoteService.castVote`).

use chatbotx_db::Db;
use chatbotx_db::repo::client::is_foreign_key_violation_error;
use chatbotx_db::repo::poll::{
    self, PollVoteOutcome, PollVoteResult, PollWithCandidates, VoteInput,
};

use super::service::flow_exists_in_workspace;
use super::state::{PollState, get_poll_state};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// A changed-nothing re-vote re-offers the after-vote flow this long after the
/// vote (a retried job), not later.
const AFTER_VOTE_RETRY_WINDOW_MS: i64 = 2 * 60_000;
const CANDIDATE_FOREIGN_KEY: &str = "PollVote_candidateId_PollCandidate_id_fkey";

/// `CastVoteOutcome`.
#[derive(Debug, Clone, PartialEq)]
pub enum CastVoteOutcome {
    Voted {
        result: PollVoteResult,
        after_vote_flow_id: Option<i64>,
        after_vote_flow_job_id: Option<String>,
    },
    PollNotFound,
    CandidateNotFound(PollWithCandidates),
    NotRunning(PollWithCandidates),
}

fn resolve_after_vote_flow(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    vote: PollVoteOutcome,
    flow_id: Option<i64>,
    now: i64,
) -> Result<Option<i64>> {
    let (Some(flow_id), Some(voted_at)) = (flow_id, vote.voted_at) else {
        return Ok(None);
    };
    if vote.result == PollVoteResult::Unchanged && now - voted_at > AFTER_VOTE_RETRY_WINDOW_MS {
        return Ok(None);
    }
    Ok(flow_exists_in_workspace(conn, workspace_id, flow_id)?.then_some(flow_id))
}

/// `pollVoteService.castVote` at time `now` (epoch ms).
pub async fn cast_vote(db: &Db, input: VoteInput, now: i64) -> Result<CastVoteOutcome> {
    todo!("P3.6-D1")
}
