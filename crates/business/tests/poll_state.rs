//! Port of `packages/business/__tests__/poll-state.test.ts`.

use chatbotx_business::poll::{PollState, get_poll_state};

const STARTS: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00.000Z
const ENDS: i64 = 1_767_312_000_000; // 2026-01-02T00:00:00.000Z

#[test]
fn get_poll_state_now_equals_starts_at_is_running() {
    assert_eq!(get_poll_state(STARTS, ENDS, STARTS), PollState::Running);
}

#[test]
fn get_poll_state_now_equals_ends_at_is_ended_exclusive() {
    assert_eq!(get_poll_state(STARTS, ENDS, ENDS), PollState::Ended);
}

#[test]
fn get_poll_state_before_starts_at_is_not_started() {
    assert_eq!(
        get_poll_state(STARTS, ENDS, STARTS - 1),
        PollState::NotStarted
    );
}
