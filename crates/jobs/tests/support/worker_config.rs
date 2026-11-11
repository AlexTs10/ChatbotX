//! Fakes for the worker-config ports (job-wait, template-queue).
//! Include with `#[path = "support/worker_config.rs"] mod worker_config;`.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobInspector, JobOptions, JobRecord, JobState, JobWaiter,
    JobsError, NewJob, QueueName, WaitError,
};
use serde_json::Value;

/// A `JobRecord` with only the attempt fields set (the TS `getJob` mock).
pub fn attempts_record(attempts_made: u32, attempts: u32) -> JobRecord {
    JobRecord {
        queue: QueueName::Heavy,
        job_id: "job-1".to_owned(),
        name: String::new(),
        data: Value::Null,
        state: JobState::Failed,
        priority: 0,
        attempts_made,
        attempts_started: attempts_made,
        opts: JobOptions {
            attempts: Some(attempts),
            ..JobOptions::default()
        },
        return_value: None,
        failed_reason: None,
    }
}

// ---------------------------------------------------------------------------
// FakeWaiter: scripted `waitUntilFinished`
// ---------------------------------------------------------------------------

/// One recorded wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitCall {
    pub queue: QueueName,
    pub job_id: String,
    pub timeout_ms: u64,
}

type Hook = Box<dyn Fn(usize) + Send + Sync>;

/// Returns the scripted results in order (the last one repeats) and runs
/// `hook(call_index)` before answering a call.
pub struct FakeWaiter {
    results: Mutex<VecDeque<Result<Value, WaitError>>>,
    last: Mutex<Option<Result<Value, WaitError>>>,
    calls: Mutex<Vec<WaitCall>>,
    hook: Option<Hook>,
}

impl FakeWaiter {
    pub fn new(results: Vec<Result<Value, WaitError>>) -> FakeWaiter {
        FakeWaiter {
            results: Mutex::new(results.into()),
            last: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
            hook: None,
        }
    }

    pub fn with_hook(mut self, hook: impl Fn(usize) + Send + Sync + 'static) -> FakeWaiter {
        self.hook = Some(Box::new(hook));
        self
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
            let index = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(WaitCall {
                    queue,
                    job_id: job_id.to_owned(),
                    timeout_ms,
                });
                calls.len() - 1
            };
            if let Some(hook) = &self.hook {
                hook(index);
            }
            let next = self.results.lock().unwrap().pop_front();
            match next {
                Some(result) => {
                    *self.last.lock().unwrap() = Some(result.clone());
                    result
                }
                None => self
                    .last
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or(Err(WaitError::Timeout)),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// FakeInspector: scripted `getJob`
// ---------------------------------------------------------------------------

pub struct FakeInspector {
    result: Result<Option<JobRecord>, JobsError>,
    get_job_calls: Mutex<usize>,
}

impl FakeInspector {
    pub fn new(result: Result<Option<JobRecord>, JobsError>) -> FakeInspector {
        FakeInspector {
            result,
            get_job_calls: Mutex::new(0),
        }
    }

    pub fn get_job_calls(&self) -> usize {
        *self.get_job_calls.lock().unwrap()
    }
}

impl JobInspector for FakeInspector {
    fn get_job<'a>(
        &'a self,
        _queue: QueueName,
        _job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>> {
        Box::pin(async move {
            *self.get_job_calls.lock().unwrap() += 1;
            self.result.clone()
        })
    }

    fn get_state<'a>(
        &'a self,
        _queue: QueueName,
        _job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>> {
        Box::pin(async move {
            match &self.result {
                Ok(Some(job)) => Ok(job.state),
                Ok(None) => Ok(JobState::Unknown),
                Err(e) => Err(e.clone()),
            }
        })
    }

    fn change_priority<'a>(
        &'a self,
        _queue: QueueName,
        _job_id: &'a str,
        _priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move { Ok(false) })
    }

    fn remove<'a>(
        &'a self,
        _queue: QueueName,
        _job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move { Ok(false) })
    }
}

// ---------------------------------------------------------------------------
// FakeTemplateQueue: BullMQ-like job map (the template-queue.test.ts mock)
// ---------------------------------------------------------------------------

/// One recorded `add`.
#[derive(Debug, Clone, PartialEq)]
pub struct AddCall {
    pub queue: QueueName,
    pub name: String,
    pub data: Value,
    pub opts: JobOptions,
}

