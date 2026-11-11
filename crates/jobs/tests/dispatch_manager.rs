//! Port of `packages/sequence-scheduler/__tests__/dispatch-manager.test.ts`
//! (spec 5.2).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use std::collections::HashSet;
use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_jobs::sequence::cancel::{
    CancelPendingParams, DispatchRef, cancel_pending_dispatches, cancel_pending_dispatches_in,
};
use chatbotx_jobs::sequence::dispatch::{
    CreateDispatchParams, DispatchRecord, calculate_bucket, create_dispatch, create_dispatch_in,
    generate_idempotency_key,
};
use chatbotx_jobs::sequence::repo::{ClientMode, SequenceError};
use serde_json::json;

use sequence::{
    C_1, CI_PARENT, D_1, D_2, D_TX, ENROLL_1, FakeInserter, FixedIds, Harness, HarnessOptions, S_1,
    S_2, SEQ_1, WS_1, id, iso_ms,
};

fn params(n: u8, run_at_ms: i64) -> CreateDispatchParams {
    CreateDispatchParams {
        workspace_id: format!("ws-{n}"),
        contact_id: format!("contact-{n}"),
        contact_inbox_id: format!("inbox-{n}"),
        enrollment_id: format!("enroll-{n}"),
        run_at_ms,
        sequence_id: format!("seq-{n}"),
        step_id: format!("step-{n}"),
    }
}

/// `returningInsertMock` default of the createDispatch block.
fn test_record() -> DispatchRecord {
    DispatchRecord {
        id: "test-id".to_owned(),
        bucket: 77,
        run_at_ms: 1_700_000_000_000,
    }
}

async fn create_harness() -> Harness {
    let h = sequence::harness(HarnessOptions {
        ids: FixedIds::new("test-id"),
        dispatch: test_record(),
        ..Default::default()
    })
    .await;
    h
}

