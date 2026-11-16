//! Ports for `contacts.bulkTagByStats` (P3.6-D2 slice contacts bulk):
//!
//! - `apps/worker/__tests__/bulk-tag-contacts-handler.test.ts` (4): adapted,
//!   the mocked `getContactIdsPage` / `bulkAttachToContacts` become stored
//!   broadcast recipients / sequence dispatches and the tags left on the
//!   contacts. n-a: "enables unsynced pair recovery on BullMQ retry attempts"
//!   (no channel tag sync is ported; `run` has no attempt count). The
//!   `restrictToAssignedUserId` scope of "dedupes repeated contact ids" is not
//!   applied (no member scope in the Rust API); the dedupe is asserted.
//! - `packages/analytics/__tests__/stats-contact-ids-page.repository.test.ts`
//!   (2): adapted, the asserted query-builder calls become the rows the page
//!   query returns (cursor, exclusions, event condition, order, limit).
//! - `apps/builder/__tests__/bulk-tag-stats-contacts-action-behavior.test.ts`
//!   (3): adapted, `queue.add("bulkTagContacts", ...)` becomes the job
//!   `prepare` returns and its `job_data()` payload.

mod common;

use chatbotx_business::contact::bulk_tag_stats::{
    BulkTagJob, BulkTagRequest, BulkTagSource, ContactIdRow, JOB_NAME, contact_ids_page, prepare,
    run,
};
use chatbotx_db::rusqlite::params;
use common::{
    TestDb, WS, contact, contact_inbox, conversation, inbox, linked_tag_ids, setup, tag,
    two_workspaces,
};
use serde_json::json;

const BROADCAST: i64 = 70;
const SEQUENCE: i64 = 80;
const STEP: i64 = 81;

/// Contacts 10..=14 (`WS`, contact inbox `40 + id`, conversation `60 + id`
/// on inbox 5); tags 1, 2; broadcast 70; sequence 80 with step 81.
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, 5, WS, "messenger")?;
        for id in 10..=14 {
            contact(tx, id, WS)?;
            contact_inbox(tx, 40 + id, id, 5, "messenger", &format!("psid-{id}"))?;
            conversation(tx, 60 + id, WS, id)?;
        }
        tag(tx, 1, WS, "one")?;
        tag(tx, 2, WS, "two")?;
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
               VALUES (?1, 'b', ?2, 'sent', 'now', 0, 'message', 'messenger')"#,
            params![BROADCAST, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 's', ?2)"#,
            params![SEQUENCE, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId") VALUES (?1, 0, 0, ?2)"#,
            params![STEP, SEQUENCE],
        )?;
        Ok(())
    })
    .await;
    db
}

/// A broadcast recipient row (`ContactOnBroadcast`).
async fn recipient(db: &TestDb, contact_id: i64, delivered: bool, failed: bool) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "deliveredAt", "failedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                BROADCAST,
                contact_id,
                40 + contact_id,
                60 + contact_id,
                delivered.then_some(1_000_i64),
                failed.then_some(2_000_i64)
            ],
        )?;
        Ok(())
    })
    .await;
}

/// A sequence-step dispatch `id` of a contact (`SequenceDispatch`, with the
/// contact's enrollment `contact_id + 100000`; one per contact and sequence).
async fn dispatch(db: &TestDb, id: i64, contact_id: i64, failed: bool, seen: bool) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT OR IGNORE INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status")
               VALUES (?1, ?2, ?3, ?4, 'completed')"#,
            params![contact_id + 100_000, contact_id, SEQUENCE, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "SequenceDispatch" ("id", "runAtMs", "status", "idempotencyKey", "deliveredAt", "seenAt", "failedAt",
                 "workspaceId", "sequenceId", "contactId", "contactInboxId", "stepId", "enrollmentId")
               VALUES (?1, 0, 'completed', ?2, 1000, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                id,
                format!("k-{id}"),
                seen.then_some(1_500_i64),
                failed.then_some(2_000_i64),
                WS,
                SEQUENCE,
                contact_id,
                40 + contact_id,
                STEP,
                contact_id + 100_000
            ],
        )?;
        Ok(())
    })
    .await;
}

fn job(source: BulkTagSource, tag_ids: &[i64], excluded: &[i64]) -> BulkTagJob {
    BulkTagJob {
        workspace_id: WS,
        requested_user_id: 1001,
        tag_ids: tag_ids.to_vec(),
        excluded_contact_ids: excluded.to_vec(),
        restrict_to_assigned_user_id: None,
        source,
    }
}

fn broadcast(event: &str) -> BulkTagSource {
    BulkTagSource::Broadcast {
        broadcast_id: BROADCAST,
        event_type: event.into(),
    }
}

fn sequence_step(event: &str) -> BulkTagSource {
    BulkTagSource::SequenceStep {
        sequence_id: SEQUENCE,
        step_id: STEP,
        event_type: event.into(),
    }
}

fn rows(pairs: &[(i64, i64)]) -> Vec<ContactIdRow> {
    pairs
        .iter()
        .map(|&(id, contact_id)| ContactIdRow { id, contact_id })
        .collect()
}

// --- bulk-tag-contacts-handler.test.ts ---------------------------------------

