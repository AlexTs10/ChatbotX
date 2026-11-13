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
    let found = tx::read(db, move |conn| {
        Ok(poll::find_with_candidates(
            conn,
            input.workspace_id,
            input.poll_id,
        )?)
    })
    .await?;
    let Some(found) = found else {
        return Ok(CastVoteOutcome::PollNotFound);
    };
    if get_poll_state(found.poll.starts_at, found.poll.ends_at, now) != PollState::Running {
        return Ok(CastVoteOutcome::NotRunning(found));
    }
    if !found.candidates.iter().any(|c| c.id == input.candidate_id) {
        return Ok(CastVoteOutcome::CandidateNotFound(found));
    }
    let flow_id = found.poll.after_vote_flow_id;
    let voted = tx::write(db, move |conn| {
        let vote = poll::upsert_vote(conn, &input)?;
        let after_vote_flow_id =
            resolve_after_vote_flow(conn, input.workspace_id, vote, flow_id, now)?;
        Ok((vote, after_vote_flow_id))
    })
    .await;
    match voted {
        Ok((vote, after_vote_flow_id)) => Ok(CastVoteOutcome::Voted {
            result: vote.result,
            after_vote_flow_job_id: match (after_vote_flow_id, vote.voted_at) {
                (Some(_), Some(voted_at)) => Some(format!(
                    "poll-vote-flow-{}-{}-{voted_at}",
                    input.poll_id, input.contact_id
                )),
                _ => None,
            },
            after_vote_flow_id,
        }),
        Err(BusinessError::Repo(error))
            if is_foreign_key_violation_error(&error, Some(CANDIDATE_FOREIGN_KEY)) =>
        {
            let fresh = tx::read(db, move |conn| {
                Ok(poll::find_with_candidates(
                    conn,
                    input.workspace_id,
                    input.poll_id,
                )?)
            })
            .await?;
            Ok(fresh.map_or(
                CastVoteOutcome::PollNotFound,
                CastVoteOutcome::CandidateNotFound,
            ))
        }
        Err(error) => Err(error),
    }
}
