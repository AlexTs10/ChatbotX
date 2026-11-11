//! Port of `packages/database/__tests__/contact-inbox-thread-owner-app-columns.test.ts`
//! (P2.3 spec section 5.1, row 7.1, `test.each` x 2). Class C: runs against a migrated SQLite file.

mod common;

use common::{column, fresh_db};

/// `column.notNull` false, `dataType` string (TEXT), `hasDefault` false, name equal.
async fn assert_nullable_text_without_default(name: &str) {
    let db = fresh_db().await;
    let col = column(&db, "ContactInbox", name).await;
    assert!(!col.notnull, "{name} must be nullable");
    assert_eq!(col.decl_type, "TEXT");
    assert_eq!(col.dflt_value, None, "{name} must have no default");
    assert_eq!(col.name, name);
}

/// Port of: packages/database/__tests__/contact-inbox-thread-owner-app-columns.test.ts:8 "ContactInbox thread owner app id columns > threadOwnerAppId is a nullable text column without a default"
#[tokio::test]
async fn thread_owner_app_id_is_a_nullable_text_column_without_a_default() {
    assert_nullable_text_without_default("threadOwnerAppId").await;
}

/// Port of: packages/database/__tests__/contact-inbox-thread-owner-app-columns.test.ts:8 "ContactInbox thread owner app id columns > threadPreviousOwnerAppId is a nullable text column without a default"
#[tokio::test]
async fn thread_previous_owner_app_id_is_a_nullable_text_column_without_a_default() {
    assert_nullable_text_without_default("threadPreviousOwnerAppId").await;
}
