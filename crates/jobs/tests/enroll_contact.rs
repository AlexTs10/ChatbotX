//! Port of `packages/sequence-scheduler/__tests__/enroll-contact.test.ts`
//! (spec 5.2). `getContactInboxes` and `createDispatch` are mocked in the
//! TypeScript test; here they are the `FakeContactInboxLookup` and
//! `RecordingDispatchCreator` of the harness (FAKE_DISPATCH = "dispatch-1"
//! = 801, bucket 42, run at 1 700 000 000 000). The enrollment insert runs
//! on the real `"ContactOnSequence"` table (P2.4); ids are integers
//! (mapping table in `support/sequence.rs`).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use chatbotx_jobs::sequence::dispatch::{CreateDispatchParams, DispatchRecord};
use chatbotx_jobs::sequence::enroll::{
    BulkEnrollment, EnrollParams, EnrollResult, enroll_contact_in_sequence,
    enroll_contacts_in_sequence_bulk,
};
use chatbotx_jobs::sequence::repo::{ClientMode, ContactInboxRef, StatementKind};
use serde_json::json;

use sequence::{
    CI_9, CONTACT_1, CONTACT_2, DISPATCH_1, ENROLL_1, ENROLL_2, ENROLL_42, ENROLLMENT_1,
    EXISTING_ENROLLMENT, FixedIds, Harness, HarnessOptions, INBOX_1, INBOX_2, INBOX_X, SEQ_1,
    STEP_1, STEP_99, WS_1, id, iso_ms,
};

const NOW: &str = "2024-03-01T10:00:00Z";
const FAKE_RUN_AT_MS: i64 = 1_700_000_000_000;
const TABLE: &str = "ContactOnSequence";

fn fake_dispatch() -> DispatchRecord {
    DispatchRecord {
        id: id(DISPATCH_1),
        bucket: 42,
        run_at_ms: FAKE_RUN_AT_MS,
    }
}

/// Harness whose id generator hands out `enrollment_ids`, plus the parent
/// rows of the enrollments (workspace, contacts, sequence; none in
/// `"ContactOnSequence"`).
async fn enroll_harness(enrollment_ids: &[i64]) -> Harness {
    let h = sequence::harness(HarnessOptions {
        ids: FixedIds::ints(enrollment_ids, 900),
        inboxes: vec![id(INBOX_1)],
        dispatch: fake_dispatch(),
        ..Default::default()
    })
    .await;
    sequence::ensure_contact(&h.db, CONTACT_1, WS_1).await;
    sequence::ensure_contact(&h.db, CONTACT_2, WS_1).await;
    sequence::ensure_sequence(&h.db, SEQ_1, WS_1).await;
    h
}

fn enroll_params() -> EnrollParams {
    EnrollParams {
        workspace_id: id(WS_1),
        contact_id: id(CONTACT_1),
        sequence_id: id(SEQ_1),
        next_run_at_ms: iso_ms(NOW),
        next_step_id: Some(id(STEP_1)),
        contact_inboxes: None,
    }
}

fn bulk_one(contact: i64, run_at: Option<i64>, step: Option<i64>) -> BulkEnrollment {
    BulkEnrollment {
        contact_id: id(contact),
        sequence_id: id(SEQ_1),
        next_run_at_ms: run_at,
        next_step_id: step.map(id),
    }
}

async fn enroll(h: &Harness, params: EnrollParams) -> EnrollResult {
    enroll_contact_in_sequence(&h.ctx, params, ClientMode::Default)
        .await
        .expect("enroll")
}

// ---------------------------------------------------------------------------
// enrollContactInSequence
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:158 "enrollContactInSequence > when the contact is already enrolled (the insert conflicts and returns no row) > inserts with onConflictDoNothing, reports it did not enroll, and creates no dispatch or schedule"
#[tokio::test]
async fn inserts_with_onconflictdonothing_reports_it_did_not_enroll() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;
    sequence::seed_enrollment(&h.db, json!({ "id": EXISTING_ENROLLMENT })).await;

    let result = enroll(&h, enroll_params()).await;

    assert_eq!(result, EnrollResult { enrolled: false });
    assert_eq!(h.probe.count(StatementKind::Insert, TABLE), 1);
    assert_eq!(sequence::count_rows(&h.db, TABLE).await, 1);
    assert!(h.lookup.calls().is_empty());
    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:173 "enrollContactInSequence > contact inboxes > looks them up when the caller did not pass them"
