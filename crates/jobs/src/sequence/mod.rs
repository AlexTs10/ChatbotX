//! Sequence scheduler (spec 3): the dispatch schedule, the business
//! functions of `packages/sequence-scheduler`, and the producer, consumer,
//! reconcile and retry parts of `apps/worker/src/sequence-scheduler`.
//! The business functions need the P2.4 sequence tables.

pub mod advance;
pub mod buckets;
pub mod cancel;
pub mod consumer;
pub mod contacts;
pub mod dispatch;
pub mod enroll;
pub mod next_run;
pub mod producer;
pub mod reconcile;
pub mod repo;
pub mod retry;
pub mod schedule;
pub mod send_time;
pub mod sqlite_schedule;
pub mod tick;
