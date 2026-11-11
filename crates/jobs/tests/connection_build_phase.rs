//! Port of `packages/worker-config/__tests__/connection-build-phase.test.ts` (spec 5.1). TS-only (ioredis
//! `lazyConnect` during `next build`) → proposal: `Jobs::new` does no I/O
//! (no file) until `Jobs::start`, which opens and migrates the jobs tables.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use chatbotx_db::rusqlite::Connection;
use chatbotx_jobs::config::{Jobs, JobsConfig};
use support::TestClock;

const DEAD_REDIS_URL: &str = "redis://127.0.0.1:6399";

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A fresh, not yet existing database path under the temp dir.
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

/// Port of: packages/worker-config/__tests__/connection-build-phase.test.ts:22 "getRedisConnection during next build > does not dial Redis when NEXT_PHASE is phase-production-build"
#[test]
fn does_not_dial_redis_when_next_phase_is_phase_production() {
    let path = fresh_db_path("build-phase");
    let config = JobsConfig::from_env_map(&env(&[
        ("SKIP_ENV_CHECK", "true"),
        ("REDIS_URL", DEAD_REDIS_URL),
        ("VITEST", ""),
        ("NEXT_PHASE", "phase-production-build"),
    ]))
    .unwrap()
    .with_database_path(&path);

    let jobs = Jobs::new(config, TestClock::new(0));

    assert!(!jobs.is_open());
    assert!(!path.exists(), "no database file before the first use");
}

/// Port of: packages/worker-config/__tests__/connection-build-phase.test.ts:34 "getRedisConnection during next build > connects eagerly outside the build phase"
#[tokio::test]
async fn connects_eagerly_outside_the_build_phase() {
    let path = fresh_db_path("eager");
    let config = JobsConfig::from_env_map(&env(&[
        ("SKIP_ENV_CHECK", "true"),
        ("REDIS_URL", DEAD_REDIS_URL),
        ("VITEST", ""),
        ("NEXT_PHASE", ""),
    ]))
    .unwrap()
    .with_database_path(&path);

    let jobs = Jobs::new(config, TestClock::new(0));
    jobs.start().await.unwrap();

    assert!(jobs.is_open());
    let conn = Connection::open(&path).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'jobs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 1);
}
