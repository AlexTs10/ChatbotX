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

use chatbotx_db::Db;
use tokio::sync::OnceCell;

use crate::clock::{Clock, SystemClock};
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::queue::{QueueGroup, QueueName, queue_group};
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
        JobsConfig::from_env_map(&std::env::vars().collect())
    }

    /// Reads the config from an env map. `REDIS_URL`, `REDIS_QUEUE_URL`,
    /// `REDIS_QUEUE_BULK_URL`, `NEXT_PHASE` and `VITEST` are ignored.
    /// The default storage is the application SQLite file
    /// (`$DATA_DIR/chatbotx.db`, `DATA_DIR` default `./data`).
    pub fn from_env_map(env: &BTreeMap<String, String>) -> Result<JobsConfig, JobsError> {
        let data_dir = env
            .get("DATA_DIR")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("./data");
        Ok(JobsConfig {
            storage: JobsStorage::File(Path::new(data_dir).join(DATABASE_FILE)),
        })
    }

    /// The same config with the database file at `path`.
    pub fn with_database_path(self, path: impl Into<PathBuf>) -> JobsConfig {
        JobsConfig {
            storage: JobsStorage::File(path.into()),
        }
    }
}

/// The application database file name under `DATA_DIR`.
pub const DATABASE_FILE: &str = "chatbotx.db";

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

struct JobsInner {
    config: JobsConfig,
    clock: Arc<dyn Clock>,
    store: OnceCell<Arc<JobStore>>,
}

impl JobsInner {
    async fn started(&self) -> Result<&Arc<JobStore>, JobsError> {
        self.store
            .get_or_try_init(|| async {
                let db = match &self.config.storage {
                    JobsStorage::Memory => Db::open_in_memory()?,
                    JobsStorage::File(path) => {
                        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                            std::fs::create_dir_all(dir).map_err(|e| {
                                JobsError::Store(format!("cannot create {}: {e}", dir.display()))
                            })?;
                        }
                        Db::open(path)?
                    }
                };
                let store = JobStore::open(Arc::new(db), Arc::clone(&self.clock)).await?;
                Ok::<_, JobsError>(Arc::new(store))
            })
            .await
    }
}

/// The jobs layer of this process.
pub struct Jobs {
    inner: Arc<JobsInner>,
    pools: [Arc<WorkerPool>; 2],
}

/// The producer handle: opens the database on first use.
struct LazyEnqueuer {
    inner: Arc<JobsInner>,
}

impl Enqueuer for LazyEnqueuer {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: serde_json::Value,
        opts: crate::options::JobOptions,
    ) -> BoxFuture<'_, Result<crate::options::AddResult, JobsError>> {
        let name = name.to_owned();
        Box::pin(async move {
            let store = self.inner.started().await?;
            store.add(queue, &name, data, opts).await
        })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<crate::options::NewJob>,
    ) -> BoxFuture<'_, Result<Vec<crate::options::AddResult>, JobsError>> {
        Box::pin(async move {
            let store = self.inner.started().await?;
            store.add_bulk(queue, jobs).await
        })
    }
}

impl Jobs {
    /// Builds the handle. Does no I/O: the database file is not opened or
    /// created until [`Jobs::start`] (or the first enqueue).
    pub fn new(config: JobsConfig, clock: Arc<dyn Clock>) -> Jobs {
        Jobs {
            inner: Arc::new(JobsInner {
                config,
                clock,
                store: OnceCell::new(),
            }),
            pools: [
                Arc::new(WorkerPool {
                    group: QueueGroup::Hot,
                }),
                Arc::new(WorkerPool {
                    group: QueueGroup::Bulk,
                }),
            ],
        }
    }

    /// A started handle on a private in-memory database: no file, no env.
    pub async fn in_memory() -> Result<Jobs, JobsError> {
        let jobs = Jobs::new(
            JobsConfig {
                storage: JobsStorage::Memory,
            },
            Arc::new(SystemClock),
        );
        jobs.start().await?;
        Ok(jobs)
    }

    /// Opens the database and creates the jobs tables (boot).
    pub async fn start(&self) -> Result<(), JobsError> {
        self.inner.started().await.map(|_| ())
    }

    /// Whether the database was opened.
    pub fn is_open(&self) -> bool {
        self.inner.store.initialized()
    }

    /// The database file, `None` for an in-memory handle.
    pub fn database_path(&self) -> Option<&Path> {
        match &self.inner.config.storage {
            JobsStorage::File(path) => Some(path),
            JobsStorage::Memory => None,
        }
    }

    /// The store that holds the jobs of `group` (the same store for both
    /// groups). Panics before [`Jobs::start`].
    pub fn store_for(&self, group: QueueGroup) -> Arc<JobStore> {
        let _ = group;
        Arc::clone(
            self.inner
                .store
                .get()
                .expect("Jobs::start must run before the store is used"),
        )
    }

    /// The store of `queue` (`getQueueConnection`).
    pub fn queue_store(&self, queue: QueueName) -> Arc<JobStore> {
        self.store_for(queue_group(queue))
    }

    /// The worker pool of `group` (`getRedisConnection(group)`: one cached
    /// instance per group).
    pub fn pool(&self, group: QueueGroup) -> Arc<WorkerPool> {
        match group {
            QueueGroup::Hot => Arc::clone(&self.pools[0]),
            QueueGroup::Bulk => Arc::clone(&self.pools[1]),
        }
    }

    /// The worker pool of the group of `queue`.
    pub fn queue_pool(&self, queue: QueueName) -> Arc<WorkerPool> {
        self.pool(queue_group(queue))
    }

    /// The producer handle. Getting it does no I/O.
    pub fn enqueuer(&self) -> Arc<dyn Enqueuer> {
        Arc::new(LazyEnqueuer {
            inner: Arc::clone(&self.inner),
        })
    }
}
