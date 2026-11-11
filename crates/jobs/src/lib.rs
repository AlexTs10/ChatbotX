//! Background jobs (P2.6, `conversion/specs/P2.6.md`).
//!
//! Replaces BullMQ/Redis queues, the job schedulers, the sequence scheduler
//! and the event bus with tables in the one SQLite file (`chatbotx-db`).
//!
//! Test-first skeleton: the public API that the ported frozen tests use is
//! declared here, and the bodies are `todo!()`. The implementation follows
//! the spec sections named in each module.

// Skeleton only: parameters of `todo!()` bodies are not read yet.
#![allow(unused_variables, dead_code)]

pub mod chat_wait;
pub mod clock;
pub mod config;
pub mod ctx;
pub mod deferral;
pub mod enqueue;
pub mod events;
pub mod options;
pub mod queue;
pub mod queues;
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
