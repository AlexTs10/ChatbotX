//! Port of `packages/database/__tests__/partition-migrations.test.ts`
//! (P2.4 spec section 3.1, rows 4.1-4.4). Class C: runs against a migrated SQLite file.
//! The original reads the text of the hash-partition data-copy migrations. SQLite has no
//! partitions and the consolidated schema has no copy, so the cases check the subject of each
//! test on the end-state schema (DECISIONS "P2.3 disputed tests"); `:63` is not-applicable.

mod common;

use common::{column, columns, fresh_db, index, index_columns, primary_key, table_names};

/// `pragma_table_xinfo.hidden` of a STORED generated column.
const HIDDEN_STORED_GENERATED: i64 = 3;

/// Port of: packages/database/__tests__/partition-migrations.test.ts:36 "partition migrations > ContactOnBroadcast copy uses explicit columns and excludes generated isRead"
///
/// Replacement: the subject (never write the generated `isRead`) still holds. `isRead` is a
/// STORED generated column, `broadcastId` and `sent` are ordinary columns, and an UPDATE of
/// `isRead` is refused by SQLite.
#[tokio::test]
async fn contact_on_broadcast_is_read_is_generated_and_cannot_be_written() {
    let db = fresh_db().await;

    let is_read = column(&db, "ContactOnBroadcast", "isRead").await;
    assert_eq!(
        is_read.hidden, HIDDEN_STORED_GENERATED,
        "isRead must be STORED generated"
    );
    for name in ["broadcastId", "sent"] {
        let col = column(&db, "ContactOnBroadcast", name).await;
        assert_eq!(
            col.hidden, 0,
            "ContactOnBroadcast.{name} must be an ordinary column"
        );
    }

    let err = db
        .write(|tx| {
            tx.execute(r#"UPDATE "ContactOnBroadcast" SET "isRead" = 1"#, [])?;
            Ok(())
        })
        .await
        .expect_err("an UPDATE of the generated isRead must fail");
    assert!(
        err.message().contains("cannot UPDATE generated column"),
        "unexpected error: {}",
        err.message()
    );
}

/// Port of: packages/database/__tests__/partition-migrations.test.ts:48 "partition migrations > SequenceDispatch copy excludes generated isRead and stale ContactOnSequence columns"
///
/// Replacement: `SequenceDispatch.isRead` is STORED generated, `status` and `workspaceId` are
/// ordinary columns, and `ContactOnSequence` has no stale `errorCount` column.
#[tokio::test]
async fn sequence_dispatch_is_read_is_generated_and_contact_on_sequence_has_no_error_count() {
    let db = fresh_db().await;

    let is_read = column(&db, "SequenceDispatch", "isRead").await;
    assert_eq!(
        is_read.hidden, HIDDEN_STORED_GENERATED,
        "isRead must be STORED generated"
    );
    for name in ["status", "workspaceId"] {
        let col = column(&db, "SequenceDispatch", name).await;
        assert_eq!(
            col.hidden, 0,
            "SequenceDispatch.{name} must be an ordinary column"
        );
    }

    let enrollment = columns(&db, "ContactOnSequence").await;
    assert!(!enrollment.is_empty(), "table ContactOnSequence is missing");
    assert!(
        !enrollment.iter().any(|c| c.name == "errorCount"),
        "ContactOnSequence must not have errorCount"
    );
}

/// Port of: packages/database/__tests__/partition-migrations.test.ts:83 "partition migrations > SequenceDispatch uses workspace hash partitioning without idempotency registry"
///
/// Replacement: the idempotency key is unique per workspace
/// (`SequenceDispatch_idempotencyKey_key` = `[idempotencyKey, workspaceId]`), both pending
/// indexes exist and are partial, and there is no `SequenceDispatchIdempotency` registry table.
/// The PK is `[id]` (DECISIONS P2.2 / P2.4 item 2: the partition column left the PK).
/// `PARTITION BY HASH` is not-applicable.
#[tokio::test]
async fn sequence_dispatch_has_workspace_scoped_idempotency_and_pending_indexes() {
    let db = fresh_db().await;

    let key = index(
        &db,
        "SequenceDispatch",
        "SequenceDispatch_idempotencyKey_key",
    )
    .await;
    assert!(
        key.unique,
        "SequenceDispatch_idempotencyKey_key must be unique"
    );
    assert_eq!(
        index_columns(&db, "SequenceDispatch_idempotencyKey_key").await,
        [
            Some("idempotencyKey".to_owned()),
            Some("workspaceId".to_owned())
        ]
    );

    for name in [
        "SequenceDispatch_pending_runAtMs_idx",
        "SequenceDispatch_pending_bucket_runAtMs_idx",
    ] {
        let idx = index(&db, "SequenceDispatch", name).await;
        assert!(idx.partial, "{name} must be partial");
    }

    let tables = table_names(&db).await;
    assert!(
        !tables.iter().any(|t| t == "SequenceDispatchIdempotency"),
        "SequenceDispatchIdempotency must not exist"
    );

    assert_eq!(primary_key(&db, "SequenceDispatch").await, ["id"]);
}
