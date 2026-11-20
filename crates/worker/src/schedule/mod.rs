//! `apps/worker/src/schedule/worker.ts`: the `schedule` queue processor.
//! Only the crons whose handlers are in Rust are registered, so an unported
//! one never wakes the process. Q.D5: broadcast and smart delay jobs; Q.W2:
//! AI hand-over scan and the retention purges.

pub mod broadcast;
pub mod recurring;

use chatbotx_jobs::schedules::{
    Edition, PURGE_BROADCASTS_INTERVAL_MINUTES, PURGE_WORKSPACES_INTERVAL_MINUTES,
    QUOTA_SYNC_INTERVAL_SECONDS, ScheduleConfig, SchedulerStore, schedules,
};
use chatbotx_jobs::{JobCtx, JobError, JobsError};
use serde_json::Value;

use crate::{Deps, purge, str_of};

/// The scheduler ids with a Rust handler (Q.W2, see [`handle`]).
pub const PORTED: &[&str] = &[
    "scanAiHandoverBulkRuns",
    "scanContactScans",
    "evaluateTriggers",
    "cleanupTriggers",
    "evaluateDateTimeWebhooks",
    "cleanupWebhookExecutions",
    "purgeErrorLogs",
    "purgeInboundWebhookRequests",
    "redispatchInboundWebhookRequests",
    "purgeCommentAutomationEvents",
    "purgeExpiredConnectSessions",
    "purgeWhatsappSignupSessions",
    "purgeCoexistStaging",
    "scanCoexistRuns",
    "purgeExpiredOAuthTokens",
    "purgeAutomationThrottle",
    "expireUnpaidOrders",
    "scanAppointmentReminders",
    "enqueueMarketingMessages",
    "dispatchMarketingMessages",
    "finalizeMarketingMessages",
    "redispatchTemplateJobs",
    "googleAdsHousekeeping",
    "googleAdsSyncSetups",
    "sweepStaleWhatsappCalls",
    "purgeExpiredCallRecordings",
    "finishOverdueLeaderboards",
    "refreshChannelTokens",
    "refreshShortLivedChannelTokens",
    "dispatchProfileSnapshots",
    "reconcileMetaCatalogSyncs",
    "scanAbandonedCarts",
    "purgeWorkspaces",
];

/// `ScheduleJobData` names of the ported jobs (broadcasts, smart delay).
pub const BROADCAST_JOBS: &[&str] = &[
    "enqueueBroadcast",
    "prepareBroadcast",
    "sendBroadcast",
    "reconcileBroadcasts",
    "finalizeBroadcasts",
    "purgeBroadcasts",
    "purgeDraftBroadcastRecipients",
    "enqueueRecurringBroadcasts",
    "materializeRecurringBroadcasts",
    "scanSmartDelay",
];

/// The `createQueueWorker(schedule, ..)` switch.
/// Proves `Send` at a concrete lifetime (works around the rustc
/// "`Send` is not general enough" error of the large `handle` future).
fn send<'a, T>(
    f: impl std::future::Future<Output = T> + Send + 'a,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>> {
    Box::pin(f)
}

pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    let data = job.data["data"].clone();
    let id = |key: &str| {
        crate::id_of(&data[key]).ok_or_else(|| JobError::unrecoverable(format!("{kind}: bad data")))
    };
    let out = match kind.as_str() {
        "enqueueBroadcast" => broadcast::enqueue_broadcast(&deps).await?,
        "prepareBroadcast" => send(broadcast::prepare_broadcast(&deps, id("broadcastId")?)).await?,
        "sendBroadcast" => {
            send(broadcast::process_broadcast_contacts(
                &deps,
                id("broadcastId")?,
            ))
            .await?
        }
        "reconcileBroadcasts" => broadcast::reconcile_broadcasts(&deps).await?,
        "finalizeBroadcasts" => broadcast::finalize_broadcasts(&deps).await?,
        "purgeBroadcasts" => broadcast::purge_broadcasts(&deps).await?,
        "enqueueRecurringBroadcasts" => recurring::enqueue_recurring_broadcasts(&deps).await?,
        "scanSmartDelay" => crate::smart_delay::scan_smart_delay(&deps).await?,
        "scanAiHandoverBulkRuns" => {
            crate::ai_handover::scan(&deps).await?;
            Value::Null
        }
        "scanContactScans" => {
            crate::contact_scan::sweep(&deps).await?;
            Value::Null
        }
        "evaluateTriggers" => {
            crate::datetime_sweep::scan_triggers(&deps).await?;
            Value::Null
        }
        "cleanupTriggers" => {
            crate::datetime_sweep::cleanup(&deps, false).await?;
            Value::Null
        }
        "evaluateDateTimeWebhooks" => {
            crate::datetime_sweep::scan_webhooks(&deps).await?;
            Value::Null
        }
        "cleanupWebhookExecutions" => {
            crate::datetime_sweep::cleanup(&deps, true).await?;
            Value::Null
        }
        "sweepStaleWhatsappCalls" => {
            crate::whatsapp_voip::sweep_stale_whatsapp_calls(&deps).await?;
            Value::Null
        }
        "purgeExpiredCallRecordings" => {
            crate::whatsapp_voip::purge_expired_call_recordings(&deps).await?;
            Value::Null
        }
        "redispatchTemplateJobs" => {
            crate::template::redispatch(&deps).await?;
            Value::Null
        }
        "purgeErrorLogs" => {
            purge::purge_error_logs(&deps.db).await?;
            Value::Null
        }
        "redispatchInboundWebhookRequests" => {
            crate::inbound_webhook::redispatch(&deps).await?;
            Value::Null
        }
        "purgeInboundWebhookRequests" => {
            purge::purge_inbound_webhook_requests(&deps.db).await?;
            Value::Null
        }
        "purgeCommentAutomationEvents" => {
            purge::purge_comment_automation_events(&deps.db).await?;
            Value::Null
        }
        "purgeExpiredConnectSessions" => {
            purge::purge_expired_connect_sessions(&deps.db).await?;
            Value::Null
        }
        "purgeWhatsappSignupSessions" => {
            purge::purge_whatsapp_signup_sessions(&deps.db).await?;
            Value::Null
        }
        crate::coexist_whatsapp::SCAN_JOB => {
            crate::coexist_whatsapp::scan_runs(&deps).await?;
            Value::Null
        }
        "purgeCoexistStaging" => {
            purge::purge_coexist_staging(&deps.db).await?;
            Value::Null
        }
        "purgeExpiredOAuthTokens" => {
            purge::purge_expired_oauth_tokens(&deps.db).await?;
            Value::Null
        }
        "expireUnpaidOrders" => crate::order_payment::expire_unpaid_orders(&deps).await?,
        "purgeAutomationThrottle" => {
            purge::purge_automation_throttle(&deps.db).await?;
            Value::Null
        }
        "enqueueMarketingMessages" => {
            crate::marketing_message::enqueue_marketing_messages(&deps).await?
        }
        "dispatchMarketingMessages" => {
            crate::marketing_message::dispatch_marketing_messages(&deps).await?
        }
        "finalizeMarketingMessages" => {
            crate::marketing_message::finalize_marketing_messages(&deps).await?
        }
        "scanAppointmentReminders" => {
            crate::appointment::scan_reminders(&deps).await?;
            Value::Null
        }
        "materializeRecurringBroadcasts" => {
            let items = data["items"].as_array().cloned().unwrap_or_default();
            recurring::materialize_recurring_broadcasts(&deps, &items).await?
        }
        "purgeDraftBroadcastRecipients" => {
            broadcast::purge_draft_broadcast_recipients(
                &deps,
                &job,
                id("workspaceId")?,
                id("broadcastId")?,
            )
            .await?
        }
        "reconcileMetaCatalogSyncs" => {
            crate::meta_catalog::sync::reconcile(&deps, chatbotx_db::repo::now_ms()).await?
        }
        "googleAdsHousekeeping" => {
            crate::google_ads_housekeeping::housekeeping(&deps).await?;
            Value::Null
        }
        "refreshChannelTokens" => {
            crate::contact_profile::refresh_channel_tokens(&deps, &data).await;
            Value::Null
        }
        "dispatchProfileSnapshots" => {
            crate::contact_profile::dispatch_profile_snapshots(&deps).await?;
            Value::Null
        }
        "googleAdsSyncSetups" => {
            crate::google_ads_housekeeping::sync_setups(&deps).await?;
            Value::Null
        }
        "scanAbandonedCarts" => send(scan_abandoned_carts(&deps)).await?,
        "purgeWorkspaces" => {
            purge_workspaces(&deps).await?;
            Value::Null
        }
        other if crate::leaderboard::run_schedule(&deps, other).await => Value::Null,
        other => {
            tracing::warn!(job_type = other, "Unknown schedule job type");
            Value::Null
        }
    };
    Ok(out)
}

