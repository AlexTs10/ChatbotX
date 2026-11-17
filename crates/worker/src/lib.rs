//! `apps/worker` core (Q.W1, `conversion/specs/Q.W1.md`): the `chat`
//! queue, the flow runner of the `integration` queue (`sendFlow`), the
//! `default` queue, and the channel send interface ([`channel::Channel`])
//! the channel integrations implement.
//!
//! Handlers run on the `chatbotx-jobs` runtime inside `chatbotx-server`
//! ([`start`]).

pub mod channel;
pub mod channels;
pub mod chat;
pub mod default_queue;
pub mod flow;
pub mod integration;
pub mod meta_catalog;
pub mod schedule;
pub mod send_flow_step;
pub mod sequence;
pub mod variables;

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::runtime::{Handler, Runtime, WorkerSpec};
use chatbotx_jobs::{Enqueuer, JobCtx, JobError, JobWaiter, JobsError, QueueName};
use chatbotx_rt::Realtime;
use serde_json::Value;

pub use channel::{Channel, Channels};

/// What the handlers share (the TypeScript module singletons: `db`, the
/// queues, `QueueEvents`, the realtime producer, the integration registry).
#[derive(Clone)]
pub struct Deps {
    pub db: Arc<Db>,
    pub jobs: Arc<dyn Enqueuer>,
    pub waiter: Arc<dyn JobWaiter>,
    pub realtime: Option<Realtime>,
    pub channels: Channels,
    /// `NEXT_PUBLIC_BUILDER_URL` without a trailing `/`.
    pub app_url: String,
    /// `{{variable}}` resolution; `None` = spintax only.
    pub variables: Option<Arc<variables::VariableDeps>>,
}

/// A handler failure that BullMQ would retry (`throw new Error(..)`).
pub(crate) fn fail(message: impl Into<String>) -> JobError {
    JobError::retryable(message)
}

/// A repo error inside a `Db` closure (the closure error is `DbError`).
pub(crate) fn repo_err(error: chatbotx_db::repo::Error) -> chatbotx_db::DbError {
    match error {
        chatbotx_db::repo::Error::Db(error) => error,
        other => chatbotx_db::DbError::other(other.message()),
    }
}

pub(crate) fn db_fail(error: impl std::fmt::Display) -> JobError {
    JobError::retryable(error.to_string())
}

/// An id that the job carries as a string, a number or an `{ id }` object.
pub(crate) fn id_of(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        Value::Object(map) => map.get("id").and_then(id_of),
        _ => None,
    }
}

/// An ISO date string (or unix ms number) as unix ms.
pub(crate) fn ms_of(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => chatbotx_core::js::parse_date(s),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

pub(crate) fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `conversationModel` by id (any workspace; the job came from inside the
/// app).
pub(crate) async fn load_conversation(db: &Db, id: i64) -> Result<Option<Conversation>, JobError> {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                &format!(r#"{} WHERE "id" = ?1"#, Conversation::select_sql()),
                params![id],
                Conversation::from_row,
            )
            .optional()?)
    })
    .await
    .map_err(db_fail)
}

/// `contactInboxModel` by id and contact.
pub(crate) async fn load_contact_inbox(
    db: &Db,
    id: i64,
    contact_id: i64,
) -> Result<Option<ContactInbox>, JobError> {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                &format!(
                    r#"{} WHERE "id" = ?1 AND "contactId" = ?2"#,
                    ContactInbox::select_sql()
                ),
                params![id, contact_id],
                ContactInbox::from_row,
            )
            .optional()?)
    })
    .await
    .map_err(db_fail)
}

/// `env.<NAME>` concurrency: an integer in `1..=max`, else the default.
pub fn concurrency_env(name: &str, default: u32, max: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|v| (1..=max).contains(v))
        .unwrap_or(default)
}

type QueueHandler = fn(Deps, JobCtx) -> chatbotx_jobs::BoxFuture<'static, Result<Value, JobError>>;

fn handler(deps: &Deps, run: QueueHandler) -> Handler {
    let deps = deps.clone();
    Arc::new(move |job| run(deps.clone(), job))
}

/// Starts the `chat`, `integration`, `default` and `schedule` workers (`runWorker` +
/// `createQueueWorker`) with the original concurrency envs.
pub fn start(runtime: &Runtime, deps: &Deps) -> Result<(), JobsError> {
    let specs = chatbotx_jobs::runtime::worker_specs();
    let spec_of = |queue: QueueName| {
        specs
            .iter()
            .find(|s| s.queue == queue)
            .cloned()
            .unwrap_or_else(|| WorkerSpec::new(queue))
    };
    runtime.spawn_worker(
        spec_of(QueueName::Chat).concurrency(concurrency_env("CHAT_WORKER_CONCURRENCY", 5, 200)),
        handler(deps, |deps, job| Box::pin(chat::handle(deps, job))),
    )?;
    runtime.spawn_worker(
        spec_of(QueueName::Integration).concurrency(concurrency_env(
            "INTEGRATION_WORKER_CONCURRENCY",
            10,
            200,
        )),
        handler(deps, |deps, job| Box::pin(integration::handle(deps, job))),
    )?;
    runtime.spawn_worker(
        spec_of(QueueName::Default),
        handler(deps, |deps, job| Box::pin(default_queue::handle(deps, job))),
    )?;
    runtime.spawn_worker(
        spec_of(QueueName::Schedule),
        handler(deps, |deps, job| Box::pin(schedule::handle(deps, job))),
    )?;
    Ok(())
}
