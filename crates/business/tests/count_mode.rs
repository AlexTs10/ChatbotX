//! Port of `packages/business/__tests__/count-mode.test.ts` (P3.6-D2 slice
//! contacts list). Adapted: the mocked `db.transaction` /
//! `setLocalStatementTimeout` / `isStatementTimeoutError` become a real
//! write transaction with the SQLite statement timeout (a progress
//! handler); the spied counters become closures that record their calls.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chatbotx_business::BusinessError;
use chatbotx_business::count_mode::{
    BOUNDED_COUNT_CAP, CountMode, CountOutcome, count_by_mode, count_by_mode_with_timeout,
};
use chatbotx_db::StatementTimeout;
use chatbotx_db::repo;
use chatbotx_db::repo::client::CappedCount;
use common::fresh_db;

type Calls = Arc<Mutex<Vec<String>>>;

fn calls() -> Calls {
    Arc::new(Mutex::new(Vec::new()))
}

fn recorded(calls: &Calls) -> Vec<String> {
    calls.lock().expect("calls").clone()
}

#[tokio::test]
async fn count_by_mode_defaults_to_an_exact_count_under_the_statement_timeout() {
    let db = fresh_db().await;
    let seen = calls();
    let (exact_seen, bounded_seen) = (Arc::clone(&seen), Arc::clone(&seen));
    let result = count_by_mode(
        &db,
        None,
        Box::new(move |tx| {
            exact_seen
                .lock()
                .expect("calls")
                .push(format!("exact {:?}", tx.statement_timeout()));
            Ok(1234)
        }),
        Box::new(move |_, _| {
            bounded_seen.lock().expect("calls").push("bounded".into());
            Ok(CappedCount {
                total: 0,
                capped: false,
                cap: 0,
            })
        }),
    )
    .await
    .expect("count");
    assert_eq!(
        result,
        CountOutcome {
            total: 1234,
            total_count_capped: false,
            count_cap: None
        }
    );
    assert_eq!(
        recorded(&seen),
        vec![format!("exact {:?}", Some(Duration::from_secs(30)))]
    );
}

#[tokio::test]
async fn count_by_mode_bounded_mode_passes_the_server_cap_and_reports_a_lower_bound() {
    let db = fresh_db().await;
    let seen = calls();
    let (exact_seen, bounded_seen) = (Arc::clone(&seen), Arc::clone(&seen));
    let result = count_by_mode(
        &db,
        Some(CountMode::Bounded),
        Box::new(move |_| {
            exact_seen.lock().expect("calls").push("exact".into());
            Ok(0)
        }),
        Box::new(move |cap, tx| {
            bounded_seen
                .lock()
                .expect("calls")
                .push(format!("bounded {cap} {:?}", tx.statement_timeout()));
            Ok(CappedCount {
                total: BOUNDED_COUNT_CAP,
                capped: true,
                cap,
            })
        }),
    )
    .await
    .expect("count");
    assert_eq!(
        result,
        CountOutcome {
            total: BOUNDED_COUNT_CAP,
            total_count_capped: true,
            count_cap: Some(BOUNDED_COUNT_CAP),
        }
    );
    assert_eq!(
        recorded(&seen),
        vec![format!(
            "bounded {BOUNDED_COUNT_CAP} {:?}",
            Some(Duration::from_secs(30))
        )]
    );
}

#[tokio::test]
async fn count_by_mode_bounded_mode_under_the_cap_is_not_flagged_capped() {
    let db = fresh_db().await;
    let result = count_by_mode(
        &db,
        Some(CountMode::Bounded),
        Box::new(|_| Ok(0)),
        Box::new(|cap, _| {
            Ok(CappedCount {
                total: 7,
                capped: false,
                cap,
            })
        }),
    )
    .await
    .expect("count");
    assert_eq!(
        result,
        CountOutcome {
            total: 7,
            total_count_capped: false,
            count_cap: Some(BOUNDED_COUNT_CAP)
        }
    );
}

/// A statement that runs far longer than the 1 ms test timeout.
fn slow_count(tx: &chatbotx_db::WriteTx<'_>) -> chatbotx_business::Result<i64> {
    let total: i64 = tx
        .query_row(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000000) SELECT count(*) FROM n",
            [],
            |r| r.get(0),
        )
        .map_err(|e| repo::Error::Db(e.into()))?;
    Ok(total)
}

#[tokio::test]
async fn count_by_mode_s_mode_turns_a_statement_timeout_into_the_typed_503() {
    let db = fresh_db().await;
    for mode in [CountMode::Exact, CountMode::Bounded] {
        let error = count_by_mode_with_timeout(
            &db,
            Some(mode),
            StatementTimeout::parse("1ms").expect("timeout"),
            Box::new(slow_count),
            Box::new(|cap, tx| {
                let total = slow_count(tx)?;
                Ok(CappedCount {
                    total,
                    capped: false,
                    cap,
                })
            }),
        )
        .await
        .expect_err("timeout");
        assert_eq!(error.code(), Some("countTimeout"), "{mode:?}");
        assert_eq!(error.status(), Some(503), "{mode:?}");
    }
}

#[tokio::test]
async fn count_by_mode_rethrows_a_non_timeout_failure_untouched() {
    let db = fresh_db().await;
    let error = count_by_mode(
        &db,
        Some(CountMode::Exact),
        Box::new(|_| Err(BusinessError::Repo(repo::Error::msg("connection reset")))),
        Box::new(|cap, _| {
            Ok(CappedCount {
                total: 0,
                capped: false,
                cap,
            })
        }),
    )
    .await
    .expect_err("failure");
    match error {
        BusinessError::Repo(repo::Error::Message(message)) => {
            assert_eq!(message, "connection reset");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
