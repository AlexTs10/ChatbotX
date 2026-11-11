//! Port of `packages/database/__tests__/statement-timeout.test.ts`
//! (P2.3 spec section 5.2, rows 17.1-17.2). The type-level case (`:47`, `expectTypeOf`) is
//! not applicable: `StatementTimeout::parse` is the only way to build the newtype.
//!
//! Adapted: SQLite has no `SET LOCAL statement_timeout`; `WriteTx::set_statement_timeout`
//! installs one progress handler with a deadline for the rest of the transaction.

mod common;

use std::time::{Duration, Instant};

use chatbotx_db::StatementTimeout;
use common::fresh_db;

/// `setLocalStatementTimeout(tx, literal)` -> exactly one handler, deadline = now + duration.
async fn assert_sets_statement_timeout(literal: &'static str, expected: Duration) {
    let timeout = StatementTimeout::parse(literal).expect("a valid literal");
    assert_eq!(timeout.duration(), expected);

    let db = fresh_db().await;
    let (before, after, deadline, active, installs) = db
        .write(move |tx| {
            let before = Instant::now();
            tx.set_statement_timeout(timeout)?;
            let after = Instant::now();
            Ok((before, after, tx.statement_deadline(), tx.statement_timeout(), tx.progress_handler_installs()))
        })
        .await
        .expect("write job");
    assert_eq!(installs, 1, "exactly one progress handler");
    assert_eq!(active, Some(expected));
    let deadline = deadline.expect("a deadline");
    assert!(deadline >= before + expected && deadline <= after + expected, "deadline is not now + {literal}");
}

/// Port of: packages/database/__tests__/statement-timeout.test.ts:12 "setLocalStatementTimeout > issues SET LOCAL statement_timeout with the 10s literal inline"
#[tokio::test]
async fn sets_the_10s_statement_timeout() {
    assert_sets_statement_timeout("10s", Duration::from_secs(10)).await;
}

/// Port of: packages/database/__tests__/statement-timeout.test.ts:12 "setLocalStatementTimeout > issues SET LOCAL statement_timeout with the 250ms literal inline"
#[tokio::test]
async fn sets_the_250ms_statement_timeout() {
    assert_sets_statement_timeout("250ms", Duration::from_millis(250)).await;
}

/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"10s; DROP TABLE x\" at runtime before any SQL is built"
/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"10\" at runtime before any SQL is built"
/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"1.5s\" at runtime before any SQL is built"
/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"-1s\" at runtime before any SQL is built"
/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"10 s\" at runtime before any SQL is built"
/// Port of: packages/database/__tests__/statement-timeout.test.ts:28 "setLocalStatementTimeout > refuses \"\" at runtime before any SQL is built"
#[tokio::test]
async fn refuses_invalid_statement_timeout_literals() {
    const INVALID: [&str; 6] = ["10s; DROP TABLE x", "10", "1.5s", "-1s", "10 s", ""];
    for invalid in INVALID {
        assert!(StatementTimeout::parse(invalid).is_err(), "{invalid:?} must be refused");
    }

    // Nothing is installed: the refused literal never reaches the transaction.
    let db = fresh_db().await;
    let (active, installs) = db
        .write(|tx| {
            for invalid in INVALID {
                if let Ok(timeout) = StatementTimeout::parse(invalid) {
                    tx.set_statement_timeout(timeout)?;
                }
            }
            Ok((tx.statement_timeout(), tx.progress_handler_installs()))
        })
        .await
        .expect("write job");
    assert_eq!(active, None);
    assert_eq!(installs, 0);
}
