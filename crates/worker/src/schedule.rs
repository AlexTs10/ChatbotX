//! The `schedule` queue (`schedule/worker.ts`). Only the crons whose
//! handlers are in Rust are registered, so an unported one never wakes the
//! process.

use chatbotx_jobs::schedules::{Edition, ScheduleConfig, SchedulerStore, schedules};
use chatbotx_jobs::{JobCtx, JobError, JobsError};
use serde_json::Value;

use crate::{Deps, str_of};

/// The scheduler ids with a Rust handler (see [`handle`]).
pub const PORTED: &[&str] = &["scanAiHandoverBulkRuns"];

/// `registerSchedules()` for the [`PORTED`] ids. The other config fields
/// only shape unported entries.
pub async fn register(store: &dyn SchedulerStore) -> Result<(), JobsError> {
    let config = ScheduleConfig {
        edition: Edition::Community,
        quota_sync_interval_seconds: 60,
        purge_workspaces_interval_minutes: 60,
        purge_broadcasts_interval_minutes: 60,
        license_refresh_minute: 0,
    };
    for def in schedules(&config)
        .into_iter()
        .filter(|d| PORTED.contains(&d.id.as_str()))
    {
        store
            .upsert_job_scheduler(&def.id, &def.repeat, &def.template)
            .await?;
    }
    Ok(())
}

/// The `schedule` queue processor (`switch (job.data.type)`).
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    match str_of(&job.data, "type").unwrap_or("") {
        "scanAiHandoverBulkRuns" => crate::ai_handover::scan(&deps).await?,
        other => tracing::warn!(job_type = other, "Unknown schedule job type"),
    }
    Ok(Value::Null)
}
