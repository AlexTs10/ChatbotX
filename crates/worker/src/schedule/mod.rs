//! `apps/worker/src/schedule/worker.ts`: the `schedule` queue processor.
//! Q.D5 ports the broadcast and smart delay jobs; the other schedule jobs are Q.W2.

pub mod broadcast;
pub mod recurring;

use chatbotx_jobs::schedules::{
    Edition, PURGE_BROADCASTS_INTERVAL_MINUTES, PURGE_WORKSPACES_INTERVAL_MINUTES,
    QUOTA_SYNC_INTERVAL_SECONDS, ScheduleConfig, SchedulerStore, schedules,
};
use chatbotx_jobs::{JobCtx, JobError, JobsError};
use serde_json::Value;

use crate::{Deps, str_of};

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
        // ponytail: the other schedule jobs are Q.W2. An unported job fails
        // (kept in the failed set) instead of the original's silent
        // completion, as the `default` queue does.
        other => {
            return Err(JobError::unrecoverable(format!(
                "ScheduleJobData type is not defined: {other}"
            )));
        }
    };
    Ok(out)
}

/// `registerSchedules` for the ported jobs ([`BROADCAST_JOBS`]); Q.W2
/// registers the rest. A registration error is logged, as the original.
pub async fn register_schedules(store: &dyn SchedulerStore) {
    let config = ScheduleConfig {
        edition: Edition::Community,
        quota_sync_interval_seconds: QUOTA_SYNC_INTERVAL_SECONDS,
        purge_workspaces_interval_minutes: PURGE_WORKSPACES_INTERVAL_MINUTES,
        purge_broadcasts_interval_minutes: PURGE_BROADCASTS_INTERVAL_MINUTES,
        license_refresh_minute: 0,
    };
    let result: Result<(), JobsError> = async {
        for def in schedules(&config) {
            if BROADCAST_JOBS.contains(&def.template.name.as_str()) {
                store
                    .upsert_job_scheduler(&def.id, &def.repeat, &def.template)
                    .await?;
            }
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => tracing::info!("Schedules registered"),
        Err(err) => tracing::error!(err = %err, "Error registering schedules"),
    }
}
