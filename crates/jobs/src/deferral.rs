//! Lock-contention deferral and the connection send gate (spec 1.4, S7):
//! ports of `apps/worker/src/lib/lock-contention-deferral.ts` and
//! `apps/worker/src/chat/utils/connection-send-gate.ts`.
//!
//! Both park a job with `JobError::Delayed { run_at_ms }` (BullMQ
//! `moveToDelayed` + `DelayedError`) and keep a deferral budget counted from
//! `attempts_started - attempts_made - 1`.

use std::fmt;
use std::future::Future;

use crate::clock::Clock;
use crate::ctx::{BoxError, JobCtx, JobError};
use crate::enqueue::BoxFuture;

/// Backoff policy of a deferral: delay = round(c/2 + r * c/2) with
/// c = min(base * 2^deferral, max); at most `max_deferrals` deferrals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferralPolicy {
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub max_deferrals: u32,
}

/// `LOCK_CONTENTION_POLICY` (integration worker).
pub const LOCK_CONTENTION_POLICY: DeferralPolicy = DeferralPolicy {
    base_delay_ms: 2000,
    max_delay_ms: 30_000,
    max_deferrals: 8,
};

/// How long the integration worker waits for a contended lock before it
/// defers (`lockWait`, 10 s).
pub const LOCK_CONTENTION_LOCK_WAIT_MS: u64 = 10_000;

/// `CONNECTION_SEND_DEFERRAL_POLICY` (chat send gate).
pub const CONNECTION_SEND_DEFERRAL_POLICY: DeferralPolicy = DeferralPolicy {
    base_delay_ms: 30_000,
    max_delay_ms: 300_000,
    max_deferrals: 5,
};

/// `deferralsSoFar({ attemptsStarted, attemptsMade })`:
/// `max(0, attempts_started - attempts_made - 1)`.
pub fn deferrals_so_far(attempts_started: u32, attempts_made: u32) -> u32 {
    todo!()
}

/// `lockContentionDelayMs(deferral, random)` with [`LOCK_CONTENTION_POLICY`].
pub fn lock_contention_delay_ms(deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    todo!()
}

/// `deferralDelayMs(deferral, random)` with [`CONNECTION_SEND_DEFERRAL_POLICY`].
pub fn deferral_delay_ms(deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    todo!()
}

/// The equal-jitter delay of `policy` for `deferral` (shared formula).
pub fn policy_delay_ms(policy: &DeferralPolicy, deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    todo!()
}

/// `LockAcquisitionError` of the P2.7 keyed lock ("Failed to acquire lock").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisitionError {
    pub message: String,
}

impl fmt::Display for LockAcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for LockAcquisitionError {}

/// Whether `err` is a lock-acquisition error: a [`LockAcquisitionError`]
/// or a `ScheduleError::LockAcquisition`, itself or anywhere in its
/// `source()` chain (the TypeScript check by `name`).
pub fn is_lock_acquisition_error(err: &(dyn std::error::Error + 'static)) -> bool {
    todo!()
}

/// `deferOnLockContention(job, token, processor)`: runs `run`; on a
/// lock-acquisition error, while the job holds a lease and the budget
/// allows, returns `JobError::Delayed { run_at_ms: now + delay }` and logs a
/// warn with fields `err`, `job_id`, `deferral`. Any other error, or a
/// lock error without lease or over budget, is returned unchanged as
/// `JobError::Retryable`.
pub async fn defer_on_lock_contention<T, F>(
    job: &JobCtx,
    clock: &dyn Clock,
    rand: &dyn Fn() -> f64,
    run: F,
) -> Result<T, JobError>
where
    F: Future<Output = Result<T, BoxError>>,
{
    todo!()
}

/// `connectionStateService.resolveSendDecision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDecision {
    Send,
    Defer,
}

/// The connection-state lookup the send gate uses (fake in tests).
pub trait ConnectionState: Send + Sync {
    /// `resolveSendDecision({ inboxId })`.
    fn resolve_send_decision<'a>(
        &'a self,
        inbox_id: &'a str,
    ) -> BoxFuture<'a, Result<SendDecision, BoxError>>;
}

/// The chat job actions the gate checks (`sendChannelMessage`,
/// `sendWhatsappTemplateMessage`, `sendWhatsappTemplateToConversation`,
/// `sendMessengerTemplateMessage`); other actions are never gated.
pub const GATED_CHAT_ACTIONS: [&str; 4] = [
    "sendChannelMessage",
    "sendWhatsappTemplateMessage",
    "sendWhatsappTemplateToConversation",
    "sendMessengerTemplateMessage",
];

/// `deferSendWhileAccountReconnecting(job, token)`: for a gated chat job
/// (`job.data = { type, data: { contactInbox: { inboxId } } }`) with a lease,
/// asks `state` for its inbox; `Defer` inside the budget parks the job
/// (`JobError::Delayed`). Not gated, no lease, budget spent, `Send`, or a
/// failing lookup → `Ok(())` (send).
pub async fn defer_send_while_account_reconnecting(
    job: &JobCtx,
    state: &dyn ConnectionState,
    clock: &dyn Clock,
    rand: &dyn Fn() -> f64,
) -> Result<(), JobError> {
    todo!()
}
