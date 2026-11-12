//! Port of `packages/database/__tests__/integration/poll-vote-upsert.test.ts` (P3.3).
//! Two Postgres connections -> two concurrent `repo::write` jobs (the one SQLite writer
//! serializes them; the asserted outcome is the same).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo;
use chatbotx_db::repo::poll::{
    PollCandidateInput, PollInsertInput, VoteInput, insert_with_candidates, upsert_vote,
};
use common::{fresh_db, seed};
use repo_common::{NOW, ok, setup};

const ROUNDS: i64 = 10;
const WS: i64 = 2001;

async fn seeded() -> (common::TestDb, i64, i64, Vec<i64>) {
    let db = fresh_db().await;
    seed(&db).await;
    let poll = ok(repo::write(&db, |tx| {
        insert_with_candidates(
            tx,
            &PollInsertInput {
                workspace_id: WS,
                name: "poll-vote".to_owned(),
                image_url: "https://example.test/p.png".to_owned(),
                starts_at: NOW - 60_000,
                ends_at: NOW + 3_600_000,
                after_vote_flow_id: None,
            },
            &[
                PollCandidateInput {
                    name: "a".to_owned(),
                    description: None,
                    image_url: None,
                },
                PollCandidateInput {
                    name: "b".to_owned(),
                    description: None,
                    image_url: None,
                },
            ],
        )
    })
    .await);
    let contacts: Vec<i64> = (0..ROUNDS).map(|i| 7000 + i).collect();
    let ids = contacts.clone();
    setup(&db, move |tx| {
        for id in ids {
            ins!(tx, "Contact", { "id" => id, "workspaceId" => WS })?;
        }
        Ok(())
    })
    .await;
    (db, poll.poll.id, poll.candidates[0].id, contacts)
}

/// Port of: packages/database/__tests__/integration/poll-vote-upsert.test.ts:111 "pollRepository.upsertVote concurrency > two parallel votes for the same candidate yield one new and one unchanged"
#[tokio::test]
async fn poll_repository_upsert_vote_concurrency_two_parallel_votes_for_the_same_candidate_yield_one_new_and_one_unchanged()
 {
    let (db, poll_id, candidate_id, contacts) = seeded().await;
    for contact_id in contacts {
        let input = VoteInput {
            workspace_id: WS,
            poll_id,
            candidate_id,
            contact_id,
            contact_inbox_id: None,
        };
        let (a, b) = tokio::join!(
            repo::write(&db, move |tx| upsert_vote(tx, &input)),
            repo::write(&db, move |tx| upsert_vote(tx, &input)),
        );
        let mut results = vec![ok(a).result.as_str(), ok(b).result.as_str()];
        results.sort_unstable();
        assert_eq!(results, vec!["new", "unchanged"]);
    }
}

/// Port of: packages/database/__tests__/integration/poll-vote-upsert.test.ts:122 "pollRepository.upsertVote concurrency > an unchanged re-vote reports when the current vote was cast"
#[tokio::test]
async fn poll_repository_upsert_vote_concurrency_an_unchanged_re_vote_reports_when_the_current_vote_was_cast()
 {
    let (db, poll_id, candidate_id, contacts) = seeded().await;
    let input = VoteInput {
        workspace_id: WS,
        poll_id,
        candidate_id,
        contact_id: contacts[0],
        contact_inbox_id: None,
    };
    let first = ok(repo::write(&db, move |tx| upsert_vote(tx, &input)).await);
    let again = ok(repo::write(&db, move |tx| upsert_vote(tx, &input)).await);
    assert_eq!(again.result.as_str(), "unchanged");
    assert!(first.voted_at.is_some());
    assert_eq!(again.voted_at, first.voted_at);
}
