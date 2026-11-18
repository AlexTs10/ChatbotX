//! `integration/handlers/confirm-order-payment.ts` and
//! `schedule/handlers/expire-unpaid-orders.ts`: web shop payments.
//! `withBlockedOwnerGuard` is `is_blocked_workspace`: a blocked owner's
//! event is dropped, their orders and attempts are looked at again later.

use std::sync::{Arc, LazyLock};

use chatbotx_business::events::EventBus;
use chatbotx_business::events::cache::ActiveCache;
use chatbotx_business::events::emitter::QueueEmitter;
use chatbotx_business::order::Deps as OrderDeps;
use chatbotx_business::order_payment::{self, ExpireOutcome};
use chatbotx_business::payments::PaymentMetadata;
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_jobs::JobError;
use chatbotx_kv::lock::{LockMap, LockOpts};
use serde_json::{Value, json};

use crate::{Deps, fail, is_blocked_workspace, str_of};

pub const CONFIRM_ORDER_PAYMENT: &str = "confirmOrderPayment";

const BATCH_SIZE: i64 = 50;
/// A backlog larger than this waits for the next minute.
const MAX_BATCHES_PER_RUN: usize = 10;
const BLOCKED_OWNER_RECHECK_MS: i64 = 60 * 60_000;

static SCHEDULE_LOCK: LazyLock<LockMap> = LazyLock::new(LockMap::new);
static ENCRYPTION: LazyLock<Option<Encryption>> = LazyLock::new(|| {
    Keys::from_env(&chatbotx_core::env::ProcessEnv)
        .ok()
        .map(Encryption::new)
});

/// A trigger / webhook bus like the API's.
pub(crate) fn bus(deps: &Deps) -> EventBus {
    let events = EventBus::new();
    events.subscribe(Arc::new(QueueEmitter::trigger(
        Arc::clone(&deps.jobs),
        Arc::new(ActiveCache::triggers(Arc::clone(&deps.db))),
    )));
    events.subscribe(Arc::new(QueueEmitter::webhook(
        Arc::clone(&deps.jobs),
        Arc::new(ActiveCache::webhooks(Arc::clone(&deps.db))),
    )));
    events
}

/// The order services' side effects: the worker's jobs and realtime, `events`, the env keys.
pub(crate) fn order_deps<'a>(deps: &'a Deps, events: &'a EventBus) -> OrderDeps<'a> {
    OrderDeps {
        db: &deps.db,
        jobs: Some(deps.jobs.as_ref()),
        events,
        realtime: deps.realtime.as_ref(),
        encryption: ENCRYPTION.as_ref(),
        app_url: &deps.app_url,
    }
}

/// `confirmOrderPayment`: the provider is asked again and the order settled.
/// A refund that fails throws, so the job is retried.
pub async fn confirm_order_payment(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let provider = str_of(data, "provider").unwrap_or_default().to_string();
    let reference = str_of(data, "reference").unwrap_or_default().to_string();
    let Some(workspace_id) = crate::id_of(&data["workspaceId"]) else {
        return Err(JobError::unrecoverable("confirmOrderPayment: bad data"));
    };
    if is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
        return Ok(());
    }
    let metadata: Option<PaymentMetadata> = serde_json::from_value(data["metadata"].clone()).ok();
    let events = bus(deps);
    order_payment::handle_webhook_event(
        order_deps(deps, &events),
        &provider,
        workspace_id,
        &reference,
        metadata.as_ref(),
    )
    .await
    .map_err(|e| fail(e.message()))
}

/// `expireUnpaidOrders`: online orders past their deadline, then the open
/// attempts past their page's end.
pub async fn expire_unpaid_orders(deps: &Deps) -> Result<Value, JobError> {
    let opts = LockOpts {
        ttl_secs: 55,
        retry_secs: Some(0),
    };
    let Ok(_guard) = SCHEDULE_LOCK
        .acquire("schedule:expire-unpaid-orders", opts)
        .await
    else {
        return Ok(Value::Null);
    };
    let events = bus(deps);
    let d = order_deps(deps, &events);
    {
        let (mut cancelled, mut paid, mut deferred) = (0, 0, 0);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..MAX_BATCHES_PER_RUN {
            let orders: Vec<_> = order_payment::list_overdue(d.db, BATCH_SIZE)
                .await
                .map_err(|e| fail(e.message()))?
                .into_iter()
                .filter(|(_, id)| !seen.contains(id))
                .collect();
            if orders.is_empty() {
                break;
            }
            for (workspace_id, order_id) in orders {
                seen.insert(order_id);
                if is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                    let until = chatbotx_core::js::now_ms() + BLOCKED_OWNER_RECHECK_MS;
                    if let Err(error) =
                        order_payment::postpone_expiry(d, workspace_id, order_id, until).await
                    {
                        tracing::warn!(%error, order_id, "Expiring an unpaid order failed");
                    }
                    deferred += 1;
                    continue;
                }
                match order_payment::expire_order(d, workspace_id, order_id).await {
                    Ok(ExpireOutcome::Cancelled) => cancelled += 1,
                    Ok(ExpireOutcome::Paid) => paid += 1,
                    Ok(ExpireOutcome::Processing | ExpireOutcome::RetryLater) => deferred += 1,
                    Ok(ExpireOutcome::Skipped) => {}
                    Err(error) => {
                        tracing::warn!(%error, order_id, "Expiring an unpaid order failed")
                    }
                }
            }
        }
        let mut stale = 0;
        for attempt in order_payment::list_stale(d.db, BATCH_SIZE)
            .await
            .map_err(|e| fail(e.message()))?
        {
            if is_blocked_workspace(&deps.db, &attempt.workspace_id.to_string()).await? {
                if let Err(error) = order_payment::postpone_stale_attempt(d, &attempt).await {
                    tracing::warn!(%error, order_payment_id = attempt.id, "Postponing a stale payment attempt failed");
                }
                continue;
            }
            match order_payment::settle_stale_attempt(d, &attempt).await {
                Ok(()) => stale += 1,
                Err(error) => {
                    tracing::warn!(%error, order_payment_id = attempt.id, "Settling a stale payment attempt failed");
                    // Tried again later, so one that keeps failing never holds the batch.
                    if let Err(error) = order_payment::postpone_stale_attempt(d, &attempt).await {
                        tracing::warn!(%error, order_payment_id = attempt.id, "Postponing a stale payment attempt failed");
                    }
                }
            }
        }
        if cancelled + paid + deferred + stale > 0 {
            tracing::info!(
                cancelled,
                paid,
                deferred,
                stale_attempts = stale,
                "Unpaid order scanner settled orders"
            );
        }
        Ok(
            json!({ "cancelled": cancelled, "paid": paid, "deferred": deferred, "staleAttempts": stale }),
        )
    }
}
