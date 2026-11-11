//! Port of `apps/worker/__tests__/schedule-worker-register-failure.test.ts`
//! (`schedule/worker.ts` start: registration failure is logged, the worker
//! still starts, no shutdown).

mod support;

#[path = "support/deferral.rs"]
mod deferral;

use std::sync::Mutex;

use chatbotx_jobs::JobsError;
use chatbotx_jobs::schedules::{
    Edition, ScheduleConfig, ScheduleWorkerHost, start_schedule_worker,
};
use deferral::FakeSchedulerStore;
use support::RecordingLog;
use tracing::Level;

/// Replaces the `createQueueWorker` and `requestShutdown` mocks.
#[derive(Default)]
struct FakeHost {
    created: Mutex<u32>,
    shutdowns: Mutex<Vec<String>>,
}

impl ScheduleWorkerHost for FakeHost {
    fn create_schedule_worker(&self) -> Result<(), JobsError> {
        *self.created.lock().unwrap() += 1;
        Ok(())
    }

    fn request_shutdown(&self, reason: &str) {
        self.shutdowns.lock().unwrap().push(reason.to_owned());
    }
}

/// Port of: apps/worker/__tests__/schedule-worker-register-failure.test.ts:48 "a registerSchedules rejection still starts the schedule worker without shutting down"
#[tokio::test]
async fn a_registerschedules_rejection_still_starts_the_schedule() {
    let log = RecordingLog::install();
    let store = FakeSchedulerStore::default();
    *store.fail_upserts.lock().unwrap() = Some(JobsError::Store("redis unavailable".to_owned()));
    let host = FakeHost::default();
    let config = ScheduleConfig {
        edition: Edition::Community,
        quota_sync_interval_seconds: 60,
        purge_workspaces_interval_minutes: 30,
        purge_broadcasts_interval_minutes: 5,
        license_refresh_minute: 7,
    };

    let _ = start_schedule_worker(&config, &store, &host).await;
    // `vi.waitFor`: allow a spawned registration to run.
    for _ in 0..100 {
        if log.has(Level::ERROR, "Error registering schedules") {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(*host.created.lock().unwrap(), 1);
    let entry = log.find("Error registering schedules").expect("error log");
    assert_eq!(entry.level, Level::ERROR);
    assert!(entry.field("err").is_some());
    assert!(host.shutdowns.lock().unwrap().is_empty());
}
