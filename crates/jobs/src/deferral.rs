//! Lock-contention deferral and the connection send gate (spec 1.4, S7):
//! ports of `apps/worker/src/lib/lock-contention-deferral.ts` and
//! `apps/worker/src/chat/utils/connection-send-gate.ts`.
//!
//! Both park a job with `JobError::Delayed { run_at_ms }` (BullMQ
//! `moveToDelayed` + `DelayedError`) and keep a deferral budget counted from
//! `attempts_started - attempts_made - 1`.

use std::fmt;
use std::future::Future;

use serde_json::Value;

use crate::clock::Clock;
use crate::ctx::{BoxError, JobCtx, JobError};
use crate::enqueue::BoxFuture;
use crate::sequence::schedule::ScheduleError;

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
    attempts_started
        .saturating_sub(attempts_made)
        .saturating_sub(1)
}

/// `lockContentionDelayMs(deferral, random)` with [`LOCK_CONTENTION_POLICY`].
pub fn lock_contention_delay_ms(deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    policy_delay_ms(&LOCK_CONTENTION_POLICY, deferral, rand)
}

/// `deferralDelayMs(deferral, random)` with [`CONNECTION_SEND_DEFERRAL_POLICY`].
pub fn deferral_delay_ms(deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    policy_delay_ms(&CONNECTION_SEND_DEFERRAL_POLICY, deferral, rand)
}

/// The equal-jitter delay of `policy` for `deferral` (shared formula).
pub fn policy_delay_ms(policy: &DeferralPolicy, deferral: u32, rand: &dyn Fn() -> f64) -> u64 {
    let grown =
        policy.base_delay_ms as f64 * 2f64.powi(i32::try_from(deferral).unwrap_or(i32::MAX));
    let ceiling = grown.min(policy.max_delay_ms as f64);
    let half = ceiling / 2.0;
    (half + rand() * half).round() as u64
}

/// `LockAcquisitionError` of the P2.7 keyed lock ("Failed to acquire lock").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisitionError {
    pub message: String,
}

impl fmt::Display for LockAcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LockAcquisitionError {}

/// Whether `err` is a lock-acquisition error: a [`LockAcquisitionError`]
/// or a `ScheduleError::LockAcquisition`, itself or anywhere in its
/// `source()` chain (the TypeScript check by `name`).
pub fn is_lock_acquisition_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if e.is::<LockAcquisitionError>()
            || e.is::<crate::sequence::consumer::LockAcquisitionError>()
            || matches!(
                e.downcast_ref::<ScheduleError>(),
                Some(ScheduleError::LockAcquisition { .. })
            )
        {
            return true;
        }
        current = e.source();
    }
    false
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
    let error = match run.await {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if job.lease_token.is_none() || !is_lock_acquisition_error(error.as_ref()) {
        return Err(JobError::Retryable(error));
    }
    let deferral = deferrals_so_far(job.attempts_started, job.attempts_made);
    if deferral >= LOCK_CONTENTION_POLICY.max_deferrals {
        return Err(JobError::Retryable(error));
    }
    let delay_ms = lock_contention_delay_ms(deferral, rand);
    tracing::warn!(
        err = %error,
        job_id = %job.job_id,
        job_name = %job.name,
        deferral,
        delay_ms,
        "Distributed lock contention; deferring job instead of failing it"
    );
    Err(JobError::Delayed {
        run_at_ms: clock
            .now_ms()
            .saturating_add(i64::try_from(delay_ms).unwrap_or(i64::MAX)),
    })
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
    let action = job.data.get("type").and_then(Value::as_str);
    if !action.is_some_and(|a| GATED_CHAT_ACTIONS.contains(&a)) {
        return Ok(());
    }
    let inbox_id = job
        .data
        .pointer("/data/contactInbox/inboxId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let Some(inbox_id) = inbox_id else {
        return Ok(());
    };
    if job.lease_token.is_none() {
        return Ok(());
    }
    // The gate only holds a send back, never blocks one: a failed lookup sends.
    let decision = match state.resolve_send_decision(inbox_id).await {
        Ok(decision) => decision,
        Err(err) => {
            tracing::warn!(
                err = %err,
                job_id = %job.job_id,
                inbox_id,
                "Could not read the account state for a send; sending without the reconnect check"
            );
            return Ok(());
        }
    };
    if decision != SendDecision::Defer {
        return Ok(());
    }
    let deferral = deferrals_so_far(job.attempts_started, job.attempts_made);
    if deferral >= CONNECTION_SEND_DEFERRAL_POLICY.max_deferrals {
        tracing::warn!(
            job_id = %job.job_id,
            inbox_id,
            deferral,
            "Account still reconnecting after the deferral budget; sending anyway"
        );
        return Ok(());
    }
    let delay_ms = deferral_delay_ms(deferral, rand);
    tracing::info!(
        job_id = %job.job_id,
        inbox_id,
        deferral,
        delay_ms,
        "Account is reconnecting; deferring send"
    );
    Err(JobError::Delayed {
        run_at_ms: clock
            .now_ms()
            .saturating_add(i64::try_from(delay_ms).unwrap_or(i64::MAX)),
    })
}
