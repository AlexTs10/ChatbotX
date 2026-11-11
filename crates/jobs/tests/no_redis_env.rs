//! Port of `packages/worker-config/__tests__/no-redis-env.test.ts` (spec 5.1). TS-only (`isNoRedisEnv`:
//! fake queues under vitest / `next build`) → proposals: an in-memory
//! `Jobs` needs no file and no env, `NEXT_PHASE` / `VITEST` are ignored,
//! the default storage is the SQLite file, and an enqueuer does no I/O until
//! it is used.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use chatbotx_jobs::config::{Jobs, JobsConfig, JobsStorage};
use chatbotx_jobs::{JobInspector, JobOptions, QueueName};
use serde_json::json;
use support::TestClock;

const DEAD_REDIS_URL: &str = "redis://127.0.0.1:6399";

/// The `beforeEach` env plus `extra`.
fn env(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> =
        [("SKIP_ENV_CHECK", "true"), ("REDIS_URL", DEAD_REDIS_URL)]
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
    for (k, v) in extra {
        env.insert((*k).to_owned(), (*v).to_owned());
    }
    env
}

fn fresh_db_path(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-jobs-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let _ = std::fs::remove_file(&path);
    path
}

/// The queue barrel works with no server: an add lands as a job row.
async fn add_lands_without_a_server(queue: QueueName) {
    let jobs = Jobs::in_memory().await.unwrap();
    let enqueuer = jobs.enqueuer();

    let added = enqueuer
        .add(
            queue,
            "probe",
            json!({ "type": "probe" }),
            JobOptions {
                job_id: Some("probe-1".to_owned()),
                ..JobOptions::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(added.job_id, "probe-1");
    let stored = jobs
        .queue_store(queue)
        .get_job(queue, "probe-1")
        .await
        .unwrap();
    assert!(stored.is_some());
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:44 "isNoRedisEnv > is true under vitest with NEXT_PHASE unset"
#[tokio::test]
async fn is_true_under_vitest_with_next_phase_unset() {
    let jobs = Jobs::in_memory().await.unwrap();

    assert!(jobs.is_open());
    assert_eq!(jobs.database_path(), None);
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:53 "isNoRedisEnv > is true when NEXT_PHASE is phase-production-build"
#[test]
fn is_true_when_next_phase_is_phase_production_build() {
    let build = JobsConfig::from_env_map(&env(&[
        ("NEXT_PHASE", "phase-production-build"),
        ("VITEST", ""),
    ]))
    .unwrap();
    let plain = JobsConfig::from_env_map(&env(&[("NEXT_PHASE", ""), ("VITEST", "")])).unwrap();

    assert_eq!(build, plain, "NEXT_PHASE has no effect");
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:62 "isNoRedisEnv > is false when neither condition holds"
#[test]
fn is_false_when_neither_condition_holds() {
    let config = JobsConfig::from_env_map(&env(&[("NEXT_PHASE", ""), ("VITEST", "")])).unwrap();

    assert!(
        matches!(config.storage, JobsStorage::File(_)),
        "{:?}",
        config.storage
    );
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:71 "isNoRedisEnv > importing a queue barrel under vitest yields the fake queue, not a BullMQ Queue"
#[tokio::test]
async fn importing_a_queue_barrel_under_vitest_yields_the_fake_queue() {
    add_lands_without_a_server(QueueName::AiAgent).await;
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:80 "isNoRedisEnv > importing the heavy queue barrel under vitest yields the fake queue, not a BullMQ Queue"
#[tokio::test]
async fn importing_the_heavy_queue_barrel_under_vitest_yields_the() {
    add_lands_without_a_server(QueueName::Heavy).await;
}

/// Port of: packages/worker-config/__tests__/no-redis-env.test.ts:89 "isNoRedisEnv > does not dial Redis for the heavy queue when NEXT_PHASE is phase-production-build"
#[test]
fn does_not_dial_redis_for_the_heavy_queue_when_next_phase_is() {
    let path = fresh_db_path("no-dial");
    let config = JobsConfig::from_env_map(&env(&[
        ("VITEST", ""),
        ("NEXT_PHASE", "phase-production-build"),
    ]))
    .unwrap()
    .with_database_path(&path);
    let jobs = Jobs::new(config, TestClock::new(0));

    let _heavy = jobs.enqueuer();

    assert!(!jobs.is_open());
    assert!(!path.exists(), "getting the enqueuer opens no file");
}
