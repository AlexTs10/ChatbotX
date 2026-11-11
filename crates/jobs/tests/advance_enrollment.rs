//! Port of `packages/sequence-scheduler/__tests__/advance-enrollment.test.ts`
//! (spec 5.2). `getContactInboxes`, `createDispatch`,
//! `calculateNextRunAtFromStep` and `calculateNextValidSendTime` are mocked
//! in the TypeScript test; here they are the harness fakes
//! (FAKE_DISPATCH = "dispatch-1" = 801, bucket 7, run at 1 700 000 000 000;
//! next run = NEXT_RUN_AT; send time passes through). The enrollment and
//! the steps are real `"ContactOnSequence"` / `"SequenceStep"` rows (P2.4);
//! ids are integers (mapping table in `support/sequence.rs`).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use chatbotx_jobs::sequence::advance::{AdvanceParams, CurrentStep, advance_enrollment};
use chatbotx_jobs::sequence::dispatch::{CreateDispatchParams, DispatchRecord};
use chatbotx_jobs::sequence::repo::{SequenceError, StatementKind};
use serde_json::{Value, json};

use sequence::{
    CONTACT_1, DISPATCH_1, ENROLLMENT_1, Harness, HarnessOptions, INBOX_1, INBOX_2, MISSING_ID,
    SEQ_1, STEP_1, STEP_2, WS_1, id, iso_ms,
};

const SENT_AT: &str = "2024-03-01T12:00:00Z";
const NEXT_RUN_AT: &str = "2024-03-02T09:00:00Z";
const FAKE_RUN_AT_MS: i64 = 1_700_000_000_000;
const TABLE: &str = "ContactOnSequence";

fn fake_dispatch() -> DispatchRecord {
    DispatchRecord {
        id: id(DISPATCH_1),
        bucket: 7,
        run_at_ms: FAKE_RUN_AT_MS,
    }
}

async fn advance_harness() -> Harness {
    sequence::harness(HarnessOptions {
        inboxes: vec![id(INBOX_1)],
        dispatch: fake_dispatch(),
        next_run_at_ms: iso_ms(NEXT_RUN_AT),
        ..Default::default()
    })
    .await
}

/// Harness + the enrollment row (`makeActiveEnrollment(overrides)`) + the
/// current step "step-1" (order 0) and, when `with_next_step`, NEXT_STEP
/// ("step-2", order 1, 1 day, anytime).
async fn setup(enrollment: Value, with_next_step: bool) -> Harness {
    let h = advance_harness().await;
    sequence::seed_enrollment(&h.db, enrollment).await;
    sequence::seed_step(&h.db, json!({ "id": STEP_1, "order": 0 })).await;
    if with_next_step {
        sequence::seed_step(
            &h.db,
            json!({ "id": STEP_2, "order": 1, "delayDays": 1, "anytime": true }),
        )
        .await;
    }
    h
}

fn params(step_order: i64) -> AdvanceParams {
    AdvanceParams {
        enrollment_id: id(ENROLLMENT_1),
        workspace_id: id(WS_1),
        sequence_id: id(SEQ_1),
        contact_id: id(CONTACT_1),
        current_step: CurrentStep {
            id: id(STEP_1),
            order: step_order,
        },
        sent_at_ms: iso_ms(SENT_AT),
    }
}

async fn advance(h: &Harness, params: AdvanceParams) {
    advance_enrollment(&h.ctx, params).await.expect("advance");
}

async fn enrollment(h: &Harness) -> Value {
    sequence::read_row(&h.db, TABLE, ENROLLMENT_1)
        .await
        .expect("enrollment row")
}

// ---------------------------------------------------------------------------
// when enrollment is not found
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:166 "advanceEnrollment > when enrollment is not found > throws with message containing the enrollment id"
#[tokio::test]
async fn throws_with_message_containing_the_enrollment_id() {
    let h = advance_harness().await;
    // Reads the real (empty) ContactOnSequence table.
    assert_eq!(sequence::count_rows(&h.db, TABLE).await, 0);

    let error = advance_enrollment(
        &h.ctx,
        AdvanceParams {
            enrollment_id: id(MISSING_ID),
            ..params(0)
        },
    )
    .await
    .expect_err("missing enrollment");

    assert!(error.to_string().contains("Enrollment 999 not found"));
}