#[tokio::test]
async fn looks_them_up_when_the_caller_did_not_pass_them() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(h.lookup.calls(), vec![(id(WS_1), id(CONTACT_1))]);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:183 "enrollContactInSequence > contact inboxes > uses the ones the caller passed and does not look them up again"
#[tokio::test]
async fn uses_the_ones_the_caller_passed_and_does_not_look_them_up() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(
        &h,
        EnrollParams {
            contact_inboxes: Some(vec![ContactInboxRef { id: id(CI_9) }]),
            ..enroll_params()
        },
    )
    .await;

    assert!(h.lookup.calls().is_empty());
    let calls = h.creator.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].contact_inbox_id, id(CI_9));
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:197 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > reports the contact as enrolled once dispatches are scheduled"
#[tokio::test]
async fn reports_the_contact_as_enrolled_once_dispatches_are() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    let result = enroll(&h, enroll_params()).await;

    assert_eq!(result, EnrollResult { enrolled: true });
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:203 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > uses a transaction when no client is provided"
#[tokio::test]
async fn uses_a_transaction_when_no_client_is_provided() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(h.probe.begins(), 1);
    // createDispatch ran on the transaction connection.
    assert_eq!(h.creator.in_tx(), vec![true]);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:217 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > uses the provided root db client without wrapping another transaction"
#[tokio::test]
async fn uses_the_provided_root_db_client_without_wrapping_another() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll_contact_in_sequence(&h.ctx, enroll_params(), ClientMode::Root)
        .await
        .expect("enroll");

    assert_eq!(h.probe.begins(), 0);
    assert_eq!(h.creator.calls().len(), 1);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:228 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > inserts a new enrollment row"
#[tokio::test]
async fn inserts_a_new_enrollment_row() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(h.probe.count(StatementKind::Insert, TABLE), 1);
    assert_eq!(sequence::count_rows(&h.db, TABLE).await, 1);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:235 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > inserted ContactOnSequence carries workspaceId"
#[tokio::test]
async fn inserted_contactonsequence_carries_workspaceid() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(
        sequence::column(&h.db, TABLE, ENROLLMENT_1, "workspaceId").await,
        json!(WS_1)
    );
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:241 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > creates a dispatch for each contact inbox"
#[tokio::test]
async fn creates_a_dispatch_for_each_contact_inbox() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;
    h.lookup.set_ints(&[INBOX_1, INBOX_2]);

    enroll(&h, enroll_params()).await;

    assert_eq!(h.creator.calls().len(), 2);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:252 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > passes correct params to createDispatch"
#[tokio::test]
async fn passes_correct_params_to_createdispatch() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(
        h.creator.calls(),
        vec![CreateDispatchParams {
            workspace_id: id(WS_1),
            contact_id: id(CONTACT_1),
            contact_inbox_id: id(INBOX_1),
            enrollment_id: id(ENROLLMENT_1),
            run_at_ms: iso_ms(NOW),
            sequence_id: id(SEQ_1),
            step_id: id(STEP_1),
        }]
    );
    assert_eq!(h.creator.in_tx(), vec![true]);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:269 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > schedules each created dispatch"
#[tokio::test]
async fn schedules_each_created_dispatch() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;
    h.lookup.set_ints(&[INBOX_1, INBOX_2]);

    enroll(&h, enroll_params()).await;

    assert_eq!(
        h.schedule.scheduled(),
        vec![
            (42, id(DISPATCH_1), FAKE_RUN_AT_MS),
            (42, id(DISPATCH_1), FAKE_RUN_AT_MS),
        ]
    );
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:286 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > schedules only after the enrollment transaction completes"
#[tokio::test]
async fn schedules_only_after_the_enrollment_transaction_completes() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(&h, enroll_params()).await;

    assert_eq!(h.order(), vec!["tx-done".to_owned(), "schedule".to_owned()]);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:292 "enrollContactInSequence > when enrollment does not exist and nextStepId is provided > does not create dispatches when contact has no inboxes"
#[tokio::test]
async fn does_not_create_dispatches_when_contact_has_no_inboxes() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;
    h.lookup.set_ints(&[]);

    enroll(&h, enroll_params()).await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:303 "enrollContactInSequence > when nextStepId is null > reports the contact as enrolled even though no dispatch is created"
#[tokio::test]
async fn reports_the_contact_as_enrolled_even_though_no_dispatch() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    let result = enroll(
        &h,
        EnrollParams {
            next_step_id: None,
            ..enroll_params()
        },
    )
    .await;

    assert_eq!(result, EnrollResult { enrolled: true });
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:309 "enrollContactInSequence > when nextStepId is null > inserts the enrollment row"
#[tokio::test]
async fn inserts_the_enrollment_row() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(
        &h,
        EnrollParams {
            next_step_id: None,
            ..enroll_params()
        },
    )
    .await;

    assert_eq!(h.probe.count(StatementKind::Insert, TABLE), 1);
    assert_eq!(sequence::count_rows(&h.db, TABLE).await, 1);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:315 "enrollContactInSequence > when nextStepId is null > does not create a dispatch or schedule"
