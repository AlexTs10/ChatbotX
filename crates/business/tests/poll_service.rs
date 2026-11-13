//! Port of `packages/business/__tests__/poll-service.test.ts`.
//! Adapted: the mocked repositories become real rows of a migrated SQLite file;
//! "the repository was (not) called with" becomes the stored rows. The
//! template-lock test is not ported here (template unit).

mod common;

use chatbotx_business::poll::service::{self, CreatePoll, UpdatePoll, UpdatePollCandidate};
use chatbotx_db::repo::poll::{PollCandidateInput, PollWithCandidates, VoteInput, upsert_vote};
use common::{OTHER_WS, WS, contact, setup, two_workspaces};

const STARTS: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z
const ENDS: i64 = 1_767_312_000_000; // 2026-01-02T00:00:00Z

fn candidate(name: &str) -> PollCandidateInput {
    PollCandidateInput {
        name: name.to_string(),
        description: None,
        image_url: None,
    }
}

fn cands(n: usize) -> Vec<PollCandidateInput> {
    (0..n).map(|i| candidate(&format!("c{i}"))).collect()
}

fn base(workspace_id: i64, candidates: Vec<PollCandidateInput>) -> CreatePoll {
    CreatePoll {
        workspace_id,
        name: "Poll".to_string(),
        image_url: "https://x/y.png".to_string(),
        starts_at: STARTS,
        ends_at: ENDS,
        after_vote_flow_id: None,
        candidates,
    }
}

/// The `existing` poll of the update tests: candidates `a` (0) and `b` (1).
async fn existing(db: &chatbotx_db::Db) -> PollWithCandidates {
    service::create(db, base(WS, vec![candidate("a"), candidate("b")]))
        .await
        .expect("create")
}

fn update_input(id: i64) -> UpdatePoll {
    UpdatePoll {
        workspace_id: WS,
        id,
        ..UpdatePoll::default()
    }
}

#[tokio::test]
async fn poll_service_rejects_1_candidate() {
    let db = two_workspaces().await;
    let error = service::create(&db, base(WS, cands(1)))
        .await
        .expect_err("1");
    assert_eq!(error.code(), Some("pollCandidateCount"));
}

#[tokio::test]
async fn poll_service_rejects_11_candidates() {
    let db = two_workspaces().await;
    let error = service::create(&db, base(WS, cands(11)))
        .await
        .expect_err("11");
    assert_eq!(error.code(), Some("pollCandidateCount"));
}

#[tokio::test]
async fn poll_service_rejects_ends_at_equal_starts_at() {
    let db = two_workspaces().await;
    let error = service::create(
        &db,
        CreatePoll {
            ends_at: STARTS,
            ..base(WS, cands(2))
        },
    )
    .await
    .expect_err("range");
    assert_eq!(error.code(), Some("pollInvalidDateRange"));
}

#[tokio::test]
async fn poll_service_rejects_an_unknown_after_vote_flow_id() {
    let db = two_workspaces().await;
    let error = service::create(
        &db,
        CreatePoll {
            after_vote_flow_id: Some(9),
            ..base(WS, cands(2))
        },
    )
    .await
    .expect_err("flow");
    assert_eq!(error.code(), Some("pollFlowNotFound"));
}

#[tokio::test]
async fn poll_service_create_inserts_poll_with_candidates() {
    let db = two_workspaces().await;
    let created = service::create(&db, base(WS, cands(2)))
        .await
        .expect("create");
    let found = service::find(&db, WS, created.poll.id).await.expect("find");
    assert_eq!(found.candidates.len(), 2);
    assert_eq!(found, created);
}

#[tokio::test]
async fn poll_service_create_runs_inside_a_transaction_and_passes_the_tx_to_the_repository() {
    let db = two_workspaces().await;
    let created = service::create(&db, base(WS, cands(2)))
        .await
        .expect("create");
    assert_eq!(created.poll.workspace_id, WS);
    assert!(
        created
            .candidates
            .iter()
            .all(|c| c.workspace_id == WS && c.poll_id == created.poll.id)
    );
}