// ---------------------------------------------------------------------------
// when enrollment status is not active / already advanced
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:176 "advanceEnrollment > when enrollment status is not active > returns without updating when status is paused"
#[tokio::test]
async fn returns_without_updating_when_status_is_paused() {
    let h = setup(json!({ "status": "paused" }), true).await;
    let before = enrollment(&h).await;

    advance(&h, params(0)).await;

    assert_eq!(h.probe.count(StatementKind::Update, TABLE), 0);
    assert_eq!(enrollment(&h).await, before);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:186 "advanceEnrollment > when enrollment status is not active > returns without updating when status is completed"
#[tokio::test]
async fn returns_without_updating_when_status_is_completed() {
    let h = setup(json!({ "status": "completed" }), true).await;
    let before = enrollment(&h).await;

    advance(&h, params(0)).await;

    assert_eq!(h.probe.count(StatementKind::Update, TABLE), 0);
    assert_eq!(enrollment(&h).await, before);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:198 "advanceEnrollment > when enrollment has already been advanced past the current step > returns without updating when lastStepId matches currentStep.id"
#[tokio::test]
async fn returns_without_updating_when_laststepid_matches_currentstep_id() {
    let h = setup(json!({ "lastStepId": STEP_1 }), true).await;
    let before = enrollment(&h).await;

    advance(&h, params(0)).await;

    assert_eq!(h.probe.count(StatementKind::Update, TABLE), 0);
    assert_eq!(enrollment(&h).await, before);
}

// ---------------------------------------------------------------------------
// when there is no next step (sequence completed)
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:212 "advanceEnrollment > when there is no next step (sequence completed) > updates enrollment status to completed"
#[tokio::test]
async fn updates_enrollment_status_to_completed() {
    let h = setup(json!({}), false).await;

    advance(&h, params(0)).await;

    assert_eq!(enrollment(&h).await["status"], json!("completed"));
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:222 "advanceEnrollment > when there is no next step (sequence completed) > sets completedAt to the sentAt value"
#[tokio::test]
async fn sets_completedat_to_the_sentat_value() {
    let h = setup(json!({}), false).await;

    advance(&h, params(0)).await;

    assert_eq!(enrollment(&h).await["completedAt"], json!(iso_ms(SENT_AT)));
    assert_eq!(iso_ms(SENT_AT), 1_709_294_400_000);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:232 "advanceEnrollment > when there is no next step (sequence completed) > sets nextStepId and nextRunAt to null"
#[tokio::test]
async fn sets_nextstepid_and_nextrunat_to_null() {
    let h = setup(json!({ "nextStepId": STEP_1, "nextRunAt": 1 }), false).await;

    advance(&h, params(0)).await;

    let row = enrollment(&h).await;
    assert_eq!(row["nextStepId"], Value::Null);
    assert_eq!(row["nextRunAt"], Value::Null);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:242 "advanceEnrollment > when there is no next step (sequence completed) > increments currentStep to currentStep.order + 1"
#[tokio::test]
async fn increments_currentstep_to_currentstep_order_1() {
    let h = setup(json!({}), false).await;

    advance(
        &h,
        AdvanceParams {
            current_step: CurrentStep {
                id: id(STEP_1),
                order: 2,
            },
            ..params(2)
        },
    )
    .await;

    assert_eq!(enrollment(&h).await["currentStep"], json!(3));
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:254 "advanceEnrollment > when there is no next step (sequence completed) > does not create a dispatch or call scheduler"
#[tokio::test]
async fn does_not_create_a_dispatch_or_call_scheduler() {
    let h = setup(json!({}), false).await;

    advance(&h, params(0)).await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:262 "advanceEnrollment > when there is no next step (sequence completed) > does not start a transaction"
#[tokio::test]
async fn does_not_start_a_transaction() {
    let h = setup(json!({}), false).await;

    advance(&h, params(0)).await;

    assert_eq!(h.probe.begins(), 0);
    assert_eq!(h.probe.count(StatementKind::Update, TABLE), 1);
}

// ---------------------------------------------------------------------------
// when there is a next step
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:292 "advanceEnrollment > when there is a next step > starts a database transaction"
#[tokio::test]
async fn starts_a_database_transaction() {
    let h = setup(json!({}), true).await;

    advance(&h, params(0)).await;

    assert_eq!(h.probe.begins(), 1);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:301 "advanceEnrollment > when there is a next step > does NOT call the top-level db.update (uses tx.update instead)"
///
/// Adapted (TS-only: Drizzle `db` vs `tx` handle): the update runs inside the
/// transaction, so a failing `createDispatch` in the same transaction rolls
/// the enrollment update back too.
#[tokio::test]
async fn does_not_call_the_top_level_db_update_uses_tx_update() {
    let h = setup(json!({}), true).await;
    h.creator
        .set_result(Err(SequenceError::Other("insert failed".to_owned())));
    let before = enrollment(&h).await;

    let result = advance_enrollment(&h.ctx, params(0)).await;

    assert!(result.is_err());
    assert_eq!(enrollment(&h).await, before);
    assert_eq!(h.probe.begins(), 1);
    assert_eq!(h.probe.commits(), 0);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:310 "advanceEnrollment > when there is a next step > creates a dispatch for each contact inbox inside the transaction"
#[tokio::test]
async fn creates_a_dispatch_for_each_contact_inbox_inside_the() {
    let h = setup(json!({}), true).await;
    h.lookup.set_ints(&[INBOX_1, INBOX_2]);

    advance(&h, params(0)).await;

    assert_eq!(h.creator.calls().len(), 2);
    assert_eq!(h.creator.in_tx(), vec![true, true]);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:323 "advanceEnrollment > when there is a next step > passes correct params to createDispatch"
#[tokio::test]
async fn passes_correct_params_to_createdispatch() {
    let h = setup(json!({}), true).await;

    advance(&h, params(0)).await;

    assert_eq!(
        h.creator.calls(),
        vec![CreateDispatchParams {
            workspace_id: id(WS_1),
            contact_id: id(CONTACT_1),
            contact_inbox_id: id(INBOX_1),
            enrollment_id: id(ENROLLMENT_1),
            run_at_ms: iso_ms(NEXT_RUN_AT),
            sequence_id: id(SEQ_1),
            step_id: id(STEP_2),
        }]
    );
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:347 "advanceEnrollment > when there is a next step > calls scheduler.addToSchedule for each dispatch created"
#[tokio::test]
async fn calls_scheduler_addtoschedule_for_each_dispatch_created() {
    let h = setup(json!({}), true).await;
    h.lookup.set_ints(&[INBOX_1, INBOX_2]);

    advance(&h, params(0)).await;

    assert_eq!(
        h.schedule.scheduled(),
        vec![
            (7, id(DISPATCH_1), FAKE_RUN_AT_MS),
            (7, id(DISPATCH_1), FAKE_RUN_AT_MS),
        ]
    );
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:375 "advanceEnrollment > when there is a next step > schedules only after the advance transaction completes"
#[tokio::test]
async fn schedules_only_after_the_advance_transaction_completes() {
    let h = setup(json!({}), true).await;

    advance(&h, params(0)).await;

    assert_eq!(h.order(), vec!["tx-done".to_owned(), "schedule".to_owned()]);
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:397 "advanceEnrollment > when there is a next step > does not create dispatches when contact has no inboxes"
#[tokio::test]
async fn does_not_create_dispatches_when_contact_has_no_inboxes() {
    let h = setup(json!({}), true).await;
    h.lookup.set_ints(&[]);

    advance(&h, params(0)).await;

    assert!(h.creator.calls().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/advance-enrollment.test.ts:407 "advanceEnrollment > when there is a next step > sets lastStepId to current step id in the update"
#[tokio::test]
async fn sets_laststepid_to_current_step_id_in_the_update() {
    let h = setup(json!({}), true).await;

    advance(&h, params(0)).await;

    let row = enrollment(&h).await;
    assert_eq!(row["lastStepId"], json!(STEP_1));
    assert_eq!(row["nextStepId"], json!(STEP_2));
}
