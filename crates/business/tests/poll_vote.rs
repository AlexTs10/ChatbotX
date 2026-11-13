//! Port of `packages/business/__tests__/poll-vote.test.ts`.
//! Adapted: the mocked poll / vote repositories become real rows; an
//! "unchanged" vote is a stored vote for the same candidate (its `updatedAt`
//! is the vote time). Not ported (mock-only races): the null `votedAt` race,
//! the candidate foreign-key races, and an arbitrary upsert error.

mod common;

use chatbotx_business::poll::service::{self, CreatePoll};
use chatbotx_business::poll::vote::{CastVoteOutcome, cast_vote};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::poll::{PollCandidateInput, PollVoteResult, PollWithCandidates, VoteInput};
use chatbotx_db::rusqlite::params;
use common::{WS, contact, scalar_i64, setup, two_workspaces};

const CONTACT: i64 = 4001;
const FLOW: i64 = 7001;
const COUNT_VOTES: &str = r#"SELECT count(*) FROM "PollVote" WHERE "pollId" = ?1"#;

fn candidate(name: &str) -> PollCandidateInput {
    PollCandidateInput {
        name: name.to_string(),
        description: None,
        image_url: None,
    }
}

/// A poll running now (±60 s) with candidates c1, c2; `flow` = after-vote flow.
async fn running(
    db: &chatbotx_db::Db,
    flow: bool,
    starts_offset: i64,
    ends_offset: i64,
) -> PollWithCandidates {
    setup(db, move |tx| {
        contact(tx, CONTACT, WS)?;
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (?1, 'After vote', ?2)"#,
            params![FLOW, WS],
        )?;
        Ok(())
    })
    .await;
    let now = now_ms();
    service::create(
        db,
        CreatePoll {
            workspace_id: WS,
            name: "Poll".to_string(),
            image_url: "https://x/y.png".to_string(),
            starts_at: now + starts_offset,
            ends_at: now + ends_offset,
            after_vote_flow_id: flow.then_some(FLOW),
            candidates: vec![candidate("c1"), candidate("c2")],
        },
    )
    .await
    .expect("poll")
}

fn input(poll: &PollWithCandidates) -> VoteInput {
    VoteInput {
        workspace_id: WS,
        poll_id: poll.poll.id,
        candidate_id: poll.candidates[0].id,
        contact_id: CONTACT,
        contact_inbox_id: None,
    }
}

/// Casts the first vote and moves its time `age_ms` into the past; returns its time.
async fn prior_vote(db: &chatbotx_db::Db, poll: &PollWithCandidates, age_ms: i64) -> i64 {
    cast_vote(db, input(poll), now_ms())
        .await
        .expect("first vote");
    let voted_at = now_ms() - age_ms;
    let poll_id = poll.poll.id;
    setup(db, move |tx| {
        tx.execute(
            r#"UPDATE "PollVote" SET "updatedAt" = ?1 WHERE "pollId" = ?2"#,
            params![voted_at, poll_id],
        )?;
        Ok(())
    })
    .await;
    voted_at
}

fn job_id(poll: &PollWithCandidates, voted_at: i64) -> String {
    format!("poll-vote-flow-{}-{CONTACT}-{voted_at}", poll.poll.id)
}

#[tokio::test]
async fn poll_vote_service_cast_vote_passes_through_unchanged() {
    let db = two_workspaces().await;
    let poll = running(&db, false, -60_000, 60_000).await;
    prior_vote(&db, &poll, 0).await;
    let outcome = cast_vote(&db, input(&poll), now_ms()).await.expect("vote");
    assert_eq!(
        outcome,
        CastVoteOutcome::Voted {
            result: PollVoteResult::Unchanged,
            after_vote_flow_id: None,
            after_vote_flow_job_id: None,
        }
    );
    assert_eq!(scalar_i64(&db, COUNT_VOTES, poll.poll.id).await, Some(1));
}

#[tokio::test]
async fn poll_vote_service_cast_vote_unchanged_vote_right_after_the_vote_re_offers_the_flow_job_retry()
 {
    let db = two_workspaces().await;
    let poll = running(&db, true, -60_000, 60_000).await;
    let voted_at = prior_vote(&db, &poll, 0).await;
    let outcome = cast_vote(&db, input(&poll), now_ms()).await.expect("vote");
    assert_eq!(
        outcome,
        CastVoteOutcome::Voted {
            result: PollVoteResult::Unchanged,
            after_vote_flow_id: Some(FLOW),
            after_vote_flow_job_id: Some(job_id(&poll, voted_at)),
        }
    );
}

