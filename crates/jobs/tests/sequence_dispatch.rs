//! Port of `packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts`
//! (spec 5.2): `sequenceDispatchUtils.bulkCancelPendingDispatches`. Seeded
//! ids are integers (P2.4 DDL; mapping table in `support/sequence.rs`).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use std::sync::{Arc, Mutex};

use chatbotx_db::rusqlite::Connection;
use chatbotx_jobs::sequence::cancel::{
    DispatchRef, PendingDispatch, PendingDispatchRepo, SqlPendingDispatchRepo,
    bulk_cancel_pending_dispatches_in,
};
use chatbotx_jobs::sequence::repo::{SequenceError, StatementKind, TxObserver};
use serde_json::json;

use sequence::{
    D_1, D_2, D_MATCH, D_N, D_OTHER_ENROLLMENT, D_OTHER_WS, D_RUNNING, ENROLL_1, ENROLL_1_WS_2,
    Harness, MY_ENROLLMENT, MY_ENROLLMENT_OTHER_WS, MY_WORKSPACE, OTHER_ENROLLMENT,
    OTHER_WORKSPACE, WS_1, WS_2, id,
};

/// Runs `bulk_cancel_pending_dispatches_in` with the SQL repository in one
/// caller write transaction (the `dbClient`).
async fn bulk_cancel(h: &Harness, workspace_id: i64, enrollment_id: i64) -> Vec<DispatchRef> {
    let observer: Arc<dyn TxObserver> = h.probe.clone();
    let repo = SqlPendingDispatchRepo {
        observer: Some(observer),
    };
    let (ws, enrollment) = (id(workspace_id), id(enrollment_id));
    h.db.write(move |tx| {
        Ok(bulk_cancel_pending_dispatches_in(
            tx,
            &repo,
            &ws,
            &enrollment,
        ))
    })
    .await
    .expect("write")
    .expect("bulk cancel")
}

async fn seed(
    h: &Harness,
    dispatch: i64,
    bucket: u32,
    workspace: i64,
    enrollment: i64,
    status: &str,
) {
    sequence::seed_dispatch(
        &h.db,
        json!({
            "id": dispatch, "bucket": bucket, "workspaceId": workspace,
            "enrollmentId": enrollment, "status": status,
        }),
    )
    .await;
}

