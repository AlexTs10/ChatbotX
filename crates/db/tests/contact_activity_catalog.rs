//! Port of `packages/database/__tests__/integration/contact-activity-catalog.test.ts`
//! (P2.3 spec section 5.1, rows 9.1, 9.2, 9.4). Class C: runs against a migrated SQLite file.
//!
//! DECISIONS P2.2: a Postgres HASH(workspaceId) x 64 partitioned table is ONE SQLite table
//! with the same primary key. The partition halves (child count 64, partition key, strategy
//! `h`, 64 leaves per index) and the `fillfactor=80` case (`:118`) are not applicable.
//! `AnalyticsEmailTopic` (`:141`) is a P2.5 table.

mod common;

use chatbotx_db::rusqlite::params;
use common::{fresh_db, index, index_columns, primary_key, schema_sql, table_sql};

/// `primaryKey` of the frozen `TABLES` list + the PK constraint text.
async fn assert_primary_key(table: &str, expected: &[&str]) {
    let db = fresh_db().await;
    assert_eq!(primary_key(&db, table).await, expected, "{table} primary key");
    let sql = table_sql(&db, table).await.unwrap_or_else(|| panic!("{table} is missing"));
    let constraint = format!("CONSTRAINT \"{table}_pkey\" PRIMARY KEY");
    assert!(sql.contains(&constraint), "{table} has no `{constraint}`: {sql}");
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:62 "contact activity partition catalog > ContactActivity is hash partitioned by workspaceId"
#[tokio::test]
async fn contact_activity_primary_key() {
    assert_primary_key(
        "ContactActivity",
        &["workspaceId", "contactId", "activityType", "referenceType", "parentReferenceId", "referenceId"],
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:62 "contact activity partition catalog > ContactKeywordMessage is hash partitioned by workspaceId"
#[tokio::test]
async fn contact_keyword_message_primary_key() {
    assert_primary_key("ContactKeywordMessage", &["workspaceId", "contactId", "messageId"]).await;
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:62 "contact activity partition catalog > ContactState is hash partitioned by workspaceId"
#[tokio::test]
async fn contact_state_primary_key() {
    assert_primary_key("ContactState", &["workspaceId", "contactId"]).await;
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:100 "contact activity partition catalog > ContactActivity has its named indexes on all 64 leaves"
#[tokio::test]
async fn contact_activity_has_its_named_indexes() {
    let db = fresh_db().await;
    index(&db, "ContactActivity", "ContactActivity_reference_idx").await;
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:100 "contact activity partition catalog > ContactKeywordMessage has its named indexes on all 64 leaves"
///
/// `text_pattern_ops` -> plain BINARY index `ContactKeywordMessage_text_prefix_idx`;
/// `gin_trgm_ops` (`ContactKeywordMessage_text_trgm_idx`) -> the FTS5 trigram table
/// `ContactKeywordMessage_fts` with external content `ContactKeywordMessage`.
#[tokio::test]
async fn contact_keyword_message_has_its_named_indexes() {
    let db = fresh_db().await;
    index(&db, "ContactKeywordMessage", "ContactKeywordMessage_text_prefix_idx").await;
    assert_eq!(
        index_columns(&db, "ContactKeywordMessage_text_prefix_idx").await,
        [Some("workspaceId".to_owned()), Some("normalizedText".to_owned())]
    );
    let fts = schema_sql(&db, "table", "ContactKeywordMessage_fts")
        .await
        .expect("ContactKeywordMessage_fts is missing");
    assert!(fts.contains("fts5"), "not an FTS5 table: {fts}");
    assert!(fts.contains("tokenize='trigram"), "not a trigram tokenizer: {fts}");
    assert!(fts.contains("content='ContactKeywordMessage'"), "not external content: {fts}");
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:100 "contact activity partition catalog > ContactState has its named indexes on all 64 leaves"
#[tokio::test]
async fn contact_state_has_its_named_indexes() {
    let db = fresh_db().await;
    let idx = index(&db, "ContactState", "ContactState_lastDeliveredAt_idx").await;
    assert!(idx.partial, "ContactState_lastDeliveredAt_idx must be partial");
}

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:131 "contact activity partition catalog > has the allowed-pair CHECK constraint on the parent"
#[tokio::test]
async fn has_the_allowed_pair_check_constraint() {
    let db = fresh_db().await;
    let sql = table_sql(&db, "ContactActivity").await.expect("ContactActivity is missing");
    assert_eq!(sql.matches(r#"CONSTRAINT "ContactActivity_allowed_pair_check" CHECK"#).count(), 1, "{sql}");

    // flowExecuted is not parent-scoped: parentReferenceId must be 0.
    let err = db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "ContactActivity"
                     ("workspaceId", "contactId", "activityType", "referenceType", "parentReferenceId", "referenceId", "firstOccurredAt")
                   VALUES (?1, ?2, 'flowExecuted', 'flow', 7, ?3, ?4)"#,
                params![1_i64, 1_i64, 1_i64, 0_i64],
            )?;
            Ok(())
        })
        .await
        .expect_err("a disallowed pair must be refused");
    assert_eq!(err.sqlstate(), Some("23514"));
}
