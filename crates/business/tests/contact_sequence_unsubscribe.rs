//! Port of `packages/business/__tests__/contact-sequence-unsubscribe.test.ts`
//! (P3.6-D2 slice contacts bulk). Adapted: the mocked `sequenceModel.findMany`,
//! `contactService.findManyByIds` and the spied
//! `removeContactSequencesForContacts` become stored sequences, contacts and
//! `ContactOnSequence` rows; the asserted calls become the rows left and the
//! outcome. The chunk count is not observable; the outcome of a 1001-id call
//! whose only stored contact sits in the second chunk is.

mod common;

use chatbotx_business::contact_sequence::{SubscribeOutcome, unsubscribe_contacts};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, setup, two_workspaces};

/// Sequences 31 (`WS`) and 39 (`OTHER_WS`); contacts 101, 102, 103 (`WS`),
/// each enrolled in 31.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for (id, ws) in [(31, WS), (39, OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![id, format!("seq-{id}"), ws],
            )?;
        }
        for id in [101, 102, 103] {
            contact(tx, id, WS)?;
            tx.execute(
                r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status")
                   VALUES (?1, ?2, 31, ?3, 'active')"#,
                params![id + 5000, id, WS],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

async fn enrolled(db: &TestDb) -> Vec<i64> {
    db.read(|conn| {
        let mut stmt =
            conn.prepare(r#"SELECT "contactId" FROM "ContactOnSequence" ORDER BY "contactId""#)?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<i64>, _>>()?)
    })
    .await
    .expect("enrollments")
}

#[tokio::test]
async fn contact_sequence_service_unsubscribe_contacts_throws_and_removes_nothing_when_a_sequence_id_belongs_to_another_workspace()
 {
    let db = db().await;
    let error = unsubscribe_contacts(&db, WS, &[101], &[31, 39])
        .await
        .expect_err("foreign sequence");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Sequence not found");
    assert_eq!(enrolled(&db).await, vec![101, 102, 103]);
}

#[tokio::test]
async fn contact_sequence_service_unsubscribe_contacts_removes_only_contacts_that_resolve_and_reports_the_rest_as_skipped()
 {
    let db = db().await;
    // 999 does not resolve in the workspace; 102 is not asked for.
    let outcome = unsubscribe_contacts(&db, WS, &[101, 999, 103], &[31])
        .await
        .expect("unsubscribe");
    assert_eq!(
        outcome,
        SubscribeOutcome {
            processed_contact_ids: vec![101, 103],
            skipped_contact_ids: vec![999],
        }
    );
    assert_eq!(enrolled(&db).await, vec![102]);
}

#[tokio::test]
async fn contact_sequence_service_unsubscribe_contacts_chunks_contact_ids_at_1000_and_skips_the_removal_call_for_an_empty_chunk()
 {
    let db = db().await;
    // 1001 ids: the first chunk (1000 unknown ids) resolves nothing, the
    // second holds the stored contact 102.
    let mut ids: Vec<i64> = (0..1000).map(|i| 1_000_000 + i).collect();
    ids.push(102);
    let outcome = unsubscribe_contacts(&db, WS, &ids, &[31])
        .await
        .expect("unsubscribe");
    assert_eq!(outcome.processed_contact_ids, vec![102]);
    assert_eq!(outcome.skipped_contact_ids.len(), 1000);
    assert_eq!(enrolled(&db).await, vec![101, 103]);
}
