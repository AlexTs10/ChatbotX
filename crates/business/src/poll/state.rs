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
        match self {
            PollState::NotStarted => "notStarted",
            PollState::Running => "running",
            PollState::Ended => "ended",
        }
    }
}

/// `getPollState`: running from `startsAt` (inclusive) to `endsAt` (exclusive).
pub fn get_poll_state(starts_at: i64, ends_at: i64, now: i64) -> PollState {
    if now < starts_at {
        return PollState::NotStarted;
    }
    if now >= ends_at {
        return PollState::Ended;
    }
    PollState::Running
}
