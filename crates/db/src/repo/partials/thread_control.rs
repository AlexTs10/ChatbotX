//! Port of the constants and helpers of `partials/thread-control.ts`. States, roles and
//! events are their stored texts; times are unix ms.

use serde_json::Value;

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
    let rank = THREAD_CONTROL_EVENT_PRECEDENCE
        .iter()
        .position(|e| *e == event)
        .unwrap_or(0);
    &THREAD_CONTROL_EVENT_PRECEDENCE[..rank]
}

/// `isInboxThreadControlActive`.
pub fn is_inbox_thread_control_active(seen_at: Option<i64>, now: i64) -> bool {
    seen_at.is_some_and(|s| now - s < THREAD_CONTROL_INBOX_ACTIVE_MS)
}

/// `threadControlStates.options`.
pub const THREAD_CONTROL_STATES: &[&str] = &["owned", "standby", "idle"];

/// `threadControlRoles.options`.
pub const THREAD_CONTROL_ROLES: &[&str] = &[
    "ai_agent",
    "ctwa",
    "customer_service",
    "escalation",
    "marketing",
    "utility",
];

/// `threadControlActions.options`.
pub const THREAD_CONTROL_ACTIONS: &[&str] = &["take", "release", "pass"];

/// `THREAD_CONTROL_METADATA_MAX_LENGTH`.
pub const THREAD_CONTROL_METADATA_MAX_LENGTH: usize = 2000;

/// `threadControlEvents.options`.
pub const THREAD_CONTROL_EVENTS: &[&str] = &[
    "inboundReceived",
    "standbyReceived",
    "controlPassed",
    "controlTaken",
    "taken",
    "released",
    "passed",
    "serviceSent",
    "serviceRejected",
];

/// `THREAD_CONTROL_TRANSITIONS` as (event, state) pairs in the TS key order.
pub const THREAD_CONTROL_TRANSITIONS: &[(&str, &str)] = &[
    ("inboundReceived", "owned"),
    ("controlPassed", "owned"),
    ("taken", "owned"),
    ("serviceSent", "owned"),
    ("standbyReceived", "standby"),
    ("controlTaken", "standby"),
    ("passed", "standby"),
    ("serviceRejected", "standby"),
    ("released", "idle"),
];

/// The input of `resolveThreadControlState` / `isServiceSendBlocked` (unix ms;
/// `thread_owner_expires_at` `None` is the TS `undefined` and `null`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadControlInput<'a> {
    pub state: Option<&'a str>,
    pub last_incoming_message_at: Option<i64>,
    pub thread_control_updated_at: Option<i64>,
    pub thread_owner_expires_at: Option<i64>,
    pub now: i64,
}

/// `resolveThreadControlState`: the stored state, or `idle` once the channel expiry has
/// passed (when set) or 24 h after the latest reference time.
pub fn resolve_thread_control_state<'a>(input: &ThreadControlInput<'a>) -> Option<&'a str> {
    let _ = input;
    todo!("P3.2b")
}

/// `isServiceSendBlocked`: a non-template send on a thread that resolves to `standby`.
pub fn is_service_send_blocked(input: &ThreadControlInput<'_>, is_template_message: bool) -> bool {
    let _ = (input, is_template_message);
    todo!("P3.2b")
}

/// `THREAD_CONTROL_TIMESTAMP_RESOLUTION_MS`.
pub const THREAD_CONTROL_TIMESTAMP_RESOLUTION_MS: i64 = 1000;

/// `toThreadControlTimestamp`: `ms` truncated to whole seconds.
pub fn to_thread_control_timestamp(ms: i64) -> i64 {
    let _ = ms;
    todo!("P3.2b")
}

/// The result of `readThreadControlColumns` (unix ms).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadControlColumns {
    pub state: Option<String>,
    pub last_incoming_message_at: Option<i64>,
    pub thread_control_updated_at: Option<i64>,
    pub thread_owner_expires_at: Option<i64>,
}

/// `readThreadControlColumns`: a row (times as unix ms) or a job payload (times as ISO
/// texts); a missing column reads as `None`.
pub fn read_thread_control_columns(row: &Value) -> ThreadControlColumns {
    let _ = row;
    todo!("P3.2b")
}

/// `parseThreadControlRole`: a known role, else `None` (`value` `None` is `undefined`).
pub fn parse_thread_control_role(value: Option<&Value>) -> Option<&'static str> {
    let _ = value;
    todo!("P3.2b")
}
