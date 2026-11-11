//! Job schedulers (spec 1.3, 2.7, S13): the scheduler table, edition gating
//! (`apps/worker/src/schedule/handlers/register-schedules.ts`), the schedule
//! worker start (`schedule/worker.ts`) and the std-only cron evaluator.
//! Cron times are UTC only (DECISIONS P2.6).

use serde_json::Value;

use crate::enqueue::{BoxFuture, JobsError};
use crate::store::JobStore;

/// `NEXT_PUBLIC_EDITION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    Community,
    Enterprise,
    Cloud,
}

/// `PURGE_WORKSPACES_INTERVAL_MINUTES` default.
pub const PURGE_WORKSPACES_INTERVAL_MINUTES: u32 = 30;
/// `PURGE_BROADCASTS_INTERVAL_MINUTES` default.
pub const PURGE_BROADCASTS_INTERVAL_MINUTES: u32 = 5;
/// `QUOTA_SYNC_INTERVAL_SECONDS` default.
pub const QUOTA_SYNC_INTERVAL_SECONDS: u64 = 60;

/// Scheduler ids that only the cloud edition registers.
pub const CLOUD_ONLY_SCHEDULERS: [&str; 3] = [
    "syncUserQuota",
    "reconcileTenants",
    "unsubscribeExpiredTrials",
];

/// `{ pattern }` or `{ every }` of `upsertJobScheduler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    /// Cron, 5 fields or 6 fields (seconds first), UTC.
    Pattern(String),
    /// `every` in ms.
    EveryMs(u64),
}

impl Repeat {
    /// The cron pattern, if this is one.
    pub fn pattern(&self) -> Option<&str> {
        todo!()
    }
}

/// The job template `{ name, data }` (`data` = `{ type, data }`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleTemplate {
    pub name: String,
    pub data: Value,
}

/// One scheduler of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDef {
    /// Scheduler id (= job name except `refreshShortLivedChannelTokens`).
    pub id: String,
    pub repeat: Repeat,
    pub template: ScheduleTemplate,
}

/// Inputs of the scheduler table (env values and the boot random minute).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleConfig {
    pub edition: Edition,
    pub quota_sync_interval_seconds: u64,
    pub purge_workspaces_interval_minutes: u32,
    pub purge_broadcasts_interval_minutes: u32,
    /// `LICENSE_REFRESH_MINUTE` (0-59, random per boot).
    pub license_refresh_minute: u32,
}

/// `randomInt(60)` from an injected `rand() -> [0, 1)`.
pub fn license_refresh_minute(rand: &dyn Fn() -> f64) -> u32 {
    todo!()
}

/// Every scheduler the edition registers, in registration order.
pub fn schedules(config: &ScheduleConfig) -> Vec<ScheduleDef> {
    todo!()
}

/// Scheduler ids the edition must remove (persisted from another edition).
pub fn removed_schedulers(edition: Edition) -> Vec<&'static str> {
    todo!()
}

/// `scheduleQueue.upsertJobScheduler` / `removeJobScheduler`.
pub trait SchedulerStore: Send + Sync {
    fn upsert_job_scheduler<'a>(
        &'a self,
        id: &'a str,
        repeat: &'a Repeat,
        template: &'a ScheduleTemplate,
    ) -> BoxFuture<'a, Result<(), JobsError>>;

    fn remove_job_scheduler<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, JobsError>>;
}

impl SchedulerStore for JobStore {
    fn upsert_job_scheduler<'a>(
        &'a self,
        id: &'a str,
        repeat: &'a Repeat,
        template: &'a ScheduleTemplate,
    ) -> BoxFuture<'a, Result<(), JobsError>> {
        todo!()
    }

    fn remove_job_scheduler<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, JobsError>> {
        todo!()
    }
}

/// `registerSchedules()`: upserts [`schedules`] and removes
/// [`removed_schedulers`] (S13 edition gating).
pub async fn register_schedules(
    config: &ScheduleConfig,
    store: &dyn SchedulerStore,
) -> Result<(), JobsError> {
    todo!()
}

/// What the schedule worker start needs from the runtime.
pub trait ScheduleWorkerHost: Send + Sync {
    /// `createQueueWorker(schedule, ...)`.
    fn create_schedule_worker(&self) -> Result<(), JobsError>;
    /// `requestShutdown(reason)`.
    fn request_shutdown(&self, reason: &str);
}

/// `schedule/worker.ts` start: registers the schedulers; a registration
/// error is logged ("Error registering schedules", field `err`) and the
/// worker still starts; no shutdown is requested.
pub async fn start_schedule_worker(
    config: &ScheduleConfig,
    store: &dyn SchedulerStore,
    host: &dyn ScheduleWorkerHost,
) -> Result<(), JobsError> {
    todo!()
}

/// A parsed cron pattern: `*`, `N` or `*/N` per field; 5 fields or 6
/// (seconds first); evaluated in UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronPattern {
    _private: (),
}

impl CronPattern {
    pub fn parse(pattern: &str) -> Result<CronPattern, JobsError> {
        todo!()
    }

    /// The first fire time strictly after `after_ms` (UTC).
    pub fn next_after(&self, after_ms: i64) -> i64 {
        todo!()
    }
}
