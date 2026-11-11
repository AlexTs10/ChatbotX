//! Port of `packages/database/__tests__/client-errors.test.ts`
//! (P2.3 spec section 5.2, rows 19.1-19.2).

mod common;

use std::fmt;

use chatbotx_db::rusqlite::params;
use chatbotx_db::{describe_database_error, DatabaseErrorDescription, DbError, StatementTimeout};
use common::fresh_db;

/// The wrapper the query layer puts around a driver error (drizzle's "Failed query").
#[derive(Debug)]
struct FailedQuery(DbError);

impl fmt::Display for FailedQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Failed query")
    }
}

impl std::error::Error for FailedQuery {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// A non-database error.
#[derive(Debug)]
struct Network;

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("network failed")
    }
}

impl std::error::Error for Network {}

/// Port of: packages/database/__tests__/client-errors.test.ts:5 "describeDatabaseError > extracts postgres fields from a nested drizzle cause"
///
/// Adapted: a real statement-timeout error (SQLSTATE 57014) from a writer job, wrapped in an
/// outer error, is described with the fields of the nested `DbError`. SQLite reports no
/// table for an interrupt, so the `table` field is checked on a real UNIQUE error instead.
#[tokio::test]
async fn extracts_database_fields_from_a_nested_cause() {
    let db = fresh_db().await;

    let timeout = db
        .write(|tx| {
            tx.set_statement_timeout(StatementTimeout::parse("100ms").expect("valid"))?;
            let n: i64 = tx.query_row(
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
                [],
                |row| row.get(0),
            )?;
            Ok(n)
        })
        .await
        .expect_err("statement timeout");
    let (message, detail, table) =
        (timeout.message().to_owned(), timeout.detail().map(str::to_owned), timeout.table().map(str::to_owned));
    assert!(!message.is_empty());
    let described = describe_database_error(&FailedQuery(timeout));
    assert_eq!(
        described,
        DatabaseErrorDescription { code: Some("57014".to_owned()), constraint: None, detail, message, table }
    );
    assert_ne!(described.message, "Failed query");

    let unique = db
        .write(|tx| {
            for id in [1_i64, 2] {
                tx.execute(
                    r#"INSERT INTO "User" ("id", "email", "tenantId") VALUES (?1, 'same@example.com', 1)"#,
                    params![id],
                )?;
            }
            Ok(())
        })
        .await
        .expect_err("duplicate (email, tenantId)");
    let described = describe_database_error(&FailedQuery(unique));
    assert_eq!(described.code.as_deref(), Some("23505"));
    assert_eq!(described.constraint.as_deref(), Some("User_email_tenant_key"));
    assert_eq!(described.table.as_deref(), Some("User"));
    assert!(described.detail.is_some());
    assert_ne!(described.message, "Failed query");
}

/// Port of: packages/database/__tests__/client-errors.test.ts:22 "describeDatabaseError > falls back to the top-level error message when no database code exists"
#[test]
fn falls_back_to_the_top_level_error_message_when_no_database_code_exists() {
    assert_eq!(
        describe_database_error(&Network),
        DatabaseErrorDescription { message: "network failed".to_owned(), ..Default::default() }
    );
}
