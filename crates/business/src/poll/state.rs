//! `poll/state.ts`.

/// `PollState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollState {
    NotStarted,
    Running,
    Ended,
}

impl PollState {
    /// The JSON value (`notStarted` / `running` / `ended`).
    pub fn as_str(self) -> &'static str {
        todo!("P3.6-D1")
    }
}

/// `getPollState`: running from `startsAt` (inclusive) to `endsAt` (exclusive).
pub fn get_poll_state(starts_at: i64, ends_at: i64, now: i64) -> PollState {
    todo!("P3.6-D1")
}
