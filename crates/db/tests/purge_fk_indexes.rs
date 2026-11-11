//! Port of `packages/database/__tests__/purge-fk-indexes.test.ts`, the `ContactToTag` case
//! of the `normalTargets` `test.each` (P2.3 spec section 5.1, row 13.1). The other 9 cases and
//! the migration-text cases belong to P2.4/P2.5.
//! Class C: runs against a migrated SQLite file.

mod common;

use common::{fresh_db, index, index_columns};

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > ContactToTag declares ContactToTag_tagId_idx with the exact FK prefix"
#[tokio::test]
async fn contact_to_tag_declares_contact_to_tag_tag_id_idx_with_the_exact_fk_prefix() {
    let db = fresh_db().await;
    let columns = [Some("tagId".to_owned())];
    index(&db, "ContactToTag", "ContactToTag_tagId_idx").await;
    let actual = index_columns(&db, "ContactToTag_tagId_idx").await;
    assert_eq!(actual.get(..columns.len()), Some(&columns[..]), "FK prefix");
    assert_eq!(actual, columns);
}