#[tokio::test]
async fn poll_service_update_keeps_c1_deletes_c2_inserts_the_new_one() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let (c1, c2) = (poll.candidates[0].id, poll.candidates[1].id);
    let updated = service::update(
        &db,
        UpdatePoll {
            candidates: Some(vec![
                UpdatePollCandidate {
                    id: Some(c1),
                    candidate: candidate("a2"),
                },
                UpdatePollCandidate {
                    id: None,
                    candidate: candidate("new"),
                },
            ]),
            ..update_input(poll.poll.id)
        },
    )
    .await
    .expect("update");
    let names: Vec<(i64, &str)> = updated
        .candidates
        .iter()
        .map(|c| (c.position, c.name.as_str()))
        .collect();
    assert_eq!(names, vec![(0, "a2"), (1, "new")]);
    assert_eq!(updated.candidates[0].id, c1);
    assert!(updated.candidates.iter().all(|c| c.id != c2));
}

#[tokio::test]
async fn poll_service_update_ignores_keys_set_to_undefined_no_empty_set() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let updated = service::update(&db, update_input(poll.poll.id))
        .await
        .expect("update");
    assert_eq!(updated, poll);
}

#[tokio::test]
async fn poll_service_update_passes_only_defined_fields_to_the_repository() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let updated = service::update(
        &db,
        UpdatePoll {
            name: Some("New".to_string()),
            ..update_input(poll.poll.id)
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.poll.name, "New");
    assert_eq!(
        (
            updated.poll.image_url.as_str(),
            updated.poll.starts_at,
            updated.poll.ends_at
        ),
        (
            poll.poll.image_url.as_str(),
            poll.poll.starts_at,
            poll.poll.ends_at
        )
    );
    assert_eq!(updated.candidates, poll.candidates);
}

#[tokio::test]
async fn poll_service_update_rejects_a_candidate_id_from_another_poll() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let other = existing(&db).await;
    let error = service::update(
        &db,
        UpdatePoll {
            candidates: Some(vec![
                UpdatePollCandidate {
                    id: Some(other.candidates[0].id),
                    candidate: candidate("a"),
                },
                UpdatePollCandidate {
                    id: None,
                    candidate: candidate("b"),
                },
            ]),
            ..update_input(poll.poll.id)
        },
    )
    .await
    .expect_err("foreign candidate");
    assert_eq!(error.code(), Some("pollCandidateNotInPoll"));
}

#[tokio::test]
async fn poll_service_update_validates_the_merged_date_range() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let error = service::update(
        &db,
        UpdatePoll {
            ends_at: Some(1_735_689_600_000), // 2025-01-01T00:00:00Z
            ..update_input(poll.poll.id)
        },
    )
    .await
    .expect_err("range");
    assert_eq!(error.code(), Some("pollInvalidDateRange"));
}

#[tokio::test]
async fn poll_service_update_404_when_the_poll_is_missing() {
    let db = two_workspaces().await;
    let error = service::update(&db, update_input(999))
        .await
        .expect_err("missing");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn poll_service_bulk_delete_returns_the_deleted_count() {
    let db = two_workspaces().await;
    let a = existing(&db).await.poll.id;
    let b = existing(&db).await.poll.id;
    let foreign = service::create(&db, base(OTHER_WS, cands(2)))
        .await
        .expect("create")
        .poll
        .id;
    let deleted = service::bulk_delete(&db, WS, vec![a, b, foreign])
        .await
        .expect("bulk");
    assert_eq!(deleted, 2);
    assert!(service::find(&db, OTHER_WS, foreign).await.is_ok());
}

#[tokio::test]
async fn poll_service_find_with_vote_counts_attaches_each_candidate_s_vote_count() {
    let db = two_workspaces().await;
    let poll = existing(&db).await;
    let (poll_id, c1) = (poll.poll.id, poll.candidates[0].id);
    setup(&db, move |tx| {
        for contact_id in 0..5_i64 {
            contact(tx, 5000 + contact_id, WS)?;
            upsert_vote(
                tx,
                &VoteInput {
                    workspace_id: WS,
                    poll_id,
                    candidate_id: c1,
                    contact_id: 5000 + contact_id,
                    contact_inbox_id: None,
                },
            )
            .map_err(|e| chatbotx_db::DbError::other(e.to_string()))?;
        }
        Ok(())
    })
    .await;
    let counted = service::find_with_vote_counts(&db, WS, poll_id)
        .await
        .expect("counts");
    let counts: Vec<(i64, i64)> = counted
        .candidates
        .iter()
        .map(|c| (c.candidate.id, c.votes_count))
        .collect();
    assert_eq!(counts, vec![(c1, 5), (poll.candidates[1].id, 0)]);
}
