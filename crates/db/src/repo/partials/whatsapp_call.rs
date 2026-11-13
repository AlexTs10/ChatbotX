//! Port of `partials/whatsapp-call.ts` (call statuses and outcomes).

/// `whatsappCallStatuses`.
pub const WHATSAPP_CALL_STATUSES: &[&str] =
    &["ringing", "accepted", "rejected", "completed", "failed"];
/// `whatsappCallOutcomes`.
pub const WHATSAPP_CALL_OUTCOMES: &[&str] = &["completed", "failed", "rejected", "canceled"];

/// `WhatsappCallTerminalStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsappCallTerminalStatus {
    Completed,
    Rejected,
    Failed,
}

/// `WhatsappCallOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsappCallOutcome {
    Completed,
    Failed,
    Rejected,
    Canceled,
}

impl WhatsappCallOutcome {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        todo!("P3.2b")
    }
}

/// `OUTCOME_BY_TERMINAL_STATUS`: the outcome of each terminal status when the call was
/// not a business cancel, as `(status, outcome)` pairs.
pub const OUTCOME_BY_TERMINAL_STATUS: &[(&str, &str)] = &[
    ("completed", "completed"),
    ("rejected", "rejected"),
    ("failed", "failed"),
];

/// `resolveWhatsappCallOutcome`.
pub fn resolve_whatsapp_call_outcome(
    status: WhatsappCallTerminalStatus,
    canceled_by_business: bool,
) -> WhatsappCallOutcome {
    let _ = (status, canceled_by_business);
    todo!("P3.2b")
}

/// `resolveWhatsappCallTerminalOutcomePair`: `(status, outcome)`.
pub fn resolve_whatsapp_call_terminal_outcome_pair(
    status: WhatsappCallTerminalStatus,
    canceled_by_business: bool,
) -> (WhatsappCallTerminalStatus, WhatsappCallOutcome) {
    let _ = (status, canceled_by_business);
    todo!("P3.2b")
}

/// `resolveDisplayCallOutcome`: `coalesce(outcome, status)` for terminal rows; `None`
/// while ringing / accepted (texts as stored).
pub fn resolve_display_call_outcome(status: &str, outcome: Option<&str>) -> Option<String> {
    let _ = (status, outcome);
    todo!("P3.2b")
}
