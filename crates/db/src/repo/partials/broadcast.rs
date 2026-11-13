//! Port of `partials/broadcast.ts`.

/// `broadcastStatuses`.
pub const BROADCAST_STATUSES: &[&str] = &[
    "scheduled",
    "sent",
    "sending",
    "cancelled",
    "draft",
    "failed",
    "recurring",
];

/// `BROADCAST_TRIAL_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_TRIAL_SEND_RATE_PER_MINUTE: i64 = 60;
/// `BROADCAST_TRIAL_MAX_ACTIVE`.
pub const BROADCAST_TRIAL_MAX_ACTIVE: i64 = 1;
/// `BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE: i64 = 100;
/// `BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER`.
pub const BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER: i64 = 20;