/// What `change_priority` does for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeBehavior {
    /// Sets the stored priority.
    Normal,
    /// The job becomes active, then the change fails with this message.
    ActivateThenFail(String),
    /// The change fails with this message (the job still waits).
    Fail(String),
}

#[derive(Debug, Clone)]
pub struct StoredJob {
    pub name: String,
    pub data: Value,
    /// The stored priority (BullMQ rebuilds it on `getJob`).
    pub priority: u32,
    /// What `add` was given.
    pub opts: JobOptions,
    pub state: JobState,
    pub change_priority_calls: usize,
    pub change_behavior: ChangeBehavior,
}

/// Every `add` per queue, and jobs by id deduped like BullMQ: a second
/// `add` with a known id changes nothing. A new job is `Prioritized`.
#[derive(Default)]
pub struct FakeTemplateQueue {
    added: Mutex<Vec<AddCall>>,
    jobs: Mutex<BTreeMap<String, StoredJob>>,
}

impl FakeTemplateQueue {
    pub fn new() -> FakeTemplateQueue {
        FakeTemplateQueue::default()
    }

    pub fn calls_on(&self, queue: QueueName) -> Vec<AddCall> {
        self.added
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.queue == queue)
            .cloned()
            .collect()
    }

    pub fn job(&self, job_id: &str) -> Option<StoredJob> {
        self.jobs.lock().unwrap().get(job_id).cloned()
    }

    pub fn priority_of(&self, job_id: &str) -> Option<u32> {
        self.job(job_id).map(|j| j.priority)
    }

    pub fn set_state(&self, job_id: &str, state: JobState) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.state = state;
        }
    }

    pub fn set_change_behavior(&self, job_id: &str, behavior: ChangeBehavior) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.change_behavior = behavior;
        }
    }

    pub fn clear_change_calls(&self, job_id: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.change_priority_calls = 0;
        }
    }

    fn record(&self, queue: QueueName, name: &str, data: Value, opts: JobOptions) -> AddResult {
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.added.lock().unwrap().push(AddCall {
            queue,
            name: name.to_owned(),
            data: data.clone(),
            opts: opts.clone(),
        });
        let mut jobs = self.jobs.lock().unwrap();
        let created = !jobs.contains_key(&job_id);
        if created {
            jobs.insert(
                job_id.clone(),
                StoredJob {
                    name: name.to_owned(),
                    data,
                    priority: opts.priority.unwrap_or(0),
                    opts,
                    state: JobState::Prioritized,
                    change_priority_calls: 0,
                    change_behavior: ChangeBehavior::Normal,
                },
            );
        }
        AddResult { job_id, created }
    }
}

impl Enqueuer for FakeTemplateQueue {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let result = self.record(queue, name, data, opts);
        Box::pin(async move { Ok(result) })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let results = jobs
            .into_iter()
            .map(|j| self.record(queue, &j.name, j.data, j.opts))
            .collect();
        Box::pin(async move { Ok(results) })
    }
}

impl JobInspector for FakeTemplateQueue {
    fn get_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>> {
        Box::pin(async move {
            Ok(self.job(job_id).map(|j| JobRecord {
                queue,
                job_id: job_id.to_owned(),
                name: j.name,
                data: j.data,
                state: j.state,
                priority: j.priority,
                attempts_made: 0,
                attempts_started: 0,
                opts: j.opts,
                return_value: None,
                failed_reason: None,
            }))
        })
    }

    fn get_state<'a>(
        &'a self,
        _queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>> {
        Box::pin(async move { Ok(self.job(job_id).map_or(JobState::Unknown, |j| j.state)) })
    }

    fn change_priority<'a>(
        &'a self,
        _queue: QueueName,
        job_id: &'a str,
        priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(job) = jobs.get_mut(job_id) else {
                return Ok(false);
            };
            job.change_priority_calls += 1;
            match job.change_behavior.clone() {
                ChangeBehavior::Normal => {
                    job.priority = priority;
                    Ok(true)
                }
                ChangeBehavior::ActivateThenFail(message) => {
                    job.state = JobState::Active;
                    Err(JobsError::Store(message))
                }
                ChangeBehavior::Fail(message) => Err(JobsError::Store(message)),
            }
        })
    }

    fn remove<'a>(
        &'a self,
        _queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move { Ok(self.jobs.lock().unwrap().remove(job_id).is_some()) })
    }
}
