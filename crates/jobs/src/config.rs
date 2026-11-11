//! Jobs configuration and the process handle (spec 1.1, 6.2).
//!
//! The Redis connection groups (`hot` / `bulk`, `REDIS_*` env) have no
//! meaning with one SQLite file: the env is ignored, both groups share one
//! [`JobStore`], and each group keeps its own worker pool (isolation of the
//! original groups). Nothing touches the database file before
//! [`Jobs::start`] (the TypeScript `lazyConnect` during `next build`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::clock::Clock;
use crate::enqueue::{Enqueuer, JobsError};
use crate::queue::{QueueGroup, QueueName};
use crate::store::JobStore;

/// Where the jobs tables live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobsStorage {
    /// The application SQLite file.
    File(PathBuf),
    /// A private in-memory database (tests).
    Memory,
}

/// Configuration of the jobs layer, read from the process env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobsConfig {
    pub storage: JobsStorage,
}

impl JobsConfig {
    /// Reads the config from `std::env`.
    pub fn from_env() -> Result<JobsConfig, JobsError> {
        todo!()
    }

    /// Reads the config from an env map. `REDIS_URL`, `REDIS_QUEUE_URL`,
    /// `REDIS_QUEUE_BULK_URL`, `NEXT_PHASE` and `VITEST` are ignored.
    /// The default storage is the application SQLite file.
    pub fn from_env_map(env: &BTreeMap<String, String>) -> Result<JobsConfig, JobsError> {
        todo!()
    }

    /// The same config with the database file at `path`.
    pub fn with_database_path(self, path: impl Into<PathBuf>) -> JobsConfig {
        todo!()
    }
}

/// The worker pool of one queue group (semaphores + dispatchers of the
/// queues of that group).
pub struct WorkerPool {
    group: QueueGroup,
}

impl WorkerPool {
    pub fn group(&self) -> QueueGroup {
        self.group
    }
}

/// The jobs layer of this process.
pub struct Jobs {
    config: JobsConfig,
    clock: Arc<dyn Clock>,
}

impl Jobs {
    /// Builds the handle. Does no I/O: the database file is not opened or
    /// created until [`Jobs::start`] (or the first enqueue).
    pub fn new(config: JobsConfig, clock: Arc<dyn Clock>) -> Jobs {
        todo!()
    }

    /// A started handle on a private in-memory database: no file, no env.
    pub async fn in_memory() -> Result<Jobs, JobsError> {
        todo!()
    }

    /// Opens the database and creates the jobs tables (boot).
    pub async fn start(&self) -> Result<(), JobsError> {
        todo!()
    }

    /// Whether the database was opened.
    pub fn is_open(&self) -> bool {
        todo!()
    }

    /// The database file, `None` for an in-memory handle.
    pub fn database_path(&self) -> Option<&Path> {
        todo!()
    }

    /// The store that holds the jobs of `group` (the same store for both
    /// groups).
    pub fn store_for(&self, group: QueueGroup) -> Arc<JobStore> {
        todo!()
    }

    /// The store of `queue` (`getQueueConnection`).
    pub fn queue_store(&self, queue: QueueName) -> Arc<JobStore> {
        todo!()
    }

    /// The worker pool of `group` (`getRedisConnection(group)`: one cached
    /// instance per group).
    pub fn pool(&self, group: QueueGroup) -> Arc<WorkerPool> {
        todo!()
    }

    /// The worker pool of the group of `queue`.
    pub fn queue_pool(&self, queue: QueueName) -> Arc<WorkerPool> {
        todo!()
    }

    /// The producer handle. Getting it does no I/O.
    pub fn enqueuer(&self) -> Arc<dyn Enqueuer> {
        todo!()
    }
}
