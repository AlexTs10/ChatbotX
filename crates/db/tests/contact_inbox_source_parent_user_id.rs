//! Port of `packages/database/__tests__/contact-inbox-source-parent-user-id.test.ts`
//! (P2.3 spec section 5.1, rows 6.1-6.2). Class C: runs against a migrated SQLite file.

mod common;

use chatbotx_db::schema::CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY;
use common::{column, fresh_db, index, index_columns, insert_contact_inbox, seed};

/// Port of: packages/database/__tests__/contact-inbox-source-parent-user-id.test.ts:29 "ContactInbox sourceParentUserId > schema declares a nullable matching-only identity with a partial unique index"
#[tokio::test]
async fn schema_declares_a_nullable_matching_only_identity_with_a_partial_unique_index() {
    assert_eq!(
        CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY,
        "ContactInbox_inboxId_sourceParentUserId_key"
    );

    let db = fresh_db().await;
    let col = column(&db, "ContactInbox", "sourceParentUserId").await;
    assert!(!col.notnull);

    let idx = index(&db, "ContactInbox", CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY).await;
    assert!(idx.unique, "{} must be unique", idx.name);
    assert!(idx.partial, "{} must have a WHERE clause", idx.name);
    assert_eq!(
        index_columns(&db, CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY).await,
        [
            Some("inboxId".to_owned()),
            Some("sourceParentUserId".to_owned())
        ]
    );
}

/// Port of: packages/database/__tests__/contact-inbox-source-parent-user-id.test.ts:50 "ContactInbox sourceParentUserId > migration adds the column and creates the partial index restart-safely"
///
/// Replacement (CONCURRENTLY / invalid-index cleanup are Postgres build mechanics): the
/// partial unique index lets many NULLs in and refuses a duplicate non-NULL value with
/// SQLSTATE 23505 and its constraint name.
#[tokio::test]
async fn partial_unique_index_allows_many_nulls_and_rejects_duplicates() {
    let db = fresh_db().await;
    let s = seed(&db).await;

    db.write(move |tx| {
        insert_contact_inbox(tx, s, 8101, "src-1", None, None)?;
        insert_contact_inbox(tx, s, 8102, "src-2", None, None)?;
        insert_contact_inbox(tx, s, 8103, "src-3", Some("parent-1"), None)?;
        Ok(())
    })
    .await
    .expect("two NULL sourceParentUserId rows and one non-NULL row must be stored");

    let err = db
        .write(move |tx| insert_contact_inbox(tx, s, 8104, "src-4", Some("parent-1"), None))
        .await
        .expect_err("a duplicate (inboxId, sourceParentUserId) must be refused");
    assert_eq!(err.sqlstate(), Some("23505"));
    assert_eq!(
        err.constraint(),
        Some(CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY)
    );
    assert_eq!(err.table(), Some("ContactInbox"));
}
