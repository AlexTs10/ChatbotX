//! Background jobs (P2.6, `conversion/specs/P2.6.md`).
//!
//! Replaces BullMQ/Redis queues, the job schedulers, the sequence scheduler
//! and the event bus with tables in the one SQLite file (`chatbotx-db`).
//!
//! Each module names the spec sections it implements.

pub mod chat_wait;
pub mod clock;
pub mod config;
pub mod ctx;
mod db_retry;
pub mod deferral;
pub mod enqueue;
pub mod events;
pub mod options;
pub mod queue;
pub mod queues;
pub mod rand;
pub mod runtime;
pub mod schedules;
pub mod sequence;
pub mod shutdown;
pub mod store;
pub mod wait;

pub use clock::{Clock, SystemClock};
pub use ctx::{BoxError, JobCtx, JobError, has_exhausted_attempts, is_final_attempt};
pub use enqueue::{BoxFuture, Enqueuer, JobsError};
pub use options::{AddResult, Backoff, Dedup, JobOptions, NewJob, Retention};
pub use queue::{QueueGroup, QueueName, queue_group};
pub use store::{JobInspector, JobRecord, JobState, JobStore};
pub use wait::{CompletionRegistry, JobEvent, JobWaiter, WaitError, WaitTarget};

/// A fresh file database in the temp dir (WAL, no shared-cache table
/// locks): for unit tests that write and read concurrently.
#[cfg(test)]
pub(crate) fn temp_db() -> std::sync::Arc<chatbotx_db::Db> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-jobs-unit-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("jobs.db");
    let _ = std::fs::remove_file(&path);
    std::sync::Arc::new(chatbotx_db::Db::open(path).expect("open temp db"))
}