#[tokio::test]
async fn handle_bulk_tag_contacts_dispatches_broadcast_pages_and_forwards_exclusions() {
    let db = fixture().await;
    recipient(&db, 10, true, false).await;
    recipient(&db, 11, true, false).await;
    recipient(&db, 12, false, false).await;
    let linked = run(&db, &job(broadcast("message:sent"), &[1], &[11]))
        .await
        .expect("run");
    assert_eq!(linked, vec![(10, 1)]);
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1]);
    assert!(linked_tag_ids(&db, 11).await.is_empty());
    assert!(linked_tag_ids(&db, 12).await.is_empty());
}

#[tokio::test]
async fn handle_bulk_tag_contacts_dispatches_sequence_step_pages_with_workspace_and_step_scope() {
    let db = fixture().await;
    dispatch(&db, 900, 10, true, false).await;
    dispatch(&db, 901, 11, false, false).await;
    let linked = run(&db, &job(sequence_step("message:failed"), &[1], &[]))
        .await
        .expect("run");
    assert_eq!(linked, vec![(10, 1)]);
    assert!(linked_tag_ids(&db, 11).await.is_empty());
}

#[tokio::test]
async fn handle_bulk_tag_contacts_dedupes_repeated_contact_ids_within_a_page() {
    let db = fixture().await;
    dispatch(&db, 900, 10, false, false).await;
    dispatch(&db, 901, 10, false, false).await;
    let linked = run(&db, &job(sequence_step("message:sent"), &[1, 2], &[]))
        .await
        .expect("run");
    assert_eq!(linked, vec![(10, 1), (10, 2)]);
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1, 2]);
}

// --- stats-contact-ids-page.repository.test.ts -------------------------------

#[tokio::test]
async fn broadcast_query_uses_contact_id_cursor_exclusions_and_sent_event_condition() {
    let db = fixture().await;
    recipient(&db, 10, true, false).await;
    recipient(&db, 11, false, true).await;
    recipient(&db, 12, true, false).await;
    recipient(&db, 13, false, false).await;
    recipient(&db, 14, true, false).await;
    // Sent = delivered or failed; after contact 10; without 12; two rows.
    let page = contact_ids_page(
        &db,
        &job(broadcast("message:sent"), &[1], &[12]),
        Some(10),
        2,
    )
    .await
    .expect("page");
    assert_eq!(page, rows(&[(11, 11), (14, 14)]));
}

#[tokio::test]
async fn sequence_query_uses_dispatch_id_cursor_and_contact_id_exclusions() {
    let db = fixture().await;
    dispatch(&db, 900, 10, true, false).await;
    dispatch(&db, 901, 11, true, false).await;
    dispatch(&db, 902, 12, true, false).await;
    dispatch(&db, 903, 13, false, false).await;
    dispatch(&db, 904, 14, true, false).await;
    // Failed only; after dispatch 900; without contact 12.
    let page = contact_ids_page(
        &db,
        &job(sequence_step("message:failed"), &[1], &[12]),
        Some(900),
        25,
    )
    .await
    .expect("page");
    assert_eq!(page, rows(&[(901, 11), (904, 14)]));
}

// --- bulk-tag-stats-contacts-action-behavior.test.ts -------------------------

#[tokio::test]
async fn bulk_tag_stats_contacts_action_enqueues_a_broadcast_bulk_tag_job_without_request_time_audit()
 {
    let db = fixture().await;
    let request = BulkTagRequest {
        source: BulkTagSource::Broadcast {
            broadcast_id: 7,
            event_type: "message:sent".into(),
        },
        tags: vec!["one".into()],
        excluded_contact_ids: vec![12],
    };
    let job = prepare(&db, WS, 1001, &request, None)
        .await
        .expect("prepare")
        .expect("a job");
    assert_eq!(job.tag_ids, vec![1]);
    assert_eq!(JOB_NAME, "bulkTagContacts");
    assert_eq!(
        job.job_data(),
        json!({
            "type": "bulkTagContacts",
            "data": {
                "workspaceId": WS.to_string(),
                "requestedUserId": "1001",
                "tagIds": ["1"],
                "excludedContactIds": ["12"],
                "source": "broadcast",
                "broadcastId": "7",
                "eventType": "message:sent",
            },
        })
    );
}

#[tokio::test]
async fn bulk_tag_stats_contacts_action_propagates_assigned_contact_scope_for_sequence_step_jobs() {
    let db = fixture().await;
    let request = BulkTagRequest {
        source: BulkTagSource::SequenceStep {
            sequence_id: 8,
            step_id: 9,
            event_type: "message:failed".into(),
        },
        tags: vec!["two".into()],
        excluded_contact_ids: vec![],
    };
    let job = prepare(&db, WS, 1001, &request, Some(3003))
        .await
        .expect("prepare")
        .expect("a job");
    assert_eq!(
        job.job_data(),
        json!({
            "type": "bulkTagContacts",
            "data": {
                "workspaceId": WS.to_string(),
                "requestedUserId": "1001",
                "tagIds": ["2"],
                "excludedContactIds": [],
                "restrictToAssignedUserId": "3003",
                "source": "sequenceStep",
                "sequenceId": "8",
                "stepId": "9",
                "eventType": "message:failed",
            },
        })
    );
}

#[tokio::test]
async fn bulk_tag_stats_contacts_action_does_not_enqueue_when_tag_resolution_returns_no_active_tags()
 {
    let db = fixture().await;
    let request = BulkTagRequest {
        source: broadcast("message:sent"),
        tags: vec!["  ".into()],
        excluded_contact_ids: vec![],
    };
    let job = prepare(&db, WS, 1001, &request, None)
        .await
        .expect("prepare");
    assert_eq!(job, None);
}
