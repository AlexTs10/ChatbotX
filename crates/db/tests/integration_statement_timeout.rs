//! Port of `packages/database/__tests__/integration/statement-timeout.test.ts`
//! (P2.3 spec section 5.2, rows 18.1-18.2). File name prefixed with `integration_` because
//! `statement_timeout.rs` ports the unit test of the same name.
//! Class C: real writer jobs on a migrated SQLite file.

mod common;

use std::time::{Duration, Instant};

use chatbotx_db::rusqlite::params;
use chatbotx_db::StatementTimeout;
use common::{fresh_db, query_i64};

/// PostgreSQL SQLSTATE for `canceling statement due to statement timeout`.
const QUERY_CANCELED: &str = "57014";

/// A statement that never ends.
const ENDLESS: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";

/// Port of: packages/database/__tests__/integration/statement-timeout.test.ts:57 "setLocalStatementTimeout against Postgres > cancels a statement that outlives the cap and aborts the transaction"
#[tokio::test]
async fn cancels_a_statement_that_outlives_the_cap_and_aborts_the_transaction() {
    let db = fresh_db().await;
    let started = Instant::now();
    let run = db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt") VALUES ('v-timeout', 'probe', 'x', 0)"#,
            [],
        )?;
        tx.set_statement_timeout(StatementTimeout::parse("100ms").expect("valid"))?;
        let n: i64 = tx.query_row(ENDLESS, [], |row| row.get(0))?;
        Ok(n)
    });
    let result = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("the statement timeout did not stop the statement within 30 s");
    let err = result.expect_err("the job must fail");
    assert_eq!(err.sqlstate(), Some(QUERY_CANCELED));
    assert!(started.elapsed() < Duration::from_secs(30));

    // The transaction was rolled back: the row inserted before the statement is gone.
    let rows = query_i64(&db, r#"SELECT count(*) FROM "Verification" WHERE "id" = 'v-timeout'"#, vec![]).await;
    assert_eq!(rows, 0);
}

/// Port of: packages/database/__tests__/integration/statement-timeout.test.ts:70 "setLocalStatementTimeout against Postgres > is scoped to the transaction: the session keeps its own setting"
#[tokio::test]
async fn is_scoped_to_the_transaction_the_session_keeps_its_own_setting() {
    let db = fresh_db().await;
    let before = db.write(|tx| Ok(tx.statement_timeout())).await.expect("write job");

    let inside = db
        .write(|tx| {
            tx.set_statement_timeout(StatementTimeout::parse("100ms").expect("valid"))?;
            Ok(tx.statement_timeout())
        })
        .await
        .expect("write job");
    assert_eq!(inside, Some(Duration::from_millis(100)));

    // Next job on the same writer connection: no cap, and a statement longer than 100 ms runs.
    let (after, installs, elapsed) = db
        .write(|tx| {
            let after = tx.statement_timeout();
            let installs = tx.progress_handler_installs();
            let mut n: i64 = 100_000;
            loop {
                let started = Instant::now();
                let _count: i64 = tx.query_row(
                    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?1) SELECT count(*) FROM c",
                    params![n],
                    |row| row.get(0),
                )?;
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_millis(200) || n >= 1_000_000_000 {
                    return Ok((after, installs, elapsed));
                }
                n *= 4;
            }
        })
        .await
        .expect("a 200 ms statement after the capped transaction must complete");
    assert_eq!(after, before);
    assert_eq!(after, None);
    assert_eq!(installs, 0);
    assert!(elapsed >= Duration::from_millis(200), "the probe statement ran only {elapsed:?}");
}
