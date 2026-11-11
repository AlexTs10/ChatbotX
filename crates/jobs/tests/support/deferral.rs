//! Fakes for the C2 ports (deferral, schedules, chat wait). Test code.

#![allow(dead_code)]

use std::sync::Mutex;

use chatbotx_jobs::schedules::{Repeat, ScheduleTemplate, SchedulerStore};
use chatbotx_jobs::{BoxFuture, JobWaiter, JobsError, QueueName, WaitError};
use serde_json::Value;

/// One recorded `upsertJobScheduler(id, repeat, template)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Upsert {
    pub id: String,
    pub repeat: Repeat,
    pub template: ScheduleTemplate,
}

/// Records upserts and removes (replaces `mockUpsertJobScheduler` /
/// `mockRemoveJobScheduler`); can fail every upsert.
#[derive(Debug, Default)]
pub struct FakeSchedulerStore {
    pub upserts: Mutex<Vec<Upsert>>,
    pub removes: Mutex<Vec<String>>,
    pub fail_upserts: Mutex<Option<JobsError>>,
}

impl FakeSchedulerStore {
    pub fn upserted_names(&self) -> Vec<String> {
        self.upserts
            .lock()
            .unwrap()
            .iter()
            .map(|u| u.id.clone())
            .collect()
    }

    pub fn upsert_for(&self, id: &str) -> Option<Upsert> {
        self.upserts
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.id == id)
            .cloned()
    }

    pub fn pattern_for(&self, id: &str) -> Option<String> {
        self.upsert_for(id).and_then(|u| match u.repeat {
            Repeat::Pattern(p) => Some(p),
            Repeat::EveryMs(_) => None,
        })
    }

    pub fn removed(&self) -> Vec<String> {
        self.removes.lock().unwrap().clone()
    }
}

impl SchedulerStore for FakeSchedulerStore {
    fn upsert_job_scheduler<'a>(
        &'a self,
        id: &'a str,
        repeat: &'a Repeat,
        template: &'a ScheduleTemplate,
    ) -> BoxFuture<'a, Result<(), JobsError>> {
        Box::pin(async move {
            if let Some(e) = self.fail_upserts.lock().unwrap().clone() {
                return Err(e);
            }
            self.upserts.lock().unwrap().push(Upsert {
                id: id.to_owned(),
                repeat: repeat.clone(),
                template: template.clone(),
            });
            Ok(())
        })
    }

    fn remove_job_scheduler<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            self.removes.lock().unwrap().push(id.to_owned());
            Ok(true)
        })
    }
}

/// One recorded `waitUntilFinished(queueEvents, ttl)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitCall {
    pub queue: QueueName,
    pub job_id: String,
    pub timeout_ms: u64,
}

/// A `JobWaiter` that records each wait and returns a fixed outcome.
#[derive(Debug)]
pub struct FakeWaiter {
    pub calls: Mutex<Vec<WaitCall>>,
    pub outcome: Result<Value, WaitError>,
}

impl FakeWaiter {
    pub fn resolving(value: Value) -> FakeWaiter {
        FakeWaiter {
            calls: Mutex::new(Vec::new()),
            outcome: Ok(value),
        }
    }

    pub fn rejecting(error: WaitError) -> FakeWaiter {
        FakeWaiter {
            calls: Mutex::new(Vec::new()),
            outcome: Err(error),
        }
    }

    pub fn calls(&self) -> Vec<WaitCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl JobWaiter for FakeWaiter {
    fn wait_until_finished<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(WaitCall {
                queue,
                job_id: job_id.to_owned(),
                timeout_ms,
            });
            self.outcome.clone()
        })
    }
}
