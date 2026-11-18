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
    "purgeCommentAutomationEvents",
    "purgeExpiredConnectSessions",
    "purgeWhatsappSignupSessions",
    "purgeCoexistStaging",
    "purgeExpiredOAuthTokens",
    "purgeAutomationThrottle",
    "expireUnpaidOrders",
    "scanAppointmentReminders",
    "enqueueMarketingMessages",
    "dispatchMarketingMessages",
    "finalizeMarketingMessages",
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
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    let data = job.data["data"].clone();
    let id = |key: &str| {
        crate::id_of(&data[key]).ok_or_else(|| JobError::unrecoverable(format!("{kind}: bad data")))
    };
    let out = match kind.as_str() {
        "enqueueBroadcast" => broadcast::enqueue_broadcast(&deps).await?,
        "prepareBroadcast" => broadcast::prepare_broadcast(&deps, id("broadcastId")?).await?,
        "sendBroadcast" => broadcast::process_broadcast_contacts(&deps, id("broadcastId")?).await?,
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
        "purgeErrorLogs" => {
            purge::purge_error_logs(&deps.db).await?;
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
