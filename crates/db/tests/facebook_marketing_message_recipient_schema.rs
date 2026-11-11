//! Port of `packages/database/__tests__/facebook-marketing-message-recipient-schema.test.ts`
//! (P2.5 spec section 3.1, rows 6.1-6.3). Class C: the original reads the Drizzle table
//! config; the port reads the migrated SQLite catalog.

mod common;

use common::{columns, fresh_db, index, table_names};

const TABLE: &str = "FacebookMarketingMessageRecipient";

/// Port of: packages/database/__tests__/facebook-marketing-message-recipient-schema.test.ts:9 "FacebookMarketingMessageRecipient > recipient table is unique per campaign+token"
#[tokio::test]
async fn recipient_table_is_unique_per_campaign_token() {
    let db = fresh_db().await;
    let tables = table_names(&db).await;
    assert!(tables.iter().any(|t| t == TABLE), "{TABLE} is missing");
    let unique = index(
        &db,
        TABLE,
        "FacebookMarketingMessageRecipient_campaign_token_key",
    )
    .await;
    assert!(unique.unique);
}

/// Port of: packages/database/__tests__/facebook-marketing-message-recipient-schema.test.ts:21 "FacebookMarketingMessageRecipient > has a partial due index"
#[tokio::test]
async fn has_a_partial_due_index() {
    let db = fresh_db().await;
    let due = index(&db, TABLE, "FacebookMarketingMessageRecipient_due_idx").await;
    assert!(due.partial, "the due index must have a WHERE clause");
}

/// Port of: packages/database/__tests__/facebook-marketing-message-recipient-schema.test.ts:29 "FacebookMarketingMessageRecipient > campaign has delivery columns"
#[tokio::test]
async fn campaign_has_delivery_columns() {
    let db = fresh_db().await;
    let names: Vec<String> = columns(&db, "FacebookMarketingMessage")
        .await
        .into_iter()
        .map(|c| c.name)
        .collect();
    for expected in [
        "status",
        "startsAt",
        "firstSentAt",
        "statusReason",
        "tokenCount",
        "dayTarget",
        "daySent",
        "metaEndsAt",
        "timezoneName",
        "resumeCount",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "FacebookMarketingMessage.{expected} is missing"
        );
    }
}
