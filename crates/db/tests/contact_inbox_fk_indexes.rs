//! Port of `packages/database/__tests__/contact-inbox-fk-indexes.test.ts`
//! (P2.4 spec section 3.1, rows 2.1-2.3): the `:46` `test.each` over `TARGETS`.
//! The migration-text cases (`:63` x3, `:124`, `:135`) are not-applicable (hash partitions,
//! `CONCURRENTLY`, re-runnable unwrapped migration; see `conversion/specs/P2.4-test-map.tsv`).
//! Class C: runs against a migrated SQLite file.

mod common;

use common::{fresh_db, index, index_columns};

/// The `:46` body: the index exists and its key is exactly `["contactInboxId"]`.
async fn assert_leading_contact_inbox_index(table: &str, name: &str) {
    let db = fresh_db().await;
    index(&db, table, name).await;
    assert_eq!(
        index_columns(&db, name).await,
        [Some("contactInboxId".to_owned())]
    );
}

/// Port of: packages/database/__tests__/contact-inbox-fk-indexes.test.ts:46 "contactInboxId FK cascade indexes > ContactOnBroadcast schema declares a leading index on contactInboxId"
#[tokio::test]
async fn contact_on_broadcast_schema_declares_a_leading_index_on_contact_inbox_id() {
    assert_leading_contact_inbox_index(
        "ContactOnBroadcast",
        "ContactOnBroadcast_contactInboxId_idx",
    )
    .await;
}

/// Port of: packages/database/__tests__/contact-inbox-fk-indexes.test.ts:46 "contactInboxId FK cascade indexes > SequenceDispatch schema declares a leading index on contactInboxId"
#[tokio::test]
async fn sequence_dispatch_schema_declares_a_leading_index_on_contact_inbox_id() {
    assert_leading_contact_inbox_index("SequenceDispatch", "SequenceDispatch_contactInboxId_idx")
        .await;
}

/// Port of: packages/database/__tests__/contact-inbox-fk-indexes.test.ts:46 "contactInboxId FK cascade indexes > AutomationThrottle schema declares a leading index on contactInboxId"
#[tokio::test]
async fn automation_throttle_schema_declares_a_leading_index_on_contact_inbox_id() {
    assert_leading_contact_inbox_index(
        "AutomationThrottle",
        "AutomationThrottle_contactInboxId_idx",
    )
    .await;
}
