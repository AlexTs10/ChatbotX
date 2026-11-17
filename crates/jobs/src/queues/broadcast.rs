//! Broadcast job ids on the `schedule` queue (`queues/schedule`).

/// `schedule-prepare-broadcast-<broadcastId>`.
pub fn broadcast_prepare_job_id(broadcast_id: &str) -> String {
    format!("schedule-prepare-broadcast-{broadcast_id}")
}

/// `broadcast-recurrence-<dueAtMs>-<page>-<chunk>`.
pub fn materialize_batch_job_id(due_at_ms: i64, page: u32, chunk: u32) -> String {
    format!("broadcast-recurrence-{due_at_ms}-{page}-{chunk}")
}

/// `broadcast-send-<broadcastId>`.
pub fn broadcast_send_job_id(broadcast_id: &str) -> String {
    format!("broadcast-send-{broadcast_id}")
}

/// `PURGE_BROADCAST_CONCURRENCY`.
pub const PURGE_BROADCAST_CONCURRENCY: usize = 5;
/// `MATERIALIZE_BATCH_SIZE`: series per `materializeRecurringBroadcasts` job.
pub const MATERIALIZE_BATCH_SIZE: usize = 100;
/// `MATERIALIZE_ITEM_CONCURRENCY`.
pub const MATERIALIZE_ITEM_CONCURRENCY: usize = 10;
/// `MATERIALIZE_RECURRING_BROADCASTS_PRIORITY`.
pub const MATERIALIZE_RECURRING_BROADCASTS_PRIORITY: u32 = 10;
