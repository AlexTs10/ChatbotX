//! `workspace/deletion-schedule.ts`.

/// `WORKSPACE_DELETION_GRACE_MS`: 24 hours.
pub const WORKSPACE_DELETION_GRACE_MS: i64 = 24 * 60 * 60 * 1000;

/// `PURGE_WORKSPACES_INTERVAL_MINUTES` (`worker-config`): 30.
pub const PURGE_WORKSPACES_INTERVAL_MINUTES: i64 = 30;

const PURGE_INTERVAL_MS: i64 = PURGE_WORKSPACES_INTERVAL_MINUTES * 60 * 1000;

/// `ceilToPurgeBoundary`: rounds up to the next purge cron boundary (an
/// epoch multiple of the interval).
pub fn ceil_to_purge_boundary(ms: i64) -> i64 {
    ms.div_euclid(PURGE_INTERVAL_MS) * PURGE_INTERVAL_MS
        + if ms.rem_euclid(PURGE_INTERVAL_MS) == 0 {
            0
        } else {
            PURGE_INTERVAL_MS
        }
}

/// `nextScheduledDeletionAt(now)`: now + 24h, rounded up to the boundary.
pub fn next_scheduled_deletion_at(now_ms: i64) -> i64 {
    ceil_to_purge_boundary(now_ms + WORKSPACE_DELETION_GRACE_MS)
}
