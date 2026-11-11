//! Port of `packages/database/__tests__/workspace-purge-coverage.test.ts`, the part-3 case
//! (P2.5 spec section 3.1, row 3.1). The FK-index case (`:188`) is in
//! `workspace_purge_coverage.rs` (P2.3). Class C: the original reads the Drizzle snapshot;
//! the port reads the migrated SQLite catalog.

mod common;

use common::{fks, fresh_db, indexes, primary_key};

/// Port of: packages/database/__tests__/workspace-purge-coverage.test.ts:224 "workspace purge coverage > keeps ResourceDeletionLog primary-key only so it outlives its workspace"
///
/// No FKs, no indexes (an `INTEGER PRIMARY KEY` has no autoindex, so an empty
/// `pragma_index_list` also means "no uniques"), and PK columns `["id"]`.
#[tokio::test]
async fn keeps_resource_deletion_log_primary_key_only_so_it_outlives_its_workspace() {
    let db = fresh_db().await;
    let foreign_keys = fks(&db, "ResourceDeletionLog").await;
    assert!(foreign_keys.is_empty(), "no FKs: {foreign_keys:?}");
    let table_indexes = indexes(&db, "ResourceDeletionLog").await;
    assert!(table_indexes.is_empty(), "no indexes: {table_indexes:?}");
    assert_eq!(primary_key(&db, "ResourceDeletionLog").await, ["id"]);
}
