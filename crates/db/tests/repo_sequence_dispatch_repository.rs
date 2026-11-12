//! Port of `packages/database/__tests__/sequence-dispatch-repository.test.ts` (P3.3). The
//! mocked query builder becomes real `SequenceDispatch` rows (with their sequence, step,
//! contact and enrollment): asserted `where` / `with` / CAS predicates are checked as the
//! rows returned or changed; "no query" = empty result; the CTE text = its effect (only
//! old terminal rows are deleted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::models::SequenceDispatch;
use chatbotx_db::repo::sequence_dispatch::{
    claim, delete_terminal_batch, find_step_with_flow, find_with_relations,
    list_pending_for_reconcile, list_pending_ids, list_pending_workspace_ids,
};
use chatbotx_db::repo::{self, find_one, now_ms};
use common::{fresh_db, insert_contact_inbox, seed, seed_workspace};
use repo_common::{NOW, ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const DAY: i64 = 86_400_000;

/// Sequence 71 (steps 81 with flow 61, 82 without), enrollment 91 of contact 4001 on
/// contact inbox 4101; dispatches: 1 pending (runAt 1000), 2 pending (runAt 3000),
/// 3 running, 4 completed long ago, 5 failed recently.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    let s = seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, move |tx| {
        repo3::flow(tx, 61, WS)?;
        insert_contact_inbox(tx, s, 4101, "psid-1", None, None)?;
        ins!(tx, "Sequence", { "id" => 71_i64, "name" => "Onboarding", "workspaceId" => WS })?;
        ins!(tx, "SequenceStep", { "id" => 81_i64, "order" => 1_i64, "delayDays" => 0_i64, "sequenceId" => 71_i64, "flowId" => 61_i64 })?;
        ins!(tx, "SequenceStep", { "id" => 82_i64, "order" => 2_i64, "delayDays" => 1_i64, "sequenceId" => 71_i64 })?;
        ins!(tx, "ContactOnSequence", { "id" => 91_i64, "contactId" => s.contact_id, "sequenceId" => 71_i64, "workspaceId" => WS })?;
        for (id, status, run_at, updated_at) in [
            (1_i64, "pending", 1_000_i64, NOW),
            (2, "pending", 3_000, NOW),
            (3, "running", 2_000, NOW),
            (4, "completed", 500, NOW - 40 * DAY),
            (5, "failed", 500, NOW - DAY),
        ] {
            ins!(tx, "SequenceDispatch", {
                "id" => id, "updatedAt" => updated_at, "runAtMs" => run_at, "status" => status,
                "idempotencyKey" => format!("key-{id}"), "workspaceId" => WS, "sequenceId" => 71_i64,
                "contactId" => s.contact_id, "contactInboxId" => 4101_i64, "stepId" => 81_i64,
                "enrollmentId" => 91_i64,
            })?;
        }
        Ok(())
    })
    .await;
    db
}

