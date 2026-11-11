//! Port of `packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts`
//! (spec 5.2). "Redis" in the names is the `DispatchSchedule` side. The
//! Drizzle WHERE-shape asserts become effects on real rows: decoy rows of
//! another enrollment, workspace or status stay unchanged. Seeded ids are
//! integers (P2.4 DDL; mapping table in `support/sequence.rs`); the
//! fake-only `removeDispatchesFromSchedule` cases keep the TS string ids.

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use chatbotx_jobs::sequence::cancel::{
    CancelPendingParams, DispatchRef, cancel_pending_dispatches, remove_dispatches_from_schedule,
};
use chatbotx_jobs::sequence::repo::StatementKind;
use serde_json::json;

use sequence::{
    D1, D2, DECOY_ENROLLMENT, DECOY_STATUS, DECOY_WORKSPACE, E1, E1_W2, E2, Harness, W1, W2, id,
};

fn d(id: &str, bucket: u32) -> DispatchRef {
    DispatchRef {
        id: id.to_owned(),
        bucket,
    }
}

async fn seed_pending_pair(h: &Harness) {
    for (dispatch, bucket) in [(D1, 1), (D2, 2)] {
        sequence::seed_dispatch(
            &h.db,
            json!({
                "id": dispatch, "bucket": bucket, "workspaceId": W1,
                "enrollmentId": E1, "status": "pending",
            }),
        )
        .await;
    }
}

async fn seed_decoys(h: &Harness) {
    // Other enrollment, other workspace (an enrollment id belongs to one
    // workspace, so this decoy has its own enrollment E1_W2), not pending.
    sequence::seed_dispatch(
        &h.db,
        json!({ "id": DECOY_ENROLLMENT, "workspaceId": W1, "enrollmentId": E2, "status": "pending" }),
    )
    .await;
    sequence::seed_dispatch(
        &h.db,
        json!({ "id": DECOY_WORKSPACE, "workspaceId": W2, "enrollmentId": E1_W2, "status": "pending" }),
    )
    .await;
    sequence::seed_dispatch(
        &h.db,
        json!({ "id": DECOY_STATUS, "workspaceId": W1, "enrollmentId": E1, "status": "running" }),
    )
    .await;
}

async fn status(h: &Harness, dispatch: i64) -> serde_json::Value {
    sequence::column(&h.db, "SequenceDispatch", dispatch, "status").await
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:60 "cancelPendingDispatches > SELECT filters enrollmentId + workspaceId + status=pending"
#[tokio::test]
async fn select_filters_enrollmentid_workspaceid_status_pending() {
    let h = sequence::harness(Default::default()).await;
    seed_decoys(&h).await;

    let result = cancel_pending_dispatches(&h.ctx, CancelPendingParams::new(&id(E1), &id(W1)))
        .await
        .expect("cancel");

    assert_eq!(result, Vec::<DispatchRef>::new());
    assert_eq!(status(&h, DECOY_ENROLLMENT).await, json!("pending"));
    assert_eq!(status(&h, DECOY_WORKSPACE).await, json!("pending"));
    assert_eq!(status(&h, DECOY_STATUS).await, json!("running"));
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:78 "cancelPendingDispatches > empty result skips UPDATE and Redis"
#[tokio::test]
async fn empty_result_skips_update_and_redis() {
    let h = sequence::harness(Default::default()).await;

    let result = cancel_pending_dispatches(&h.ctx, CancelPendingParams::new(&id(E1), &id(W1)))
        .await
        .expect("cancel");

    assert_eq!(result, Vec::<DispatchRef>::new());
    assert_eq!(h.probe.count(StatementKind::Update, "SequenceDispatch"), 0);
    assert!(h.schedule.removed().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:93 "cancelPendingDispatches > UPDATE WHERE has id + workspaceId + status; Redis removed per dispatch"
#[tokio::test]
async fn update_where_has_id_workspaceid_status_redis_removed_per() {
    let h = sequence::harness(Default::default()).await;
    seed_pending_pair(&h).await;
    seed_decoys(&h).await;

    cancel_pending_dispatches(&h.ctx, CancelPendingParams::new(&id(E1), &id(W1)))
        .await
        .expect("cancel");

    assert_eq!(status(&h, D1).await, json!("canceled"));
    assert_eq!(status(&h, D2).await, json!("canceled"));
    assert_eq!(status(&h, DECOY_ENROLLMENT).await, json!("pending"));
    assert_eq!(status(&h, DECOY_WORKSPACE).await, json!("pending"));
    assert_eq!(status(&h, DECOY_STATUS).await, json!("running"));
    assert_eq!(h.schedule.removed().len(), 2);
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:111 "cancelPendingDispatches > skips Redis removal when removal is deferred"
#[tokio::test]
async fn skips_redis_removal_when_removal_is_deferred() {
    let h = sequence::harness(Default::default()).await;
    seed_pending_pair(&h).await;

    let result = cancel_pending_dispatches(
        &h.ctx,
        CancelPendingParams {
            remove_from_schedule: false,
            ..CancelPendingParams::new(&id(E1), &id(W1))
        },
    )
    .await
    .expect("cancel");

    assert_eq!(result, vec![d(&id(D1), 1), d(&id(D2), 2)]);
    assert_eq!(h.probe.count(StatementKind::Update, "SequenceDispatch"), 1);
    assert!(h.schedule.removed().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:133 "cancelPendingDispatches > removeDispatchesFromSchedule removes each dispatch from Redis"
#[tokio::test]
async fn removedispatchesfromschedule_removes_each_dispatch_from() {
    let order = sequence::order_log();
    let schedule = sequence::RecordingSchedule::new(&order);

    remove_dispatches_from_schedule(schedule.as_ref(), &[d("d1", 1), d("d2", 2)])
        .await
        .expect("remove");

    let removed = schedule.removed();
    assert_eq!(removed.len(), 2);
    assert!(removed.contains(&(1, "d1".to_owned())));
    assert!(removed.contains(&(2, "d2".to_owned())));
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:148 "cancelPendingDispatches > removeDispatchesFromSchedule attempts every Redis removal before reporting failures"
#[tokio::test]
async fn removedispatchesfromschedule_attempts_every_removal_before() {
    let order = sequence::order_log();
    let schedule = sequence::RecordingSchedule::new(&order);
    schedule.fail_call(1, "redis down");

    let error = remove_dispatches_from_schedule(schedule.as_ref(), &[d("d1", 1), d("d2", 2)])
        .await
        .expect_err("one removal failed");

    assert!(
        error
            .to_string()
            .contains("Failed to remove sequence dispatches from schedule")
    );
    let removed = schedule.removed();
    assert_eq!(removed.len(), 2);
    assert!(removed.contains(&(1, "d1".to_owned())));
    assert!(removed.contains(&(2, "d2".to_owned())));
}

/// Port of: packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts:168 "cancelPendingDispatches > removeDispatchesFromSchedule identifies failed dispatches"
#[tokio::test]
async fn removedispatchesfromschedule_identifies_failed_dispatches() {
    let order = sequence::order_log();
    let schedule = sequence::RecordingSchedule::new(&order);
    schedule.fail_call(1, "redis down");

    let error = remove_dispatches_from_schedule(schedule.as_ref(), &[d("d1", 1), d("d2", 2)])
        .await
        .expect_err("one removal failed");

    assert!(error.to_string().contains("d1 bucket=1"));
}