/// `registerSchedules()` for the ported jobs ([`PORTED`] ids and
/// [`BROADCAST_JOBS`] names).
pub async fn register(store: &dyn SchedulerStore) -> Result<(), JobsError> {
    let config = ScheduleConfig {
        edition: Edition::Community,
        quota_sync_interval_seconds: QUOTA_SYNC_INTERVAL_SECONDS,
        purge_workspaces_interval_minutes: PURGE_WORKSPACES_INTERVAL_MINUTES,
        purge_broadcasts_interval_minutes: PURGE_BROADCASTS_INTERVAL_MINUTES,
        license_refresh_minute: 0,
    };
    for def in schedules(&config) {
        if PORTED.contains(&def.id.as_str()) || BROADCAST_JOBS.contains(&def.template.name.as_str())
        {
            store
                .upsert_job_scheduler(&def.id, &def.repeat, &def.template)
                .await?;
        }
    }
    Ok(())
}

/// `scanAbandonedCarts` (every minute): claims the due carts in batches of
/// 100 (at most 20 batches a run) and reminds each; a failed reminder is
/// logged, not retried; a blocked owner's carts are claimed and skipped.
/// ponytail: no distributed lock (one worker process runs the schedules).
async fn scan_abandoned_carts(deps: &Deps) -> Result<Value, JobError> {
    use chatbotx_business::webshop::{Reminder, claim_abandoned_carts, remind_abandoned_cart};
    const BATCH_SIZE: i64 = 100;
    const MAX_BATCHES_PER_RUN: usize = 20;
    let bus = crate::order_payment::bus(deps);
    let mut reminded = 0;
    for _ in 0..MAX_BATCHES_PER_RUN {
        let carts = claim_abandoned_carts(&deps.db, BATCH_SIZE, chatbotx_db::repo::now_ms())
            .await
            .map_err(|e| JobError::retryable(e.to_string()))?;
        for cart in &carts {
            if crate::is_blocked_workspace(&deps.db, &cart.workspace_id.to_string()).await? {
                continue;
            }
            match remind_abandoned_cart(&deps.db, deps.jobs.as_ref(), &bus, *cart).await {
                Ok(Reminder::Reminded) => reminded += 1,
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(err = %err, cart_id = cart.id, "Abandoned cart reminder failed");
                }
            }
        }
        if (carts.len() as i64) < BATCH_SIZE {
            break;
        }
    }
    if reminded > 0 {
        tracing::info!(reminded, "Abandoned cart scanner reminded carts");
    }
    Ok(serde_json::json!({ "reminded": reminded }))
}

/// `purgeWorkspaces`: `purgeDueScheduled` with the payment-provider
/// teardown; one run at a time in this process.
/// ponytail: `allIntegrations` (provider disconnects) is not passed, as in
/// `purge_due_scheduled` (its module doc).
async fn purge_workspaces(deps: &Deps) -> Result<(), JobError> {
    use chatbotx_business::workspace::purge::{PaymentTeardown, PurgeOptions, purge_due_scheduled};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        tracing::warn!("purgeWorkspaces: skipped because a local purge is still running");
        return Ok(());
    }
    let db = Arc::clone(&deps.db);
    let app_url = deps.app_url.clone();
    let teardown: PaymentTeardown = Arc::new(move |workspace_id| {
        let (db, app_url) = (Arc::clone(&db), app_url.clone());
        Box::pin(async move {
            let ctx = chatbotx_business::payment_provider::Ctx {
                db: &db,
                encryption: chatbotx_business::meta_catalog::process_encryption(),
                app_url: &app_url,
            };
            chatbotx_business::payment_provider::teardown_for_purge(ctx, workspace_id).await
        })
    });
    let options = PurgeOptions {
        teardown_payment_providers: Some(teardown),
        ..PurgeOptions::default()
    };
    // `purge_due_scheduled` is not `Send`: it runs on its own thread and
    // runtime.
    let db = Arc::clone(&deps.db);
    let outcome = tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(purge_due_scheduled(&db, options))
            .map_err(|e| e.to_string())
    })
    .await;
    RUNNING.store(false, Ordering::SeqCst);
    let deleted = outcome
        .map_err(|e| e.to_string())
        .and_then(|r| r)
        .map_err(JobError::retryable)?;
    if deleted > 0 {
        tracing::info!(deleted, "purgeWorkspaces: workspaces purged");
    }
    Ok(())
}
