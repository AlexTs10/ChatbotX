//! Port of `apps/worker/__tests__/meta-catalog-reconciler.test.ts`
//! (Q.P6-U2). Adapted: the mocked `listRecoverablePushRuns` becomes real
//! `MetaCatalogSyncRun` rows in a migrated SQLite database and the default
//! queue a recording enqueuer; the distributed lock is not ported (one
//! in-process schedule consumer).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName, Retention,
};
use chatbotx_worker::Deps;
use serde_json::{Value, json};

/// 2026-07-16T00:00:00.000Z, the reconciler's `now`.
const NOW: i64 = 1_784_160_000_000;

#[derive(Default)]
struct Queue {
    bulks: Mutex<Vec<(QueueName, Vec<NewJob>)>>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        _: QueueName,
        _: &str,
        _: Value,
        _: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        panic!("the reconciler queues its jobs in one bulk")
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        self.bulks.lock().unwrap().push((queue, jobs));
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// (workspace, run, lease, status, direction, updatedAt)
type Run = (
    i64,
    i64,
    Option<&'static str>,
    &'static str,
    &'static str,
    i64,
);

async fn fixture() -> (Arc<Queue>, Deps) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-mc-reconciler-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@example.test')"#,
            [],
        )?;
        let rows: [Run; 5] = [
            (1, 11, Some("lease-1"), "running", "push", NOW - 3 * 60_000),
            (2, 12, None, "running", "push", NOW - 3 * 60_000),
            // Touched within the last 2 minutes: still live.
            (3, 13, None, "running", "push", NOW - 60_000),
            // Not a push run / not running.
            (4, 14, None, "running", "import", NOW - 3 * 60_000),
            (5, 15, None, "succeeded", "push", NOW - 3 * 60_000),
        ];
        for (ws, run, lease, status, direction, updated_at) in rows {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
                [ws],
            )?;
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'metaCatalog')"#,
                [ws + 100, ws],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId") VALUES (?1, ?2, ?3)"#,
                [ws + 200, ws, ws + 100],
            )?;
            tx.execute(
                r#"INSERT INTO "MetaCatalogSyncRun" ("id", "workspaceId", "integrationMetaCatalogId", "status",
                       "direction", "submissionLeaseId", "updatedAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                chatbotx_db::rusqlite::params![run, ws, ws + 200, status, direction, lease, updated_at],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed");
    let queue = Arc::new(Queue::default());
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(chatbotx_jobs::SystemClock))
            .await
            .expect("store"),
    );
    let deps = Deps {
        db,
        jobs: queue.clone(),
        waiter: store,
        realtime: None,
        channels: chatbotx_worker::Channels::new(),
        app_url: "http://app.test".into(),
        variables: None,
        ai: Arc::new(chatbotx_worker::ai::Ai::live()),
        files: None,
        webhook_http: Arc::new(chatbotx_worker::webhook::RecordingWebhookHttp::default()),
        channel_http: None,
        events: None,
        push: None,
    };
    (queue, deps)
}

/// "re-drives stale submissions and undispatched status checks".
#[tokio::test]
async fn meta_catalog_sync_reconciler_re_drives_stale_submissions_and_undispatched_status_checks() {
    let (queue, deps) = fixture().await;
    let out = chatbotx_worker::meta_catalog::sync::reconcile(&deps, NOW)
        .await
        .expect("reconcile");
    assert_eq!(out, json!({ "reconciled": 2 }));
    let bulks = queue.bulks.lock().unwrap();
    let (name, jobs) = &bulks[0];
    assert_eq!(*name, QueueName::Default);
    let seen: Vec<(&str, &Value)> = jobs.iter().map(|j| (j.name.as_str(), &j.data)).collect();
    assert_eq!(
        seen,
        vec![
            (
                "submitMetaCatalogSync",
                &json!({
                    "type": "submitMetaCatalogSync",
                    "data": { "workspaceId": "1", "runId": "11", "recovery": true },
                })
            ),
            (
                "checkMetaCatalogSync",
                &json!({
                    "type": "checkMetaCatalogSync",
                    "data": { "workspaceId": "2", "runId": "12", "attempt": 0 },
                })
            ),
        ]
    );
    for job in jobs {
        let id = job.opts.job_id.as_deref().unwrap();
        assert!(!id.contains(':'), "{id}");
        assert_eq!(job.opts.remove_on_complete, Some(Retention::Remove));
        assert_eq!(job.opts.remove_on_fail, Some(Retention::Remove));
    }
}
