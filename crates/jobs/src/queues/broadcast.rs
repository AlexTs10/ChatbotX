//! Broadcast job ids on the `schedule` queue (`queues/schedule`).

/// `schedule-prepare-broadcast-<broadcastId>`.
pub fn broadcast_prepare_job_id(broadcast_id: &str) -> String {
    format!("schedule-prepare-broadcast-{broadcast_id}")
}

/// `broadcast-recurrence-<dueAtMs>-<page>-<chunk>`.
pub fn materialize_batch_job_id(due_at_ms: i64, page: u32, chunk: u32) -> String {
    format!("broadcast-recurrence-{due_at_ms}-{page}-{chunk}")
}
