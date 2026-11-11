//! Port of `apps/worker/__tests__/sequence-retry-scheduler.test.ts`
//! (spec 5.3 "sequence-retry-scheduler"): real SQLite `SequenceDispatch`
//! rows (P2.4 schema) instead of the Drizzle chain spies; `TestClock` for
//! `updatedAt`.

#[path = "support/sequence_worker.rs"]
mod sequence_worker;
mod support;

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_jobs::Clock;
use chatbotx_jobs::sequence::retry::RetrySchedulerService;
use sequence_worker::ids::{D1, D2, D3, WS1, WS2, s};
use sequence_worker::{SeedDispatch, read_dispatch, seed_dispatches};
use support::{RecordingLog, TestClock, migrated_db};
use tracing::Level;

const SEEDED_AT: i64 = 1_700_000_000_000;
const NOW: i64 = 1_700_000_060_000;

/// d1 = the running target in ws1; d2 = pending in ws1; d3 = running in ws2
/// (integer ids per the P2.4 DDL, see `sequence_worker::ids`).
async fn setup() -> (Arc<Db>, Arc<TestClock>, RetrySchedulerService) {
    let db = migrated_db().await;
    seed_dispatches(
        &db,
        vec![
            SeedDispatch::new(D1, WS1, "running"),
            SeedDispatch::new(D2, WS1, "pending"),
            SeedDispatch::new(D3, WS2, "running"),
        ],
    )
    .await;
    let clock = TestClock::new(NOW);
    let service = RetrySchedulerService::new(Arc::clone(&db), clock.clone());
    (db, clock, service)
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:54 "RetrySchedulerService > markDispatchFailed > updates dispatch to failed status with the provided error message"
#[tokio::test]
async fn updates_dispatch_to_failed_status_with_the_provided_error() {
    let (db, _clock, service) = setup().await;

    service
        .mark_dispatch_failed(&s(D1), &s(WS1), "something went wrong")
        .await
        .expect("mark failed");

    let row = read_dispatch(&db, D1).await.expect("d1");
    assert_eq!(row.status, "failed");
    assert_eq!(row.last_error.as_deref(), Some("something went wrong"));
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:69 "RetrySchedulerService > markDispatchFailed > stamps updatedAt with a current timestamp"
#[tokio::test]
async fn markdispatchfailed_stamps_updatedat_with_a_current_timestamp() {
    let (db, clock, service) = setup().await;
    let before = clock.now_ms();

    service
        .mark_dispatch_failed(&s(D1), &s(WS1), "err")
        .await
        .expect("mark failed");

    let after = clock.now_ms();
    let row = read_dispatch(&db, D1).await.expect("d1");
    assert!(
        row.updated_at >= before && row.updated_at <= after,
        "{row:?}"
    );
    assert_eq!(row.updated_at, NOW);
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:85 "RetrySchedulerService > markDispatchFailed > WHERE clause includes dispatchId + workspaceId + status=running"
#[tokio::test]
async fn markdispatchfailed_where_clause_includes_dispatchid() {
    let (db, _clock, service) = setup().await;

    // status guard: d2 is pending; workspace guard: d3 is in ws2.
    service
        .mark_dispatch_failed(&s(D2), &s(WS1), "err")
        .await
        .expect("no-op");
    service
        .mark_dispatch_failed(&s(D3), &s(WS1), "err")
        .await
        .expect("no-op");
    // id guard: only d1 changes.
    service
        .mark_dispatch_failed(&s(D1), &s(WS1), "err")
        .await
        .expect("mark");

    assert_eq!(read_dispatch(&db, D1).await.expect("d1").status, "failed");
    let d2 = read_dispatch(&db, D2).await.expect("d2");
    assert_eq!((d2.status.as_str(), d2.updated_at), ("pending", SEEDED_AT));
    let d3 = read_dispatch(&db, D3).await.expect("d3");
    assert_eq!((d3.status.as_str(), d3.updated_at), ("running", SEEDED_AT));
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:94 "RetrySchedulerService > markDispatchFailed > calls logger.error with dispatchId and errorMessage"
#[tokio::test]
async fn calls_logger_error_with_dispatchid_and_errormessage() {
    let (_db, _clock, service) = setup().await;
    let log = RecordingLog::install();

    service
        .mark_dispatch_failed(&s(D1), &s(WS1), "boom")
        .await
        .expect("mark failed");

    let errors = log.at(Level::ERROR);
    assert!(
        errors
            .iter()
            .any(|e| e.field("dispatch_id") == Some(s(D1).as_str())
                && e.field("error_message") == Some("boom")
                && !e.message.is_empty()),
        "{errors:?}"
    );
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:107 "RetrySchedulerService > markDispatchCanceled > updates dispatch to canceled status"
#[tokio::test]
async fn updates_dispatch_to_canceled_status() {
    let (db, _clock, service) = setup().await;

    service
        .mark_dispatch_canceled(&s(D1), &s(WS1), "step inactive")
        .await
        .expect("mark canceled");

    assert_eq!(read_dispatch(&db, D1).await.expect("d1").status, "canceled");
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:120 "RetrySchedulerService > markDispatchCanceled > stamps updatedAt with a current timestamp"
#[tokio::test]
async fn markdispatchcanceled_stamps_updatedat_with_a_current() {
    let (db, clock, service) = setup().await;
    let before = clock.now_ms();

    service
        .mark_dispatch_canceled(&s(D1), &s(WS1), "reason")
        .await
        .expect("mark canceled");

    let after = clock.now_ms();
    let row = read_dispatch(&db, D1).await.expect("d1");
    assert!(
        row.updated_at >= before && row.updated_at <= after,
        "{row:?}"
    );
    assert_eq!(row.updated_at, NOW);
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:140 "RetrySchedulerService > markDispatchCanceled > WHERE clause includes dispatchId + workspaceId + status=running"
#[tokio::test]
async fn markdispatchcanceled_where_clause_includes_dispatchid() {
    let (db, _clock, service) = setup().await;

    service
        .mark_dispatch_canceled(&s(D2), &s(WS1), "reason")
        .await
        .expect("no-op");
    service
        .mark_dispatch_canceled(&s(D3), &s(WS1), "reason")
        .await
        .expect("no-op");
    service
        .mark_dispatch_canceled(&s(D1), &s(WS1), "reason")
        .await
        .expect("mark");

    assert_eq!(read_dispatch(&db, D1).await.expect("d1").status, "canceled");
    let d2 = read_dispatch(&db, D2).await.expect("d2");
    assert_eq!((d2.status.as_str(), d2.updated_at), ("pending", SEEDED_AT));
    let d3 = read_dispatch(&db, D3).await.expect("d3");
    assert_eq!((d3.status.as_str(), d3.updated_at), ("running", SEEDED_AT));
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:153 "RetrySchedulerService > markDispatchCanceled > stores the reason string in lastError"
#[tokio::test]
async fn stores_the_reason_string_in_lasterror() {
    let (db, _clock, service) = setup().await;

    service
        .mark_dispatch_canceled(&s(D1), &s(WS1), "step inactive")
        .await
        .expect("mark canceled");

    let row = read_dispatch(&db, D1).await.expect("d1");
    assert_eq!(row.last_error.as_deref(), Some("step inactive"));
}

/// Port of: apps/worker/__tests__/sequence-retry-scheduler.test.ts:164 "RetrySchedulerService > markDispatchCanceled > does NOT call logger.error (only markDispatchFailed logs)"
#[tokio::test]
async fn does_not_call_logger_error_only_markdispatchfailed_logs() {
    let (_db, _clock, service) = setup().await;
    let log = RecordingLog::install();

    service
        .mark_dispatch_canceled(&s(D1), &s(WS1), "reason")
        .await
        .expect("mark canceled");

    assert!(log.at(Level::ERROR).is_empty(), "{:?}", log.entries());
}