fn fr(n: i64, bucket: u32) -> DispatchRef {
    DispatchRef { id: id(n), bucket }
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:45 "sequenceDispatchUtils.bulkCancelPendingDispatches > returns empty array when no pending dispatches are found"
#[tokio::test]
async fn returns_empty_array_when_no_pending_dispatches_are_found() {
    let h = sequence::harness(Default::default()).await;

    let result = bulk_cancel(&h, WS_1, ENROLL_1).await;

    assert_eq!(result, Vec::<DispatchRef>::new());
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:58 "sequenceDispatchUtils.bulkCancelPendingDispatches > does not call update when no pending dispatches are found"
#[tokio::test]
async fn does_not_call_update_when_no_pending_dispatches_are_found() {
    let h = sequence::harness(Default::default()).await;

    bulk_cancel(&h, WS_1, ENROLL_1).await;

    assert_eq!(h.probe.count(StatementKind::Update, "SequenceDispatch"), 0);
    assert_eq!(h.probe.writes(), 0);
}

/// `findMany` finds `d-1`, the UPDATE matches nothing (the
/// `returningUpdateMock` resolving `[]`).
struct ConcurrentCancelRepo {
    updates: Mutex<usize>,
}

impl PendingDispatchRepo for ConcurrentCancelRepo {
    fn find_pending(
        &self,
        _conn: &Connection,
        _workspace_id: &str,
        _enrollment_id: &str,
    ) -> Result<Vec<PendingDispatch>, SequenceError> {
        Ok(vec![PendingDispatch {
            id: "d-1".to_owned(),
            bucket: 10,
            sequence_id: "seq-1".to_owned(),
            contact_id: "c-1".to_owned(),
            step_id: "s-1".to_owned(),
        }])
    }

    fn cancel_pending(
        &self,
        _conn: &Connection,
        _workspace_id: &str,
        _ids: &[String],
    ) -> Result<Vec<DispatchRef>, SequenceError> {
        *self.updates.lock().unwrap() += 1;
        Ok(Vec::new())
    }
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:71 "sequenceDispatchUtils.bulkCancelPendingDispatches > returns empty array when update affects no rows (concurrent cancel)"
///
/// Adapted (spec 5.2 note): SELECT and UPDATE share one `BEGIN IMMEDIATE`
/// transaction, so the race needs a repository fake whose UPDATE matches 0 rows.
#[tokio::test]
async fn returns_empty_array_when_update_affects_no_rows_concurrent() {
    let db = chatbotx_db::Db::open_in_memory().expect("open db");
    let repo = Arc::new(ConcurrentCancelRepo {
        updates: Mutex::new(0),
    });
    let repo_in_tx = Arc::clone(&repo);

    let result = db
        .write(move |tx| {
            Ok(bulk_cancel_pending_dispatches_in(
                tx,
                repo_in_tx.as_ref(),
                "ws-1",
                "enroll-1",
            ))
        })
        .await
        .expect("write")
        .expect("bulk cancel");

    assert_eq!(result, Vec::<DispatchRef>::new());
    assert_eq!(*repo.updates.lock().unwrap(), 1);
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:93 "sequenceDispatchUtils.bulkCancelPendingDispatches > returns mapped {id, bucket} pairs when dispatches are successfully canceled"
#[tokio::test]
async fn returns_mapped_id_bucket_pairs_when_dispatches_are() {
    let h = sequence::harness(Default::default()).await;
    seed(&h, D_1, 10, WS_1, ENROLL_1, "pending").await;
    seed(&h, D_2, 20, WS_1, ENROLL_1, "pending").await;

    let result = bulk_cancel(&h, WS_1, ENROLL_1).await;

    assert_eq!(result, vec![fr(D_1, 10), fr(D_2, 20)]);
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_1, "status").await,
        json!("canceled")
    );
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_2, "status").await,
        json!("canceled")
    );
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:127 "sequenceDispatchUtils.bulkCancelPendingDispatches > UPDATE WHERE includes workspaceId and status=pending"
///
/// The WHERE shape becomes its effect: the UPDATE is limited to the
/// workspace and to pending rows. The other-workspace decoy has its own
/// enrollment (an enrollment id belongs to one workspace, composite FK).
#[tokio::test]
async fn update_where_includes_workspaceid_and_status_pending() {
    let h = sequence::harness(Default::default()).await;
    seed(&h, D_1, 1, WS_1, ENROLL_1, "pending").await;
    seed(&h, D_OTHER_WS, 1, WS_2, ENROLL_1_WS_2, "pending").await;
    seed(&h, D_RUNNING, 1, WS_1, ENROLL_1, "running").await;

    let result = bulk_cancel(&h, WS_1, ENROLL_1).await;

    assert_eq!(result, vec![fr(D_1, 1)]);
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_OTHER_WS, "status").await,
        json!("pending")
    );
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_RUNNING, "status").await,
        json!("running")
    );
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:166 "sequenceDispatchUtils.bulkCancelPendingDispatches > calls update exactly once regardless of how many dispatches are found"
#[tokio::test]
async fn calls_update_exactly_once_regardless_of_how_many() {
    let h = sequence::harness(Default::default()).await;
    for i in 0..5u32 {
        seed(&h, D_N + i64::from(i), i, WS_1, ENROLL_1, "pending").await;
    }

    let result = bulk_cancel(&h, WS_1, ENROLL_1).await;

    assert_eq!(result.len(), 5);
    assert_eq!(h.probe.count(StatementKind::Update, "SequenceDispatch"), 1);
}

/// Port of: packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts:187 "sequenceDispatchUtils.bulkCancelPendingDispatches > uses the provided workspaceId and enrollmentId when querying pending dispatches"
#[tokio::test]
async fn uses_the_provided_workspaceid_and_enrollmentid_when() {
    let h = sequence::harness(Default::default()).await;
    seed(&h, D_MATCH, 1, MY_WORKSPACE, MY_ENROLLMENT, "pending").await;
    seed(
        &h,
        D_OTHER_ENROLLMENT,
        1,
        MY_WORKSPACE,
        OTHER_ENROLLMENT,
        "pending",
    )
    .await;
    seed(
        &h,
        D_OTHER_WS,
        1,
        OTHER_WORKSPACE,
        MY_ENROLLMENT_OTHER_WS,
        "pending",
    )
    .await;

    let result = bulk_cancel(&h, MY_WORKSPACE, MY_ENROLLMENT).await;

    assert_eq!(result, vec![fr(D_MATCH, 1)]);
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_OTHER_ENROLLMENT, "status").await,
        json!("pending")
    );
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_OTHER_WS, "status").await,
        json!("pending")
    );
}