#[tokio::test]
async fn does_not_create_a_dispatch_or_schedule() {
    let h = enroll_harness(&[ENROLLMENT_1]).await;

    enroll(
        &h,
        EnrollParams {
            next_step_id: None,
            ..enroll_params()
        },
    )
    .await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

// ---------------------------------------------------------------------------
// enrollContactsInSequenceBulk
// ---------------------------------------------------------------------------

async fn bulk(h: &Harness, enrollments: Vec<BulkEnrollment>) {
    enroll_contacts_in_sequence_bulk(&h.ctx, &id(WS_1), enrollments)
        .await
        .expect("bulk enroll");
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:338 "enrollContactsInSequenceBulk > calls addToSchedule for each inbox per enrollment"
#[tokio::test]
async fn calls_addtoschedule_for_each_inbox_per_enrollment() {
    let h = enroll_harness(&[ENROLL_1]).await;
    h.lookup.set_ints(&[INBOX_1, INBOX_2]);

    bulk(
        &h,
        vec![bulk_one(CONTACT_1, Some(iso_ms(NOW)), Some(STEP_1))],
    )
    .await;

    assert_eq!(h.schedule.scheduled().len(), 2);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:359 "enrollContactsInSequenceBulk > does not schedule enrollments that already existed before bulk insert"
#[tokio::test]
async fn does_not_schedule_enrollments_that_already_existed_before() {
    let h = enroll_harness(&[ENROLL_1]).await;
    sequence::seed_enrollment(&h.db, json!({ "id": EXISTING_ENROLLMENT })).await;

    bulk(
        &h,
        vec![bulk_one(CONTACT_1, Some(iso_ms(NOW)), Some(STEP_1))],
    )
    .await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:368 "enrollContactsInSequenceBulk > calls addToSchedule once per enrollment when one inbox exists"
///
/// The mocked insert returned 2 rows for a 1-row input; with a real insert
/// the input carries the 2 enrollments (contact-1, contact-2).
#[tokio::test]
async fn calls_addtoschedule_once_per_enrollment_when_one_inbox() {
    let h = enroll_harness(&[ENROLL_1, ENROLL_2]).await;
    h.lookup.set_ints(&[INBOX_1]);

    bulk(
        &h,
        vec![
            bulk_one(CONTACT_1, Some(iso_ms(NOW)), Some(STEP_1)),
            bulk_one(CONTACT_2, Some(iso_ms(NOW)), Some(STEP_1)),
        ],
    )
    .await;

    // 2 enrollments × 1 inbox each
    assert_eq!(h.schedule.scheduled().len(), 2);
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:396 "enrollContactsInSequenceBulk > skips enrollment when nextStepId is null"
#[tokio::test]
async fn skips_enrollment_when_nextstepid_is_null() {
    let h = enroll_harness(&[ENROLL_1]).await;

    bulk(&h, vec![bulk_one(CONTACT_1, Some(iso_ms(NOW)), None)]).await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:413 "enrollContactsInSequenceBulk > skips enrollment when nextRunAt is null"
#[tokio::test]
async fn skips_enrollment_when_nextrunat_is_null() {
    let h = enroll_harness(&[ENROLL_1]).await;

    bulk(&h, vec![bulk_one(CONTACT_1, None, Some(STEP_1))]).await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:430 "enrollContactsInSequenceBulk > skips enrollment when both nextStepId and nextRunAt are null"
#[tokio::test]
async fn skips_enrollment_when_both_nextstepid_and_nextrunat_are() {
    let h = enroll_harness(&[ENROLL_1]).await;

    bulk(&h, vec![bulk_one(CONTACT_1, None, None)]).await;

    assert!(h.creator.calls().is_empty());
    assert!(h.schedule.scheduled().is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/enroll-contact.test.ts:447 "enrollContactsInSequenceBulk > passes correct params to createDispatch including enrollment id and runAt"
#[tokio::test]
async fn passes_correct_params_to_createdispatch_including() {
    let h = enroll_harness(&[ENROLL_42]).await;
    h.lookup.set_ints(&[INBOX_X]);
    let run_at = iso_ms("2024-06-01T09:00:00Z");

    bulk(&h, vec![bulk_one(CONTACT_1, Some(run_at), Some(STEP_99))]).await;

    assert_eq!(
        h.creator.calls(),
        vec![CreateDispatchParams {
            workspace_id: id(WS_1),
            contact_id: id(CONTACT_1),
            contact_inbox_id: id(INBOX_X),
            enrollment_id: id(ENROLL_42),
            run_at_ms: run_at,
            sequence_id: id(SEQ_1),
            step_id: id(STEP_99),
        }]
    );
}
