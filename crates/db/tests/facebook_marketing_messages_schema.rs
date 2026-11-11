//! Port of `packages/database/__tests__/facebook-marketing-messages-schema.test.ts`
//! (P2.5 spec section 3.1, rows 7.1-7.7). Class C: the original reads the Drizzle table
//! config; the port reads the migrated SQLite catalog.

mod common;

use common::{columns, fks, fresh_db, index, table_names};

const AUTH: &str = "FacebookMarketingMessagesAuth";
const MESSAGE: &str = "FacebookMarketingMessage";

async fn assert_has_columns(table: &str, expected: &[&str]) {
    let db = fresh_db().await;
    let names: Vec<String> = columns(&db, table)
        .await
        .into_iter()
        .map(|c| c.name)
        .collect();
    for name in expected {
        assert!(names.iter().any(|n| n == name), "{table}.{name} is missing");
    }
}

async fn assert_table_exists(table: &str) {
    let db = fresh_db().await;
    let tables = table_names(&db).await;
    assert!(tables.iter().any(|t| t == table), "{table} is missing");
}

async fn assert_unique_index(table: &str, name: &str) {
    let db = fresh_db().await;
    assert!(
        index(&db, table, name).await.unique,
        "{name} must be unique"
    );
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:11 "FacebookMarketingMessagesAuth > is named FacebookMarketingMessagesAuth"
#[tokio::test]
async fn is_named_facebook_marketing_messages_auth() {
    assert_table_exists(AUTH).await;
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:15 "FacebookMarketingMessagesAuth > holds one grant per workspace"
#[tokio::test]
async fn holds_one_grant_per_workspace() {
    assert_unique_index(AUTH, "FacebookMarketingMessagesAuth_workspaceId_key").await;
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:24 "FacebookMarketingMessagesAuth > carries the App-Scoped User ID and an expiry"
#[tokio::test]
async fn carries_the_app_scoped_user_id_and_an_expiry() {
    assert_has_columns(
        AUTH,
        &["facebookUserId", "tokenExpiresAt", "auth", "status"],
    )
    .await;
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:41 "FacebookMarketingMessage > is named FacebookMarketingMessage"
#[tokio::test]
async fn is_named_facebook_marketing_message() {
    assert_table_exists(MESSAGE).await;
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:45 "FacebookMarketingMessage > stores the Meta campaign id uniquely so a retry cannot duplicate a row"
#[tokio::test]
async fn stores_the_meta_campaign_id_uniquely_so_a_retry_cannot_duplicate_a_row() {
    assert_unique_index(MESSAGE, "FacebookMarketingMessage_campaignId_key").await;
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:54 "FacebookMarketingMessage > keeps the page as a raw id with no foreign key"
///
/// The original: `pageId` exists and no FK points to `IntegrationMessenger`. The port also
/// checks (spec 3.1 row 7.6) that no FK starts from `pageId` and that the FK parents are
/// exactly `FacebookMarketingMessageAudience` and `Workspace`.
#[tokio::test]
async fn keeps_the_page_as_a_raw_id_with_no_foreign_key() {
    let db = fresh_db().await;
    assert!(
        columns(&db, MESSAGE)
            .await
            .iter()
            .any(|c| c.name == "pageId"),
        "{MESSAGE}.pageId is missing"
    );
    let table_fks = fks(&db, MESSAGE).await;
    assert!(
        !table_fks
            .iter()
            .any(|fk| fk.parent == "IntegrationMessenger"),
        "no FK to IntegrationMessenger: {table_fks:?}"
    );
    assert!(
        !table_fks.iter().any(|fk| fk.from == "pageId"),
        "no FK from pageId: {table_fks:?}"
    );
    let mut parents: Vec<&str> = table_fks.iter().map(|fk| fk.parent.as_str()).collect();
    parents.sort_unstable();
    parents.dedup();
    assert_eq!(parents, ["FacebookMarketingMessageAudience", "Workspace"]);
}

/// Port of: packages/database/__tests__/facebook-marketing-messages-schema.test.ts:63 "FacebookMarketingMessage > records the currency offset alongside the minor-unit budget"
#[tokio::test]
async fn records_the_currency_offset_alongside_the_minor_unit_budget() {
    assert_has_columns(
        MESSAGE,
        &[
            "currency",
            "currencyOffset",
            "budgetType",
            "budgetMinorUnits",
            "content",
            "campaignId",
            "facebookUserId",
        ],
    )
    .await;
}
