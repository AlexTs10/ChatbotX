//! Port of `packages/business/__tests__/contact-sequence.service.test.ts`
//! (P3.6-D2 slice contact sequences), the remove / update cases. Adapted:
//! the mocked transaction, `contactsOnSequenceModel` queries and
//! `cancelPendingDispatches` become stored rows; the asserted call order
//! becomes the stored rows after the call. The scheduler-removal, event and
//! client-option cases are not-applicable (no scheduler or event bus in the
//! port; no `client` / `useTransaction` options).

mod common;

use chatbotx_business::contact_sequence::{
    list_by_contact_id, remove_contact_sequences_for_contacts, update_contact_sequences,
};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, setup, two_workspaces};

/// Sequences 31 "alpha", 32 "beta", 33 "gamma" (`WS`), 39 (`OTHER_WS`);
/// contact 101 enrolled in 31 and 32.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for (id, name, ws) in [(31, "alpha", WS), (32, "beta", WS), (33, "gamma", WS), (39, "x", OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![id, name, ws],
            )?;
        }
        contact(tx, 101, WS)?;
        for (id, sequence) in [(501, 31), (502, 32)] {
            tx.execute(
                r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status") VALUES (?1, 101, ?2, ?3, 'active')"#,
                params![id, sequence, WS],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

async fn names(db: &TestDb) -> Vec<String> {
    let mut names: Vec<String> = list_by_contact_id(db, WS, 101)
        .await
        .expect("list")
        .into_iter()
        .map(|row| row.sequence_name)
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn contact_sequence_service_removes_contact_sequences_in_a_transaction_before_removing_scheduler_entries()
 {
    let db = db().await;
    remove_contact_sequences_for_contacts(&db, WS, &[101], &[31, 33])
        .await
        .expect("remove");
    assert_eq!(names(&db).await, vec!["beta".to_string()]);
}

#[tokio::test]
async fn contact_sequence_service_updates_contact_sequences_in_one_transaction_and_defers_scheduler_removal_until_add_succeeds()
 {
    let db = db().await;
    let rows = update_contact_sequences(&db, WS, 101, &[32, 33])
        .await
        .expect("update");
    let mut ids: Vec<i64> = rows.iter().map(|row| row.sequence_id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![32, 33]);
    assert_eq!(
        names(&db).await,
        vec!["beta".to_string(), "gamma".to_string()]
    );
    // A sequence to add of another workspace: 404, nothing removed.
    let error = update_contact_sequences(&db, WS, 101, &[39])
        .await
        .expect_err("foreign");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(
        names(&db).await,
        vec!["beta".to_string(), "gamma".to_string()]
    );
}
