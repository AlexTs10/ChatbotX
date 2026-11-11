//! Port of `packages/database/__tests__/integration/schema-default-parity.test.ts`
//! (P2.3 spec section 5.1, row 1.1). Class C: runs against a migrated SQLite file.

mod common;

use chatbotx_db::schema::APP_SIDE_DEFAULTS;
use common::{column, fresh_db, table_names};

/// The frozen list `DEFAULTS_MISSING_FROM_DATABASE` (sorted `Table.column`).
const DEFAULTS_MISSING_FROM_DATABASE: &[&str] = &[
    "AIAgent.messages",
    "AIAgent.models",
    "AIAgent.tools",
    "AIMCPServer.selectedTools",
    "AutomatedResponse.keywords",
    "Folder.paths",
    "IntegrationInstagram.conversationStarters",
    "IntegrationInstagram.persistentMenus",
    "IntegrationMessenger.conversationStarters",
    "IntegrationMessenger.persistentMenus",
    "IntegrationMessenger.personas",
    "IntegrationWebchat.authorizedDomains",
    "IntegrationWebchat.conversationStarters",
    "IntegrationWebchat.persistentMenus",
    "Trigger.actions",
    "UserPersistentMenu.menus",
];

/// The part-1 tables (P2.3) of the list above: these must exist after `0001_core.sql`.
const PART_1_TABLES: &[&str] = &[
    "Folder",
    "IntegrationInstagram",
    "IntegrationMessenger",
    "IntegrationWebchat",
    "UserPersistentMenu",
];

/// Port of: packages/database/__tests__/integration/schema-default-parity.test.ts:91 "drizzle defaults vs database defaults > only the known columns declare a default the database does not have"
///
/// Adapted (spec 5.1 row 1.1): Rust has no Drizzle `hasDefault`, so the "only the known"
/// half is the Rust const `APP_SIDE_DEFAULTS` (columns the Rust insert code fills), which
/// must equal the frozen list. The database half: every listed column of a table that
/// exists is NOT NULL with no default (P2.3: the 10 part-1 columns; after P2.5: all 16).
#[tokio::test]
async fn only_the_known_columns_declare_a_default_the_database_does_not_have() {
    let mut app_side: Vec<&str> = APP_SIDE_DEFAULTS.to_vec();
    app_side.sort_unstable();
    assert_eq!(app_side, DEFAULTS_MISSING_FROM_DATABASE);

    let db = fresh_db().await;
    let tables = table_names(&db).await;
    for table in PART_1_TABLES {
        assert!(
            tables.iter().any(|t| t == table),
            "part-1 table {table} is missing"
        );
    }

    let mut checked = 0;
    for entry in DEFAULTS_MISSING_FROM_DATABASE {
        let (table, name) = entry.split_once('.').expect("Table.column");
        if !tables.iter().any(|t| t == table) {
            continue; // table of a later part (P2.4/P2.5)
        }
        let col = column(&db, table, name).await;
        assert!(col.notnull, "{entry} must be NOT NULL");
        assert_eq!(
            col.dflt_value, None,
            "{entry} must have no database default"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 16,
        "expected all 16 listed columns, checked {checked}"
    );
}
