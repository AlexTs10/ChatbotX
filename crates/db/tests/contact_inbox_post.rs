//! Port of `packages/database/__tests__/integration/contact-inbox-post.test.ts`, catalog case
//! only (P2.3 spec section 5.1, row 11.1). The pruning / lock / snapshot cases belong to P3.
//! Class C: runs against a migrated SQLite file.

mod common;

use common::{fresh_db, index, primary_key, table_sql};

/// Port of: packages/database/__tests__/integration/contact-inbox-post.test.ts:203 "ContactInboxPost partition catalog > uses 64 workspace hash partitions with the planned key and indexes"
///
/// Adapted: one SQLite table (DECISIONS P2.2), so the partition half (64 children, hash by
/// `workspaceId`) is not applicable. SQLite names the PK index `sqlite_autoindex_...`, so
/// `ContactInboxPost_pkey` is checked as the constraint name in the table SQL.
#[tokio::test]
async fn contact_inbox_post_key_and_indexes() {
    let db = fresh_db().await;
    assert_eq!(primary_key(&db, "ContactInboxPost").await, ["workspaceId", "contactInboxId", "postId"]);
    let sql = table_sql(&db, "ContactInboxPost").await.expect("ContactInboxPost is missing");
    assert!(sql.contains(r#"CONSTRAINT "ContactInboxPost_pkey" PRIMARY KEY"#), "{sql}");
    index(&db, "ContactInboxPost", "ContactInboxPost_workspaceId_postId_idx").await;
}
