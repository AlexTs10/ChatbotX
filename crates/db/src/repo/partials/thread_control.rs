//! Port of the constants and helpers of `partials/thread-control.ts`.

/// `THREAD_IDLE_AFTER_MS`.
pub const THREAD_IDLE_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
/// `THREAD_CONTROL_INBOX_ACTIVE_MS`.
pub const THREAD_CONTROL_INBOX_ACTIVE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// `THREAD_CONTROL_SEEN_REFRESH_MS`.
pub const THREAD_CONTROL_SEEN_REFRESH_MS: i64 = 24 * 60 * 60 * 1000;

/// `THREAD_CONTROL_EVENT_PRECEDENCE`: tie-break order, lowest first.
pub const THREAD_CONTROL_EVENT_PRECEDENCE: &[&str] = &[
    "inboundReceived",
    "standbyReceived",
    "serviceSent",
    "serviceRejected",
    "passed",
    "released",
    "taken",
    "controlPassed",
    "controlTaken",
];

/// `THREAD_CONTROL_TRANSITIONS`: the state an event moves the thread to.
pub fn transition(event: &str) -> Option<&'static str> {
    match event {
        "inboundReceived" | "controlPassed" | "taken" | "serviceSent" => Some("owned"),
        "standbyReceived" | "controlTaken" | "passed" | "serviceRejected" => Some("standby"),
        "released" => Some("idle"),
        _ => None,
    }
}

/// `eventsOutrankedBy`: the events that lose a same-time tie against `event`.
pub fn events_outranked_by(event: &str) -> &'static [&'static str] {
    let rank = THREAD_CONTROL_EVENT_PRECEDENCE.iter().position(|e| *e == event).unwrap_or(0);
    &THREAD_CONTROL_EVENT_PRECEDENCE[..rank]
}

/// `isInboxThreadControlActive`.
pub fn is_inbox_thread_control_active(seen_at: Option<i64>, now: i64) -> bool {
    seen_at.is_some_and(|s| now - s < THREAD_CONTROL_INBOX_ACTIVE_MS)
}
