//! Port of `packages/worker-config/__tests__/connection-queue-groups.test.ts` (spec 5.1). TS-only (Redis URL
//! routing per queue group) → proposals: the `REDIS_*` env is ignored, both
//! groups share one `JobStore`, each group keeps its own cached worker pool,
//! and `queue_group` keeps the documented group as metadata.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_jobs::config::{Jobs, JobsConfig, WorkerPool};
use chatbotx_jobs::{QueueGroup, QueueName, queue_group};

const DEAD_REDIS_URL: &str = "redis://127.0.0.1:6399";
const DEAD_QUEUE_URL: &str = "redis://127.0.0.1:6398";
const DEAD_BULK_URL: &str = "redis://127.0.0.1:6397";

/// The `beforeEach` env plus `extra`.
fn env(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = [
        ("SKIP_ENV_CHECK", "true"),
        ("NEXT_PHASE", "phase-production-build"),
        ("VITEST", "true"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    for (k, v) in extra {
        env.insert((*k).to_owned(), (*v).to_owned());
    }
    env
}

fn all_urls() -> BTreeMap<String, String> {
    env(&[
        ("REDIS_URL", DEAD_REDIS_URL),
        ("REDIS_QUEUE_URL", DEAD_QUEUE_URL),
        ("REDIS_QUEUE_BULK_URL", DEAD_BULK_URL),
    ])
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:21 "getRedisConnection queue-group routing > hot uses REDIS_QUEUE_URL and ignores REDIS_QUEUE_BULK_URL"
#[test]
fn hot_uses_redis_queue_url_and_ignores_redis_queue_bulk_url() {
    let with_urls = JobsConfig::from_env_map(&all_urls()).unwrap();
    let without = JobsConfig::from_env_map(&env(&[])).unwrap();

    assert_eq!(with_urls, without, "REDIS_* env has no effect");
    assert_eq!(queue_group(QueueName::Integration), QueueGroup::Hot);
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:35 "getRedisConnection queue-group routing > bulk uses REDIS_QUEUE_BULK_URL when set"
#[test]
fn bulk_uses_redis_queue_bulk_url_when_set() {
    JobsConfig::from_env_map(&all_urls()).unwrap();

    assert_eq!(queue_group(QueueName::Heavy), QueueGroup::Bulk);
    assert_eq!(queue_group(QueueName::Template), QueueGroup::Bulk);
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:49 "getRedisConnection queue-group routing > bulk falls back to the hot chain when REDIS_QUEUE_BULK_URL is unset"
#[tokio::test]
async fn bulk_falls_back_to_the_hot_chain_when_redis_queue_bulk_url() {
    JobsConfig::from_env_map(&env(&[
        ("REDIS_URL", DEAD_REDIS_URL),
        ("REDIS_QUEUE_URL", DEAD_QUEUE_URL),
    ]))
    .unwrap();
    let jobs = Jobs::in_memory().await.unwrap();

    assert!(Arc::ptr_eq(
        &jobs.store_for(QueueGroup::Bulk),
        &jobs.store_for(QueueGroup::Hot)
    ));
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:64 "getRedisConnection queue-group routing > hot and bulk both fall back to REDIS_URL when queue URLs are unset"
#[tokio::test]
async fn hot_and_bulk_both_fall_back_to_redis_url_when_queue_urls_are() {
    JobsConfig::from_env_map(&env(&[("REDIS_URL", DEAD_REDIS_URL)])).unwrap();
    let jobs = Jobs::in_memory().await.unwrap();

    let hot = jobs.store_for(QueueGroup::Hot);
    let bulk = jobs.store_for(QueueGroup::Bulk);
    assert!(Arc::ptr_eq(&hot, &bulk), "one store for every queue");
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:80 "getRedisConnection queue-group routing > hot and bulk are cached as distinct connection instances"
#[tokio::test]
async fn hot_and_bulk_are_cached_as_distinct_connection_instances() {
    JobsConfig::from_env_map(&all_urls()).unwrap();
    let jobs = Jobs::in_memory().await.unwrap();

    let hot_first = jobs.pool(QueueGroup::Hot);
    let hot_second = jobs.pool(QueueGroup::Hot);
    let bulk = jobs.pool(QueueGroup::Bulk);

    assert!(Arc::ptr_eq(&hot_second, &hot_first));
    assert!(!Arc::ptr_eq(&bulk, &hot_first));
    assert_eq!(hot_first.group(), QueueGroup::Hot);
    assert_eq!(bulk.group(), QueueGroup::Bulk);
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:97 "getRedisConnection queue-group routing > requires an explicit queue group"
#[tokio::test]
async fn requires_an_explicit_queue_group() {
    // `getRedisConnection.length === 1`: the group is the one required argument.
    let pool: fn(&Jobs, QueueGroup) -> Arc<WorkerPool> = Jobs::pool;
    let jobs = Jobs::in_memory().await.unwrap();

    assert_eq!(pool(&jobs, QueueGroup::Hot).group(), QueueGroup::Hot);
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:107 "getRedisConnection queue-group routing > returns the singleton assigned to each queue's group"
#[tokio::test]
async fn returns_the_singleton_assigned_to_each_queue_s_group() {
    JobsConfig::from_env_map(&all_urls()).unwrap();
    let jobs = Jobs::in_memory().await.unwrap();
    let hot = jobs.pool(QueueGroup::Hot);
    let bulk = jobs.pool(QueueGroup::Bulk);

    assert_eq!(queue_group(QueueName::Chat), QueueGroup::Hot);
    assert_eq!(queue_group(QueueName::Quota), QueueGroup::Bulk);
    assert!(Arc::ptr_eq(&jobs.queue_pool(QueueName::Chat), &hot));
    assert!(Arc::ptr_eq(&jobs.queue_pool(QueueName::Quota), &bulk));
}

/// Port of: packages/worker-config/__tests__/connection-queue-groups.test.ts:126 "getRedisConnection queue-group routing > assigns every queue to its documented Redis group"
#[test]
fn assigns_every_queue_to_its_documented_redis_group() {
    use QueueGroup::{Bulk, Hot};
    let expected: BTreeMap<&str, QueueGroup> = [
        ("integration", Hot),
        ("chat", Hot),
        ("aiAgent", Bulk),
        ("heavy", Bulk),
        ("schedule", Bulk),
        ("trigger", Bulk),
        ("webhook", Bulk),
        ("default", Bulk),
        ("sequenceScheduler", Hot),
        ("broadcast", Hot),
        ("quota", Bulk),
        ("notification", Hot),
        ("callTranscription", Hot),
        ("whatsappVoipSignaling", Hot),
        ("low", Hot),
        ("profileSnapshot", Bulk),
        ("leaderboard", Bulk),
        ("marketingMessage", Bulk),
        ("template", Bulk),
    ]
    .into_iter()
    .collect();

    let actual: BTreeMap<&str, QueueGroup> = QueueName::ALL
        .iter()
        .map(|q| (q.as_str(), queue_group(*q)))
        .collect();

    assert_eq!(actual, expected);
}
