//! Port of `packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts`
//! (P2.5 spec section 3.1, rows 5.1-5.5). Class C: the original reads the Drizzle table
//! config; the port reads the migrated SQLite catalog.

mod common;

use common::{columns, fks, fresh_db, index, indexes, table_names};

const TABLE: &str = "FacebookMarketingMessageAudience";

/// Port of: packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts:8 "FacebookMarketingMessageAudience > is named FacebookMarketingMessageAudience"
#[tokio::test]
async fn is_named_facebook_marketing_message_audience() {
    let db = fresh_db().await;
    let tables = table_names(&db).await;
    assert!(tables.iter().any(|t| t == TABLE), "{TABLE} is missing");
}

/// Port of: packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts:12 "FacebookMarketingMessageAudience > stores the Meta audience id uniquely so a retry cannot duplicate a row"
#[tokio::test]
async fn stores_the_meta_audience_id_uniquely_so_a_retry_cannot_duplicate_a_row() {
    let db = fresh_db().await;
    let unique = index(
        &db,
        TABLE,
        "FacebookMarketingMessageAudience_customAudienceId_key",
    )
    .await;
    assert!(unique.unique);
}

/// Port of: packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts:21 "FacebookMarketingMessageAudience > indexes workspaceId"
#[tokio::test]
async fn indexes_workspace_id() {
    let db = fresh_db().await;
    assert!(
        indexes(&db, TABLE)
            .await
            .iter()
            .any(|i| i.name == "FacebookMarketingMessageAudience_workspaceId_idx")
    );
}

/// Port of: packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts:31 "FacebookMarketingMessageAudience > has the columns the service relies on"
#[tokio::test]
async fn has_the_columns_the_service_relies_on() {
    let db = fresh_db().await;
    let cols = columns(&db, TABLE).await;
    let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "workspaceId",
        "customAudienceId",
        "adAccountId",
        "pageId",
        "name",
        "description",
        "rowsUploaded",
        "readyAt",
        "facebookUserId",
    ] {
        assert!(names.contains(&expected), "{TABLE}.{expected} is missing");
    }
    let not_null = |name: &str| {
        cols.iter()
            .find(|c| c.name == name)
            .map(|c| c.notnull)
            .unwrap_or_else(|| panic!("{TABLE}.{name} is missing"))
    };
    assert!(not_null("customAudienceId"));
    assert!(not_null("pageId"));
    assert!(!not_null("description"));
    assert!(!not_null("readyAt"));
    assert!(not_null("rowsUploaded"));
}

/// Port of: packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts:55 "FacebookMarketingMessageAudience > cascades on workspace delete"
///
/// The table has one FK (`workspaceId`); `config.foreignKeys[0]` is that FK.
#[tokio::test]
async fn cascades_on_workspace_delete() {
    let db = fresh_db().await;
    let table_fks = fks(&db, TABLE).await;
    let first = table_fks.first().expect("the first FK must exist");
    assert_eq!(first.parent, "Workspace");
    assert_eq!(first.on_delete, "CASCADE");
}
