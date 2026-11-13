//! Port of the `contactSequenceService.subscribeContacts` part of
//! `packages/business/__tests__/contact-sequence-subscribe.test.ts` (P3.6-D2
//! slice contact sequences). Adapted: the mocked `sequenceModel.findMany`,
//! `contactService.findManyByIds`, `getExistingEnrollments` and
//! `enrollContactsInSequenceBulk` become stored sequences, contacts and
//! `ContactOnSequence` rows; the asserted calls become the stored rows and
//! the outcome. `subscribeFromFlow` / `subscribeContactForTrigger` belong to
//! the flow and trigger ports.

mod common;

use chatbotx_business::contact_sequence::subscribe_contacts;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, setup, two_workspaces};

/// Sequences 31, 32 (`WS`) and 39 (`OTHER_WS`); contacts 101, 102 (`WS`).
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for (id, ws) in [(31, WS), (32, WS), (39, OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![id, format!("seq-{id}"), ws],
            )?;
        }
        contact(tx, 101, WS)?;
        contact(tx, 102, WS)?;
        Ok(())
    })
    .await;
    db
}

async fn enrollments(db: &TestDb) -> Vec<(i64, i64)> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "contactId", "sequenceId" FROM "ContactOnSequence" ORDER BY "contactId", "sequenceId""#,
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<(i64, i64)>, _>>()?)
    })
    .await
    .expect("enrollments")
}

#[tokio::test]
async fn contact_sequence_service_subscribe_contacts_throws_and_inserts_nothing_when_a_sequence_id_belongs_to_another_workspace()
 {
    let db = db().await;
    let error = subscribe_contacts(&db, WS, &[101], &[31, 39])
        .await
        .expect_err("foreign sequence");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Sequence not found");
    assert!(enrollments(&db).await.is_empty());
}

#[tokio::test]
async fn contact_sequence_service_subscribe_contacts_chunks_contact_ids_at_1000_1001_ids_means_2_find_many_by_ids_calls()
 {
    let db = db().await;
    // 1001 ids: the two stored contacts sit in different chunks.
    let mut ids: Vec<i64> = (1..=999).map(|i| 1_000_000 + i).collect();
    ids.insert(0, 101);
    ids.push(102);
    assert_eq!(ids.len(), 1001);
    let outcome = subscribe_contacts(&db, WS, &ids, &[31])
        .await
        .expect("subscribe");
    assert_eq!(outcome.processed_contact_ids, vec![101, 102]);
    assert_eq!(outcome.skipped_contact_ids.len(), 999);
    assert_eq!(enrollments(&db).await, vec![(101, 31), (102, 31)]);
}

#[tokio::test]
async fn contact_sequence_service_subscribe_contacts_skips_an_already_enrolled_contact_sequence_pair_instead_of_duplicating_it()
 {
    let db = db().await;
    subscribe_contacts(&db, WS, &[101], &[31])
        .await
        .expect("first");
    subscribe_contacts(&db, WS, &[101, 102], &[31])
        .await
        .expect("second");
    assert_eq!(enrollments(&db).await, vec![(101, 31), (102, 31)]);
}

#[tokio::test]
async fn contact_sequence_service_subscribe_contacts_reports_unresolved_contact_ids_as_skipped() {
    let db = db().await;
    let outcome = subscribe_contacts(&db, WS, &[101, 999], &[31])
        .await
        .expect("subscribe");
    assert_eq!(outcome.processed_contact_ids, vec![101]);
    assert_eq!(outcome.skipped_contact_ids, vec![999]);
}