// ---------------------------------------------------------------------------
// calculateBucket
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:69 "calculateBucket > returns a number between 0 and 255 inclusive"
#[test]
fn returns_a_number_between_0_and_255_inclusive() {
    let bucket = calculate_bucket("workspace-1", "contact-1");

    assert!(bucket <= 255);
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:78 "calculateBucket > is deterministic — same inputs always produce the same bucket"
#[test]
fn is_deterministic_same_inputs_always_produce_the_same() {
    let first = calculate_bucket("workspace-abc", "contact-xyz");
    let second = calculate_bucket("workspace-abc", "contact-xyz");

    assert_eq!(first, second);
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:87 "calculateBucket > produces different buckets for different workspaceId + contactId pairs"
#[test]
fn produces_different_buckets_for_different_workspaceid() {
    let bucket_a = calculate_bucket("workspace-1", "contact-1");
    let bucket_b = calculate_bucket("workspace-2", "contact-2");

    assert_ne!(bucket_a, bucket_b);
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:98 "calculateBucket > spreads sequential ids across all 256 buckets"
#[test]
fn spreads_sequential_ids_across_all_256_buckets() {
    let mut buckets = HashSet::new();
    let mut load = vec![0u32; 256];
    for index in 0..5000 {
        let bucket = calculate_bucket(&format!("workspace-{index}"), &format!("contact-{index}"));
        buckets.insert(bucket);
        load[bucket as usize] += 1;
    }

    assert_eq!(buckets.len(), 256);
    assert!(*load.iter().max().unwrap() < 50);
}

// ---------------------------------------------------------------------------
// generateIdempotencyKey
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:120 "generateIdempotencyKey > produces the exact format workspaceId:enrollmentId:stepId:runAt.toISOString()"
#[test]
fn produces_the_exact_format_workspaceid_enrollmentid_stepid() {
    let run_at = iso_ms("2024-06-01T10:30:00.000Z");

    let key = generate_idempotency_key("ws-1", "enroll-1", "step-1", run_at);

    assert_eq!(key, "ws-1:enroll-1:step-1:2024-06-01T10:30:00.000Z");
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:129 "generateIdempotencyKey > uses runAt.toISOString so the key is stable across calls with the same Date"
#[test]
fn uses_runat_toisostring_so_the_key_is_stable_across_calls() {
    let run_at = iso_ms("2025-01-15T08:00:00.000Z");

    let key_a = generate_idempotency_key("ws-x", "e-1", "s-1", run_at);
    let key_b = generate_idempotency_key("ws-x", "e-1", "s-1", run_at);

    assert_eq!(key_a, key_b);
}

// ---------------------------------------------------------------------------
// createDispatch
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:153 "createDispatch > returns the created dispatch record on success"
#[tokio::test]
async fn returns_the_created_dispatch_record_on_success() {
    let h = create_harness().await;

    let result = create_dispatch(
        &h.ctx,
        CreateDispatchParams {
            workspace_id: "ws-1".to_owned(),
            contact_id: "contact-1".to_owned(),
            contact_inbox_id: "inbox-1".to_owned(),
            enrollment_id: "enroll-1".to_owned(),
            run_at_ms: 1_700_000_000_000,
            sequence_id: "seq-1".to_owned(),
            step_id: "step-1".to_owned(),
        },
        ClientMode::Default,
    )
    .await
    .expect("create dispatch");

    assert_eq!(
        result,
        DispatchRecord {
            id: "test-id".to_owned(),
            bucket: 77,
            run_at_ms: 1_700_000_000_000,
        }
    );
    assert_eq!(h.probe.begins(), 0);
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:175 "createDispatch > throws 'Failed to create dispatch' when insert returns empty array"
///
/// Adapted (TS-only: SQLite `RETURNING` of a successful insert is never
/// empty): the inserter fake returns `None`.
#[tokio::test]
async fn throws_failed_to_create_dispatch_when_insert_returns_empty() {
    let mut h = create_harness().await;
    h.ctx.inserter = FakeInserter::returning(Ok(None));

    let error = create_dispatch(
        &h.ctx,
        CreateDispatchParams {
            workspace_id: "ws-1".to_owned(),
            contact_id: "contact-1".to_owned(),
            contact_inbox_id: "inbox-1".to_owned(),
            enrollment_id: "enroll-1".to_owned(),
            run_at_ms: h.clock_now(),
            sequence_id: "seq-1".to_owned(),
            step_id: "step-1".to_owned(),
        },
        ClientMode::Default,
    )
    .await
    .expect_err("empty returning");

    assert!(error.to_string().contains("Failed to create dispatch"));
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:192 "createDispatch > uses provided client instead of default db"
///
/// The provided client is the caller's open write transaction and its own
/// inserter; the context inserter (the "default db") is never used.
#[tokio::test]
async fn uses_provided_client_instead_of_default_db() {
    let h = create_harness().await;
    let custom = FakeInserter::returning(Ok(Some(DispatchRecord {
        id: "custom-id".to_owned(),
        bucket: 5,
        run_at_ms: 999,
    })));
    let custom_in_tx = Arc::clone(&custom);
    let ids = FixedIds::new("custom-id");

    let result =
        h.db.write(move |tx| {
            let record =
                create_dispatch_in(tx, custom_in_tx.as_ref(), ids.as_ref(), &params(2, 999))
                    .expect("create dispatch in tx");
            Ok(record)
        })
        .await
        .expect("write");

    assert_eq!(custom.calls(), 1);
    assert_eq!(h.inserter.calls(), 0);
    assert_eq!(h.probe.begins(), 0);
    assert_eq!(result.id, "custom-id");
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:218 "createDispatch > uses the provided root db client without wrapping another transaction"
#[tokio::test]
async fn uses_the_provided_root_db_client_without_wrapping_another() {
    let h = create_harness().await;

    let result = create_dispatch(&h.ctx, params(2, h.clock_now()), ClientMode::Root)
        .await
        .expect("create dispatch");

    assert_eq!(h.probe.begins(), 0);
    assert_eq!(result.id, "test-id");
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:237 "createDispatch > throws when SequenceDispatch unique idempotency insert conflicts"
#[tokio::test]
async fn throws_when_sequencedispatch_unique_idempotency_insert() {
    let mut h = create_harness().await;
    h.ctx.inserter = FakeInserter::returning(Err(SequenceError::Db("duplicate key".to_owned())));

    let error = create_dispatch(&h.ctx, params(1, h.clock_now()), ClientMode::Default)
        .await
        .expect_err("duplicate key");

    assert!(error.to_string().contains("duplicate key"));
}

// ---------------------------------------------------------------------------
// cancelPendingDispatches (seeded ids are integers, P2.4 DDL)
// ---------------------------------------------------------------------------

async fn seed_pending(db: &Db, dispatch: i64, bucket: u32, step: i64) {
    sequence::seed_dispatch(
        db,
        json!({
            "id": dispatch,
            "bucket": bucket,
            "workspaceId": WS_1,
            "enrollmentId": ENROLL_1,
            "sequenceId": SEQ_1,
            "contactId": C_1,
            "stepId": step,
            "status": "pending",
        }),
    )
    .await;
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:263 "cancelPendingDispatches > returns empty array and skips update when no pending dispatches exist"
#[tokio::test]
async fn returns_empty_array_and_skips_update_when_no_pending() {
    let h = create_harness().await;

    let result =
        cancel_pending_dispatches(&h.ctx, CancelPendingParams::new(&id(ENROLL_1), &id(WS_1)))
            .await
            .expect("cancel");

    assert_eq!(result, Vec::<DispatchRef>::new());
    assert_eq!(
        h.probe.count(
            chatbotx_jobs::sequence::repo::StatementKind::Update,
            "SequenceDispatch"
        ),
        0
    );
    assert!(h.schedule.removed().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:277 "cancelPendingDispatches > cancels pending dispatches, calls removeFromSchedule for each, and returns id+bucket"
#[tokio::test]
async fn cancels_pending_dispatches_calls_removefromschedule_for() {
    let h = create_harness().await;
    seed_pending(&h.db, D_1, 10, S_1).await;
    seed_pending(&h.db, D_2, 20, S_2).await;

    let result =
        cancel_pending_dispatches(&h.ctx, CancelPendingParams::new(&id(ENROLL_1), &id(WS_1)))
            .await
            .expect("cancel");

    assert_eq!(
        h.probe.count(
            chatbotx_jobs::sequence::repo::StatementKind::Update,
            "SequenceDispatch"
        ),
        1
    );
    assert_eq!(h.schedule.removed().len(), 2);
    assert!(h.schedule.removed().contains(&(10, id(D_1))));
    assert!(h.schedule.removed().contains(&(20, id(D_2))));
    assert_eq!(
        result,
        vec![
            DispatchRef {
                id: id(D_1),
                bucket: 10,
            },
            DispatchRef {
                id: id(D_2),
                bucket: 20,
            },
        ]
    );
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_1, "status").await,
        json!("canceled")
    );
    assert_eq!(
        sequence::column(&h.db, "SequenceDispatch", D_2, "status").await,
        json!("canceled")
    );
}

// dispatch-manager.test.ts:312 "calls useExisting to obtain the redis client
// before scheduling removal" is not ported (disputed, see the test map).

/// Port of: packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:332 "cancelPendingDispatches > uses provided client instead of default db for the query"
///
/// The pending row exists only inside the caller's open transaction: the
/// query must run on that client to see (and cancel) it.
#[tokio::test]
async fn uses_provided_client_instead_of_default_db_for_the_query() {
    let h = create_harness().await;
    sequence::ensure_dispatch_parents(&h.db, WS_1, ENROLL_1, SEQ_1, C_1, S_1, CI_PARENT).await;

    let result =
        h.db.write(|tx| {
            tx.execute(
                "INSERT INTO \"SequenceDispatch\" (\"id\", \"workspaceId\", \"sequenceId\", \
                 \"contactId\", \"contactInboxId\", \"stepId\", \"enrollmentId\", \"bucket\", \
                 \"runAtMs\", \"status\", \"idempotencyKey\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 3, 1700000000000, 'pending', 'key-d-tx')",
                [D_TX, WS_1, SEQ_1, C_1, CI_PARENT, S_1, ENROLL_1],
            )?;
            let canceled = cancel_pending_dispatches_in(
                tx,
                None,
                &CancelPendingParams::new(&id(ENROLL_1), &id(WS_1)),
            )
            .expect("cancel in tx");
            Ok(canceled)
        })
        .await
        .expect("write");

    assert_eq!(
        result,
        vec![DispatchRef {
            id: id(D_TX),
            bucket: 3,
        }]
    );
}

trait ClockNow {
    fn clock_now(&self) -> i64;
}

impl ClockNow for Harness {
    fn clock_now(&self) -> i64 {
        use chatbotx_jobs::Clock;
        self.clock.now_ms()
    }
}
