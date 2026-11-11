//! Port of `packages/database/__tests__/integration/contact-activity-catalog.test.ts`, the
//! part-3 case (P2.5 spec section 3.1, row 2.1). The other cases are in
//! `contact_activity_catalog.rs` (P2.3). Class C: runs against a migrated SQLite file.

mod common;

use common::{column, fks, fresh_db};

/// Port of: packages/database/__tests__/integration/contact-activity-catalog.test.ts:141 "contact activity partition catalog > AnalyticsEmailTopic keeps tracking rows when a topic is deleted"
///
/// Adapted (lookup by column): SQLite FKs have no name, so the FK
/// `AnalyticsEmailTopic_topicId_EmailTopic_id_fkey` is found by its column `topicId`.
/// `confdeltype = 'n'` is `on_delete = SET NULL`; `attnotnull = false` is `notnull == false`.
#[tokio::test]
async fn analytics_email_topic_keeps_tracking_rows_when_a_topic_is_deleted() {
    let db = fresh_db().await;
    let topic_fks: Vec<_> = fks(&db, "AnalyticsEmailTopic")
        .await
        .into_iter()
        .filter(|fk| fk.from == "topicId")
        .collect();
    assert_eq!(topic_fks.len(), 1, "one FK from topicId: {topic_fks:?}");
    let fk = &topic_fks[0];
    assert_eq!(fk.parent, "EmailTopic");
    assert_eq!(fk.to.as_deref(), Some("id"));
    assert_eq!(fk.on_delete, "SET NULL");

    assert!(
        !column(&db, "AnalyticsEmailTopic", "topicId").await.notnull,
        "AnalyticsEmailTopic.topicId must be nullable"
    );
}