async fn dispatch(db: &common::TestDb, id: i64) -> Option<SequenceDispatch> {
    ok(repo::read(db, move |c| find_one(c, "\"id\" = ?1", &[&id])).await)
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:60 "findWithRelations > scopes by id, status, and workspaceId with sequence/contact/enrollment relations"
#[tokio::test]
async fn find_with_relations_scopes_by_id_status_and_workspace_id_with_sequence_contact_enrollment_relations()
 {
    let db = seeded().await;
    let found =
        ok(repo::read(&db, |c| find_with_relations(c, 1, "pending", WS)).await).expect("dispatch");
    assert_eq!(found.dispatch.id, 1);
    assert_eq!(found.sequence.map(|s| s.id), Some(71));
    assert_eq!(found.contact.map(|c| c.id), Some(4001));
    assert_eq!(found.enrollment.map(|e| e.id), Some(91));
    // Wrong status or workspace: not found.
    assert_eq!(
        ok(repo::read(&db, |c| find_with_relations(c, 3, "pending", WS)).await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_with_relations(c, 1, "pending", OTHER)).await),
        None
    );
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:77 "findWithRelations > returns null when no row matches"
#[tokio::test]
async fn find_with_relations_returns_null_when_no_row_matches() {
    let db = seeded().await;
    assert_eq!(
        ok(repo::read(&db, |c| find_with_relations(c, 999, "pending", WS)).await),
        None
    );
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:90 "claim > issues a status='pending' CAS update and returns true when a row was affected"
#[tokio::test]
async fn claim_issues_a_status_pending_cas_update_and_returns_true_when_a_row_was_affected() {
    let db = seeded().await;
    // The workspace is part of the guard.
    assert!(!ok(
        repo::write(&db, |tx| claim(tx, 1, OTHER, "host-1")).await
    ));
    let before = now_ms();
    assert!(ok(repo::write(&db, |tx| claim(tx, 1, WS, "host-1")).await));
    let after = now_ms();
    let row = dispatch(&db, 1).await.expect("row");
    assert_eq!(row.status, "running");
    assert_eq!(row.lock_owner.as_deref(), Some("host-1"));
    let locked_at = row.locked_at.expect("lockedAt");
    assert!(locked_at >= before && locked_at <= after);
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:125 "claim > returns false when the row was no longer pending (lost the race)"
#[tokio::test]
async fn claim_returns_false_when_the_row_was_no_longer_pending_lost_the_race() {
    let db = seeded().await;
    assert!(ok(repo::write(&db, |tx| claim(tx, 1, WS, "host-1")).await));
    assert!(!ok(repo::write(&db, |tx| claim(tx, 1, WS, "host-2")).await));
    assert_eq!(
        dispatch(&db, 1).await.and_then(|d| d.lock_owner).as_deref(),
        Some("host-1")
    );
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:142 "listPendingWorkspaceIds > returns [] without querying when ids is empty"
#[tokio::test]
async fn list_pending_workspace_ids_returns_without_querying_when_ids_is_empty() {
    let db = seeded().await;
    assert!(ok(repo::read(&db, |c| list_pending_workspace_ids(c, &[])).await).is_empty());
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:150 "listPendingWorkspaceIds > scopes to the given ids and status=pending"
#[tokio::test]
async fn list_pending_workspace_ids_scopes_to_the_given_ids_and_status_pending() {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| list_pending_workspace_ids(c, &[1, 3])).await);
    assert_eq!(rows, vec![(1, WS)]);
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:166 "listPendingForReconcile > pages by runAtMs ascending, scoped to status=pending"
#[tokio::test]
async fn list_pending_for_reconcile_pages_by_run_at_ms_ascending_scoped_to_status_pending() {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| list_pending_for_reconcile(c, 1_000, 0, 1_000)).await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
    let all = ok(repo::read(&db, |c| list_pending_for_reconcile(c, 10_000, 0, 1_000)).await);
    assert_eq!(
        all.iter().map(|r| (r.id, r.run_at_ms)).collect::<Vec<_>>(),
        vec![(1, 1_000), (2, 3_000)]
    );
    let paged = ok(repo::read(&db, |c| list_pending_for_reconcile(c, 10_000, 1, 1)).await);
    assert_eq!(paged.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2]);
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:186 "listPendingIds > returns [] without querying when ids is empty"
#[tokio::test]
async fn list_pending_ids_returns_without_querying_when_ids_is_empty() {
    let db = seeded().await;
    assert!(ok(repo::read(&db, |c| list_pending_ids(c, &[])).await).is_empty());
    assert_eq!(
        ok(repo::read(&db, |c| list_pending_ids(c, &[2, 4])).await),
        vec![2]
    );
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:196 "deleteTerminalBatch > deletes terminal rows via the CTE joined on workspaceId for partition pruning"
#[tokio::test]
async fn delete_terminal_batch_deletes_terminal_rows_via_the_cte_joined_on_workspace_id_for_partition_pruning()
 {
    let db = seeded().await;
    // Only the completed row older than 30 days goes; the recent failed row stays.
    let deleted = ok(repo::write(&db, |tx| delete_terminal_batch(tx, 30, 1_000, NOW)).await);
    assert_eq!(deleted, 1);
    assert_eq!(dispatch(&db, 4).await, None);
    assert!(dispatch(&db, 5).await.is_some());
    assert!(dispatch(&db, 1).await.is_some());
}

/// Port of: packages/database/__tests__/sequence-dispatch-repository.test.ts:226 "findStepWithFlow > delegates to sequenceStepModel with the flow relation"
#[tokio::test]
async fn find_step_with_flow_delegates_to_sequence_step_model_with_the_flow_relation() {
    let db = seeded().await;
    let step = ok(repo::read(&db, |c| find_step_with_flow(c, 81)).await).expect("step");
    assert_eq!(step.step.id, 81);
    assert_eq!(step.flow.map(|f| f.id), Some(61));
    let without = ok(repo::read(&db, |c| find_step_with_flow(c, 82)).await).expect("step");
    assert_eq!(without.flow, None);
}
