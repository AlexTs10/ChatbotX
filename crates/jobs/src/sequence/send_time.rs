//! `send-time-validator.ts`: move a run time into the allowed send days and
//! the daily send window. Local times are in an explicit zone (spec 5.2:
//! the TypeScript code used the process zone; production passes UTC).

use chrono_tz::Tz;

/// `MAX_ATTEMPTS`: days tried before giving up and returning the base time.
pub const MAX_ATTEMPTS: u32 = 14;

/// The send settings of a step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendWindow {
    pub anytime: bool,
    /// JSON text such as `["monday","tuesday"]`; `None` or invalid JSON =
    /// every day.
    pub send_days: Option<String>,
    /// `"HH:MM"`.
    pub send_time_start: Option<String>,
    /// `"HH:MM"`; a time `>=` the end rolls to the next allowed day.
    pub send_time_end: Option<String>,
}

/// `calculateNextValidSendTime(baseTime, step)` in zone `tz`; unix ms in
/// and out.
pub fn calculate_next_valid_send_time(base_ms: i64, window: &SendWindow, tz: Tz) -> i64 {
    todo!()
}