#[tokio::test]
async fn poll_vote_service_cast_vote_unchanged_vote_long_after_the_vote_does_not_rerun_the_flow() {
    let db = two_workspaces().await;
    let poll = running(&db, true, -60_000, 60_000).await;
    prior_vote(&db, &poll, 10 * 60_000).await;
    let outcome = cast_vote(&db, input(&poll), now_ms()).await.expect("vote");
    assert_eq!(
        outcome,
        CastVoteOutcome::Voted {
            result: PollVoteResult::Unchanged,
            after_vote_flow_id: None,
            after_vote_flow_job_id: None,
        }
    );
}

#[tokio::test]
async fn poll_vote_service_cast_vote_new_vote_returns_the_after_vote_flow_when_it_still_exists() {
    let db = two_workspaces().await;
    let poll = running(&db, true, -60_000, 60_000).await;
    let outcome = cast_vote(&db, input(&poll), now_ms()).await.expect("vote");
    let CastVoteOutcome::Voted {
        result,
        after_vote_flow_id,
        after_vote_flow_job_id,
    } = outcome
    else {
        panic!("expected a vote, got {outcome:?}");
    };
    assert_eq!(
        (result, after_vote_flow_id),
        (PollVoteResult::New, Some(FLOW))
    );
    let prefix = format!("poll-vote-flow-{}-{CONTACT}-", poll.poll.id);
    assert!(after_vote_flow_job_id.is_some_and(|id| id.starts_with(&prefix)));
}

#[tokio::test]
async fn poll_vote_service_cast_vote_deleted_after_vote_flow_resolves_to_null() {
    let db = two_workspaces().await;
    let poll = running(&db, true, -60_000, 60_000).await;
    prior_vote(&db, &poll, 0).await;
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Flow" WHERE "id" = ?1"#, params![FLOW])?;
        Ok(())
    })
    .await;
    let changed = VoteInput {
        candidate_id: poll.candidates[1].id,
        ..input(&poll)
    };
    let outcome = cast_vote(&db, changed, now_ms()).await.expect("vote");
    assert_eq!(
        outcome,
        CastVoteOutcome::Voted {
            result: PollVoteResult::Changed,
            after_vote_flow_id: None,
            after_vote_flow_job_id: None,
        }
    );
}

#[tokio::test]
async fn poll_vote_service_cast_vote_ended_poll_is_rejected_without_upsert() {
    let db = two_workspaces().await;
    let poll = running(&db, false, -120_000, -60_000).await;
    let outcome = cast_vote(&db, input(&poll), now_ms()).await.expect("vote");
    assert_eq!(outcome, CastVoteOutcome::NotRunning(poll.clone()));
    assert_eq!(scalar_i64(&db, COUNT_VOTES, poll.poll.id).await, Some(0));
}

#[tokio::test]
async fn poll_vote_service_cast_vote_candidate_from_another_poll_is_rejected() {
    let db = two_workspaces().await;
    let poll = running(&db, false, -60_000, 60_000).await;
    let other = VoteInput {
        candidate_id: 999,
        ..input(&poll)
    };
    let outcome = cast_vote(&db, other, now_ms()).await.expect("vote");
    assert_eq!(outcome, CastVoteOutcome::CandidateNotFound(poll.clone()));
    assert_eq!(scalar_i64(&db, COUNT_VOTES, poll.poll.id).await, Some(0));
}

#[tokio::test]
async fn poll_vote_service_cast_vote_missing_poll_is_rejected() {
    let db = two_workspaces().await;
    let poll = running(&db, false, -60_000, 60_000).await;
    let missing = VoteInput {
        poll_id: 999,
        ..input(&poll)
    };
    let outcome = cast_vote(&db, missing, now_ms()).await.expect("vote");
    assert_eq!(outcome, CastVoteOutcome::PollNotFound);
}

#[tokio::test]
async fn poll_vote_service_cast_vote_other_fk_violations_e_g_a_deleted_contact_propagate() {
    let db = two_workspaces().await;
    let poll = running(&db, false, -60_000, 60_000).await;
    let ghost = VoteInput {
        contact_id: 999,
        ..input(&poll)
    };
    let error = cast_vote(&db, ghost, now_ms())
        .await
        .expect_err("FK violation");
    assert_eq!(error.code(), None, "{error}");
}
