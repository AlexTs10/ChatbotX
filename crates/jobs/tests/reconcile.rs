//! Port of `apps/worker/__tests__/reconcile.test.ts` (spec 5.3
//! "reconcile"). `SchedulerClient` = `FakeDispatchSchedule`, the dispatch
//! queries = `FakeReconcileRepo`; the retention delete runs on real SQLite
//! (`SequenceDispatch`, P2.4 schema).

#[path = "support/sequence_worker.rs"]
mod sequence_worker;
mod support;

use std::sync::{Arc, Mutex};

use chatbotx_jobs::sequence::reconcile::{
    PendingDue, ReconcileDeps, ReconcileJob, ReconcileOptions, RetentionOutcome, RunAtOrder,
};
use chatbotx_jobs::sequence::schedule::ScheduleKind;
use sequence_worker::ids::{D1, D2, D3, D4, D5, WS1};
use sequence_worker::{
    FakeDispatchSchedule, FakeReconcileRepo, ScheduleCall, SeedDispatch, dispatch_ids,
    seed_dispatches,
};
use support::{TestClock, migrated_db};

const NOW: i64 = 1_700_000_000_000;
const DAY_MS: i64 = 86_400_000;

/// `new ReconcileJob({ intervalMs: 1000, cleanupIntervalMs: 1000 })` with
/// `SCHEDULER_BUCKET_RANGE = "0-0"`.
fn options() -> ReconcileOptions {
    ReconcileOptions {
        interval_ms: Some(1000),
        cleanup_interval_ms: Some(1000),
        buckets: Some(vec![0]),
        ..ReconcileOptions::default()
    }
}

fn job(
    options: ReconcileOptions,
    schedule: &Arc<FakeDispatchSchedule>,
    repo: &Arc<FakeReconcileRepo>,
    db: Option<Arc<chatbotx_db::Db>>,
) -> ReconcileJob {
    ReconcileJob::new(
        options,
        ReconcileDeps {
            schedule: Arc::clone(schedule) as _,
            repo: Arc::clone(repo) as _,
            db,
            clock: TestClock::new(NOW),
        },
    )
}

/// Port of: apps/worker/__tests__/reconcile.test.ts:91 "ReconcileJob > reconcile scans pending dispatches by runAtMs without workspaceId"
#[tokio::test(start_paused = true)]
async fn reconcile_scans_pending_dispatches_by_runatms_without() {
    let schedule = FakeDispatchSchedule::new();
    let repo = Arc::new(FakeReconcileRepo::default());
    let job = job(options(), &schedule, &repo, None);

    job.reconcile().await.expect("reconcile");

    let queries = repo.queries.lock().unwrap().clone();
    assert_eq!(queries.len(), 1);
    let q = &queries[0];
    assert_eq!(q.run_at_ms_lte, NOW + DAY_MS);
    assert_eq!(q.workspace_id, None);
    assert_eq!(q.limit, 1000);
    assert_eq!(q.order, RunAtOrder::Asc);
}

/// Port of: apps/worker/__tests__/reconcile.test.ts:121 "ReconcileJob > reconcile adds each pending dispatch back to the scheduler"
#[tokio::test(start_paused = true)]
async fn reconcile_adds_each_pending_dispatch_back_to_the_scheduler() {
    let schedule = FakeDispatchSchedule::new();
    let repo = Arc::new(FakeReconcileRepo {
        due: Mutex::new(
            [vec![
                PendingDue {
                    id: "dispatch-1".into(),
                    bucket: 7,
                    run_at_ms: 1234,
                },
                PendingDue {
                    id: "dispatch-2".into(),
                    bucket: 8,
                    run_at_ms: 5678,
                },
            ]]
            .into(),
        ),
        ..FakeReconcileRepo::default()
    });
    let job = job(options(), &schedule, &repo, None);

    // The 1000 ms pause at the end of the pass runs on paused time.
    job.reconcile().await.expect("reconcile");

    let adds: Vec<_> = schedule
        .calls()
        .into_iter()
        .filter(|c| matches!(c, ScheduleCall::AddToSchedule { .. }))
        .collect();
    assert_eq!(
        adds,
        vec![
            ScheduleCall::AddToSchedule {
                bucket: 7,
                id: "dispatch-1".into(),
                at_ms: 1234
            },
            ScheduleCall::AddToSchedule {
                bucket: 8,
                id: "dispatch-2".into(),
                at_ms: 5678
            },
        ]
    );
}

/// Port of: apps/worker/__tests__/reconcile.test.ts:138 "ReconcileJob > cleanupOrphans validates Redis ids against pending dispatches only"
#[tokio::test(start_paused = true)]
async fn cleanuporphans_validates_redis_ids_against_pending_dispatches() {
    let schedule = FakeDispatchSchedule::new();
    schedule.push_members(ScheduleKind::Schedule, &["dispatch-1", "dispatch-orphan"]);
    schedule.push_members(ScheduleKind::Retry, &[]);
    let repo = Arc::new(FakeReconcileRepo {
        pending_ids: Mutex::new(vec!["dispatch-1".into()]),
        ..FakeReconcileRepo::default()
    });
    let job = job(options(), &schedule, &repo, None);

    job.cleanup_orphans().await.expect("cleanup");

    assert_eq!(
        repo.id_lookups.lock().unwrap().clone(),
        vec![vec!["dispatch-1".to_owned(), "dispatch-orphan".to_owned()]]
    );
    assert!(schedule.has(&ScheduleCall::RemoveFromAll {
        bucket: 0,
        id: "dispatch-orphan".into()
    }));
    assert!(!schedule.has(&ScheduleCall::RemoveFromAll {
        bucket: 0,
        id: "dispatch-1".into()
    }));
}

/// Port of: apps/worker/__tests__/reconcile.test.ts:159 "ReconcileJob > deleteTerminalDispatches batches terminal rows older than the retention TTL"
#[tokio::test]
async fn deleteterminaldispatches_batches_terminal_rows_older_than() {
    let db = migrated_db().await;
    let old = NOW - 31 * DAY_MS;
    let recent = NOW - DAY_MS;
    let row = |id, status, at| SeedDispatch {
        created_at: at,
        updated_at: at,
        ..SeedDispatch::new(id, WS1, status)
    };
    seed_dispatches(
        &db,
        vec![
            row(D1, "completed", old),
            row(D2, "failed", old),
            row(D3, "canceled", old),
            // Kept: terminal but inside the TTL, and old but still pending.
            row(D4, "completed", recent),
            row(D5, "pending", old),
        ],
    )
    .await;
    let schedule = FakeDispatchSchedule::new();
    let repo = Arc::new(FakeReconcileRepo::default());
    let job = job(
        ReconcileOptions {
            retention_batch_size: Some(2),
            retention_ttl_days: Some(30),
            ..options()
        },
        &schedule,
        &repo,
        Some(Arc::clone(&db)),
    );

    let outcome = job.delete_terminal_dispatches().await.expect("retention");

    assert_eq!(
        outcome,
        RetentionOutcome {
            deleted: 3,
            batches: 2
        }
    );
    assert_eq!(dispatch_ids(&db).await, vec![D4, D5]);
}
