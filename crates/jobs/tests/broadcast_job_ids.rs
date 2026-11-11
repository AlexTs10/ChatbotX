//! Port of `packages/worker-config/__tests__/broadcast-job-ids.test.ts` (spec 5.1).

use chatbotx_jobs::queues::broadcast::{broadcast_prepare_job_id, materialize_batch_job_id};

/// Port of: packages/worker-config/__tests__/broadcast-job-ids.test.ts:10 "broadcast job ids > the prepare id keeps the format enqueueBroadcast has always used"
#[test]
fn the_prepare_id_keeps_the_format_enqueuebroadcast_has_always() {
    assert_eq!(
        broadcast_prepare_job_id("123"),
        "schedule-prepare-broadcast-123"
    );
}

/// Port of: packages/worker-config/__tests__/broadcast-job-ids.test.ts:14 "broadcast job ids > a recurrence batch id is deterministic per tick, page and chunk, and BullMQ-safe"
#[test]
fn a_recurrence_batch_id_is_deterministic_per_tick_page_and() {
    let id = materialize_batch_job_id(1_760_320_800_000, 2, 3);
    assert_eq!(id, "broadcast-recurrence-1760320800000-2-3");
    assert!(!id.contains(':'));
    assert_ne!(materialize_batch_job_id(1_760_320_800_000, 2, 4), id);
}
