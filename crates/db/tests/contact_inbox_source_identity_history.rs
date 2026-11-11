//! Port of `packages/database/__tests__/contact-inbox-source-identity-history.test.ts`
//! (P2.3 spec section 5.1, rows 5.1-5.2). Class C: runs against a migrated SQLite file.

mod common;

use common::{
    column, fresh_db, index_columns, index_sqls, indexes, insert_contact_inbox, seed, table_sql,
};

/// Port of: packages/database/__tests__/contact-inbox-source-identity-history.test.ts:28 "ContactInbox sourceIdentityHistory > schema declares nullable jsonb history without a default"
///
/// jsonb -> `TEXT` with a `json_valid` CHECK: nullable, no default, invalid JSON refused.
#[tokio::test]
async fn schema_declares_nullable_jsonb_history_without_a_default() {
    let db = fresh_db().await;
    let col = column(&db, "ContactInbox", "sourceIdentityHistory").await;
    assert_eq!(col.decl_type, "TEXT");
    assert!(!col.notnull);
    assert_eq!(col.dflt_value, None);
    let sql = table_sql(&db, "ContactInbox")
        .await
        .expect("ContactInbox sql");
    assert!(
        sql.contains(r#"json_valid("sourceIdentityHistory")"#),
        "no json_valid CHECK in: {sql}"
    );

    let s = seed(&db).await;
    let refused = db
        .write(move |tx| insert_contact_inbox(tx, s, 8001, "src-1", None, Some("{")))
        .await
        .expect_err("invalid JSON must be refused");
    assert_eq!(refused.sqlstate(), Some("23514"));

    db.write(move |tx| {
        insert_contact_inbox(tx, s, 8002, "src-2", None, Some(r#"[{"sourceId":"old"}]"#))
    })
    .await
    .expect("valid JSON must be stored");
    db.write(move |tx| insert_contact_inbox(tx, s, 8003, "src-3", None, None))
        .await
        .expect("NULL history must be stored");
}

/// Port of: packages/database/__tests__/contact-inbox-source-identity-history.test.ts:40 "ContactInbox sourceIdentityHistory > migration adds only the nullable history column restart-safely"
///
/// Replacement (no per-column migration file in SQLite): the column has no default and no
/// index covers it.
#[tokio::test]
async fn history_column_has_no_default_and_no_index() {
    let db = fresh_db().await;
    let col = column(&db, "ContactInbox", "sourceIdentityHistory").await;
    assert_eq!(col.dflt_value, None);

    let idx = indexes(&db, "ContactInbox").await;
    assert!(!idx.is_empty(), "ContactInbox has no indexes at all");
    for index in idx {
        let cols = index_columns(&db, &index.name).await;
        assert!(
            !cols
                .iter()
                .any(|c| c.as_deref() == Some("sourceIdentityHistory")),
            "index {} covers sourceIdentityHistory",
            index.name
        );
    }
    for (name, sql) in index_sqls(&db, "ContactInbox").await {
        assert!(
            !sql.contains("sourceIdentityHistory"),
            "index {name} uses sourceIdentityHistory: {sql}"
        );
    }
}
