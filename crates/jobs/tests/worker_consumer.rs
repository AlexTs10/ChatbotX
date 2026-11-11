//! Port of `apps/worker/__tests__/worker-consumer.test.ts` (spec 5.3
//! "worker-consumer"): `revertDispatchToPending` on real SQLite
//! `SequenceDispatch` rows (P2.4 schema).

#[path = "support/sequence_worker.rs"]
mod sequence_worker;
mod support;

use chatbotx_jobs::sequence::retry::revert_dispatch_to_pending;
use sequence_worker::ids::{D1, D2, D3, WS1, WS2, s};
use sequence_worker::{SeedDispatch, read_dispatch, seed_dispatches};
use support::{TestClock, migrated_db};

const SEEDED_AT: i64 = 1_700_000_000_000;
const NOW: i64 = 1_700_000_060_000;

/// Port of: apps/worker/__tests__/worker-consumer.test.ts:40 "revertDispatchToPending > sets status=pending and WHERE includes id + workspaceId + status=running"
#[tokio::test]
async fn sets_status_pending_and_where_includes_id_workspaceid_status() {
    let db = migrated_db().await;
    let locked = |id, ws, status| SeedDispatch {
        locked_at: Some(SEEDED_AT),
        lock_owner: Some("worker-host"),
        ..SeedDispatch::new(id, ws, status)
    };
    seed_dispatches(
        &db,
        vec![
            locked(D1, WS1, "running"),
            locked(D2, WS1, "completed"),
            locked(D3, WS2, "running"),
        ],
    )
    .await;
    let clock = TestClock::new(NOW);

    // status guard (d2) and workspace guard (d3): no change.
    revert_dispatch_to_pending(&db, clock.as_ref(), &s(D2), &s(WS1))
        .await
        .expect("no-op");
    revert_dispatch_to_pending(&db, clock.as_ref(), &s(D3), &s(WS1))
        .await
        .expect("no-op");
    revert_dispatch_to_pending(&db, clock.as_ref(), &s(D1), &s(WS1))
        .await
        .expect("revert");

    let d1 = read_dispatch(&db, D1).await.expect("d1");
    assert_eq!(d1.status, "pending");
    assert_eq!(d1.locked_at, None);
    assert_eq!(d1.lock_owner, None);
    assert_eq!(d1.updated_at, NOW);
    let d2 = read_dispatch(&db, D2).await.expect("d2");
    assert_eq!(
        (d2.status.as_str(), d2.updated_at),
        ("completed", SEEDED_AT)
    );
    let d3 = read_dispatch(&db, D3).await.expect("d3");
    assert_eq!((d3.status.as_str(), d3.updated_at), ("running", SEEDED_AT));
    assert_eq!(d3.lock_owner.as_deref(), Some("worker-host"));
}
