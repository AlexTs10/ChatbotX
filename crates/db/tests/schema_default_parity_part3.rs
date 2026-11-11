//! Port of `packages/database/__tests__/integration/schema-default-parity.test.ts`, the part-3
//! case (P2.5 spec section 3.1, row 1.1). The `:91` case is in `schema_default_parity.rs`
//! (P2.3; its limit is the full 16-entry list since P2.5). Class C: runs against a migrated
//! SQLite file.

mod common;

use chatbotx_db::rusqlite::params;
use common::{column, fresh_db, query_text, seed_workspace};

/// Port of: packages/database/__tests__/integration/schema-default-parity.test.ts:118 "drizzle defaults vs database defaults > AIAgent.actionRules has a database default"
///
/// The original reads `information_schema.columns.column_default IS NOT NULL`; the port reads
/// `pragma_table_xinfo.dflt_value` and also checks that an insert without the column stores
/// the default.
#[tokio::test]
async fn ai_agent_action_rules_has_a_database_default() {
    let db = fresh_db().await;
    let col = column(&db, "AIAgent", "actionRules").await;
    assert!(col.notnull, "AIAgent.actionRules must be NOT NULL");
    assert_eq!(col.dflt_value.as_deref(), Some("'[]'"));

    let (_, workspace_id) = seed_workspace(&db, 1001, 2001).await;
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "AIAgent" ("id", "workspaceId", "name", "messages", "tools", "models", "temperature", "maxOutputTokens")
               VALUES (7001, ?1, 'Agent', '[]', '[]', '[]', 0.5, 1024)"#,
            params![workspace_id],
        )?;
        Ok(())
    })
    .await
    .expect("insert AIAgent without actionRules");
    assert_eq!(
        query_text(
            &db,
            r#"SELECT "actionRules" FROM "AIAgent" WHERE "id" = 7001"#,
            vec![]
        )
        .await
        .as_deref(),
        Some("[]")
    );
}
