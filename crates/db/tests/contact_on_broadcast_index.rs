//! Port of `packages/database/__tests__/contact-on-broadcast-index.test.ts`
//! (P2.4 spec section 3.1, rows 3.1-3.3). Class C: runs against a migrated SQLite file.
//! Rows 3.2 and 3.3 read migration-file text in the original; the consolidated schema has no
//! such files, so they are end-state replacements (DECISIONS "P2.3 disputed tests").

mod common;

use common::{column, fresh_db, index, index_columns, indexes, schema_sql, table_names};

const UNSENT_ORDER_IDX: &str = "ContactOnBroadcast_unsent_order_idx";
/// The predicate of the original (`"sent" = false AND "failedAt" IS NULL`) in SQLite form.
const UNSENT_PREDICATE: &str = r#"WHERE "sent" = 0 AND "failedAt" IS NULL"#;

/// `ContactOnBroadcast_unsent_order_idx` exists, is partial, has the key
/// `[broadcastId, contactInboxId]` and the unsent predicate.
async fn assert_unsent_order_index(db: &chatbotx_db::Db) {
    let idx = index(db, "ContactOnBroadcast", UNSENT_ORDER_IDX).await;
    assert!(idx.partial, "{UNSENT_ORDER_IDX} must have a WHERE clause");
    assert_eq!(
        index_columns(db, UNSENT_ORDER_IDX).await,
        [
            Some("broadcastId".to_owned()),
            Some("contactInboxId".to_owned())
        ]
    );
    let sql = schema_sql(db, "index", UNSENT_ORDER_IDX)
        .await
        .expect("index SQL");
    assert!(
        sql.trim_end().ends_with(UNSENT_PREDICATE),
        "{UNSENT_ORDER_IDX} must end with {UNSENT_PREDICATE}: {sql}"
    );
}

/// The old single-column unsent index was dropped by a later migration.
async fn assert_old_unsent_index_is_gone(db: &chatbotx_db::Db) {
    let tables = table_names(db).await;
    assert!(
        tables.iter().any(|t| t == "ContactOnBroadcast"),
        "table ContactOnBroadcast is missing"
    );
    let names: Vec<String> = indexes(db, "ContactOnBroadcast")
        .await
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert!(
        !names.iter().any(|n| n == "ContactOnBroadcast_unsent_idx"),
        "ContactOnBroadcast_unsent_idx must not exist: {names:?}"
    );
}

/// Port of: packages/database/__tests__/contact-on-broadcast-index.test.ts:23 "ContactOnBroadcast unsent-batch partial index > schema declares the ordered partial index the batch scan depends on"
#[tokio::test]
async fn schema_declares_the_ordered_partial_index_the_batch_scan_depends_on() {
    let db = fresh_db().await;
    assert_unsent_order_index(&db).await;
}

/// Port of: packages/database/__tests__/contact-on-broadcast-index.test.ts:37 "ContactOnBroadcast unsent-batch partial index > migration creates the index idempotently with the matching predicate"
///
/// Replacement (end state of migration `20260902054310_add_broadcast_soft_delete_and_resume`):
/// `Broadcast` has the nullable `deletedAt` and `resumeCount` NOT NULL default 0, the partial
/// `Broadcast_deletedAt_idx` exists, and the `ContactOnBroadcast_unsent_idx` that this
/// migration created is gone (a later migration replaced it with `_unsent_order_idx`).
#[tokio::test]
async fn soft_delete_columns_and_index_exist_and_the_old_unsent_index_is_gone() {
    let db = fresh_db().await;

    let deleted_at = column(&db, "Broadcast", "deletedAt").await;
    assert!(!deleted_at.notnull, "Broadcast.deletedAt must be nullable");
    assert_eq!(deleted_at.decl_type, "INTEGER");

    let resume_count = column(&db, "Broadcast", "resumeCount").await;
    assert!(
        resume_count.notnull,
        "Broadcast.resumeCount must be NOT NULL"
    );
    assert_eq!(resume_count.dflt_value.as_deref(), Some("0"));

    let deleted_idx = index(&db, "Broadcast", "Broadcast_deletedAt_idx").await;
    assert!(
        deleted_idx.partial,
        "Broadcast_deletedAt_idx must be partial"
    );

    assert_old_unsent_index_is_gone(&db).await;
}

/// Port of: packages/database/__tests__/contact-on-broadcast-index.test.ts:52 "ContactOnBroadcast unsent-batch partial index > send-order-indexes migration builds the ContactOnBroadcast index per partition"
///
/// Replacement (end state of migration `20260920111706_broadcast_send_order_indexes`):
/// `ContactInbox_inboxId_id_idx` is `[inboxId, id]`, the unsent-order index is as in `:23`,
/// and the old `ContactOnBroadcast_unsent_idx` (the final `DROP`) is gone. The partition count,
/// `ON ONLY` parent, `ATTACH PARTITION` loop and `CONCURRENTLY` rules are not-applicable
/// (no hash partitions in SQLite).
#[tokio::test]
async fn send_order_indexes_end_state() {
    let db = fresh_db().await;

    index(&db, "ContactInbox", "ContactInbox_inboxId_id_idx").await;
    assert_eq!(
        index_columns(&db, "ContactInbox_inboxId_id_idx").await,
        [Some("inboxId".to_owned()), Some("id".to_owned())]
    );

    assert_unsent_order_index(&db).await;
    assert_old_unsent_index_is_gone(&db).await;
}
