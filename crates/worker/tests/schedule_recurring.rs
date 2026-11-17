//! Ports of `apps/worker/__tests__/enqueue-recurring-broadcasts.test.ts`,
//! `materialize-recurring-broadcasts.test.ts` and
//! `schedule-worker-recurring-broadcasts-dispatch.test.ts` (Q.D5 slice 5b).
//! Adapted: the mocked `broadcastRecurrenceService` becomes a migrated SQLite
//! database and the queues a recording enqueuer; the page size and the
//! per-tick cap are parameters so a test needs tens of rows, not 200 000.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobCtx, JobError, JobOptions, JobsError, NewJob, QueueName,
    Retention,
};
use chatbotx_worker::Deps;
use chatbotx_worker::schedule::broadcast::cron_lock;
use chatbotx_worker::schedule::recurring::{self, LOCK_KEY, MAX_PENDING_BATCHES};
use serde_json::{Value, json};

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;
const DAY: i64 = 86_400_000;
/// 2030-10-13 09:00 in Ho Chi Minh: armed, but not due by the real clock.
const SLOT: i64 = 1_918_087_200_000;

#[derive(Debug, Clone)]
struct Call {
    queue: QueueName,
    name: String,
    data: Value,
    opts: JobOptions,
}

#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<Call>>,
    bulks: Mutex<Vec<Vec<NewJob>>>,
    fail: Mutex<bool>,
    backlog: Mutex<u64>,
}

impl Enqueuer for Recording {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let fail = *self.fail.lock().unwrap();
        if !fail {
            self.calls.lock().unwrap().push(Call {
                queue,
                name: name.into(),
                data,
                opts,
            });
        }
        Box::pin(async move {
            if fail {
                return Err(JobsError::Other("redis down".into()));
            }
            Ok(AddResult {
                job_id: "1".into(),
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let fail = *self.fail.lock().unwrap();
        if !fail {
            self.bulks.lock().unwrap().push(jobs);
        }
        Box::pin(async move {
            if fail {
                return Err(JobsError::Other("redis down".into()));
            }
            Ok(Vec::new())
        })
    }

    fn backlog_count(&self, _: QueueName) -> BoxFuture<'_, Result<u64, JobsError>> {
        let backlog = *self.backlog.lock().unwrap();
        Box::pin(async move { Ok(backlog) })
    }
}

struct Fixture {
    db: Arc<Db>,
    jobs: Arc<Recording>,
    deps: Deps,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    fn prepares(&self) -> Vec<Call> {
        self.jobs
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.name == "prepareBroadcast")
            .cloned()
            .collect()
    }

    fn audits(&self) -> Vec<Value> {
        self.jobs
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.name == "sendAuditLog")
            .map(|c| c.data["data"].clone())
            .collect()
    }

    fn bulks(&self) -> Vec<Vec<NewJob>> {
        self.jobs.bulks.lock().unwrap().clone()
    }

    async fn exec(&self, sql: String) {
        self.db
            .write(move |tx| {
                tx.execute_batch(&sql)?;
                Ok(())
            })
            .await
            .expect("exec");
    }

    async fn int(&self, sql: String) -> Option<i64> {
        self.db
            .read(move |conn| {
                Ok(conn
                    .query_row(&sql, [], |r| r.get::<_, Option<i64>>(0))
                    .ok()
                    .flatten())
            })
            .await
            .expect("read")
    }

    /// The runs of template `id`.
    async fn runs(&self, id: i64) -> Vec<i64> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    r#"SELECT "id" FROM "Broadcast" WHERE "parentBroadcastId" = ?1 ORDER BY "id""#,
                )?;
                let ids = stmt
                    .query_map([id], |r| r.get(0))?
                    .collect::<Result<Vec<i64>, _>>()?;
                Ok(ids)
            })
            .await
            .expect("runs")
    }

    /// A daily 09:00 series on template `id` (recurrence `id + 1000`), armed at `next_run_at`.
    async fn series(&self, id: i64, workspace: i64, next_run_at: i64, rule: Value) {
        let rule = rule.to_string();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "flowId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                       VALUES (?1, 'Daily promo', ?2, 61, 'recurring', 'recurring', ?3, 'allContacts', 'webchat')"#,
                    params![id, workspace, next_run_at],
                )?;
                tx.execute(
                    r#"INSERT INTO "BroadcastRecurrence" ("id", "workspaceId", "broadcastId", "rule", "nextRunAt", "accountedThroughAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                    params![id + 1000, workspace, id, rule, next_run_at, next_run_at - DAY],
                )?;
                Ok(())
            })
            .await
            .expect("series");
    }

    /// `count` series due long ago (recurrence ids `20001..`), all at the same instant.
    async fn due(&self, count: i64) {
        let rule = rule().to_string();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                       INSERT INTO "Broadcast" ("id", "name", "workspaceId", "flowId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                       SELECT 10000 + i, 'B', ?2, 61, 'recurring', 'recurring', 0, 'allContacts', 'webchat' FROM n"#,
                    params![count, WS],
                )?;
                tx.execute(
                    r#"WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                       INSERT INTO "BroadcastRecurrence" ("id", "workspaceId", "broadcastId", "rule", "nextRunAt", "accountedThroughAt")
                       SELECT 20000 + i, ?2, 10000 + i, ?3, 1000000, 0 FROM n"#,
                    params![count, WS, rule],
                )?;
                Ok(())
            })
            .await
            .expect("due");
    }
}

fn rule() -> Value {
    json!({
        "frequency": "daily",
        "interval": 1,
        "timeOfDay": "09:00",
        "timezone": "Asia/Ho_Chi_Minh",
        "startsOn": "2030-10-01",
        "end": { "kind": "never" },
    })
}

fn seed(tx: &Connection) -> chatbotx_db::rusqlite::Result<()> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (1001, 'a@example.test')"#,
        [],
    )?;
    for ws in [WS, OTHER_WS] {
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1001)"#,
            [ws],
        )?;
    }
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo')"#,
        [WS],
    )?;
    Ok(())
}

async fn fixture() -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-recurring-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    db.write(|tx| Ok(seed(tx)?)).await.expect("seed");
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(chatbotx_jobs::SystemClock))
            .await
            .expect("store"),
    );
    let jobs = Arc::new(Recording::default());
    let deps = Deps {
        db: db.clone(),
        jobs: jobs.clone(),
        waiter: store,
        realtime: None,
        channels: chatbotx_worker::Channels::new(),
        app_url: "http://app.test".into(),
        variables: None,
    };
    Fixture {
        db,
        jobs,
        deps,
        dir,
    }
}

fn item(recurrence: i64, workspace: i64, at: i64) -> Value {
    json!({
        "recurrenceId": recurrence.to_string(),
        "workspaceId": workspace.to_string(),
        "occurrenceAtMs": at,
    })
}

fn tick(scanned: usize, enqueued: usize, skipped: bool) -> Value {
    json!({ "scanned": scanned, "enqueued": enqueued, "skipped": skipped })
}

// ---- enqueueRecurringBroadcasts ----------------------------------------

#[tokio::test]
async fn does_nothing_but_report_the_backlog_age_when_no_series_is_due() {
    let f = fixture().await;
    let out = recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();
    assert_eq!(out, tick(0, 0, false));
    assert!(f.bulks().is_empty());
}

#[tokio::test]
async fn hands_due_series_to_the_schedule_queue_in_batches_carrying_the_armed_instant() {
    let f = fixture().await;
    f.due(250).await;

    let out = recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();

    assert_eq!(out, tick(250, 250, false));
    let jobs = &f.bulks()[0];
    assert_eq!(
        jobs.iter()
            .map(|j| j.data["data"]["items"].as_array().unwrap().len())
            .collect::<Vec<_>>(),
        vec![100, 100, 50]
    );
    assert_eq!(jobs[0].name, "materializeRecurringBroadcasts");
    assert_eq!(jobs[0].data["type"], "materializeRecurringBroadcasts");
    assert_eq!(jobs[0].data["data"]["items"][0], item(20001, WS, 1_000_000));
}

#[tokio::test]
async fn batches_are_prioritized_so_they_yield_to_every_other_schedule_job() {
    let f = fixture().await;
    f.due(150).await;
    recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();
    let jobs = &f.bulks()[0];
    assert_eq!(jobs.len(), 2);
    for job in jobs {
        assert_eq!(job.opts.priority, Some(10));
    }
}

#[tokio::test]
async fn batch_job_ids_carry_no_colon_and_free_their_key_when_they_end() {
    let f = fixture().await;
    f.due(150).await;
    recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();
    let jobs = &f.bulks()[0];
    let mut ids = std::collections::HashSet::new();
    for job in jobs {
        let id = job.opts.job_id.clone().unwrap();
        assert!(!id.contains(':'), "{id}");
        assert!(matches!(
            job.opts.remove_on_complete,
            Some(Retention::Remove)
        ));
        assert!(matches!(job.opts.remove_on_fail, Some(Retention::Remove)));
        ids.insert(id);
    }
    assert_eq!(ids.len(), jobs.len());
}

#[tokio::test]
async fn pages_through_a_large_backlog_with_a_keyset_cursor_until_a_short_page() {
    // Adapted: pages of 100 instead of 5000.
    let f = fixture().await;
    f.due(150).await;

    let out = recurring::enqueue_recurring_broadcasts_with(&f.deps, 100, 200_000)
        .await
        .unwrap();

    assert_eq!(out["scanned"], 150);
    let bulks = f.bulks();
    assert_eq!(bulks.len(), 2);
    // The second page starts after the cursor: no series is handed out twice.
    assert_eq!(
        bulks[1][0].data["data"]["items"][0]["recurrenceId"],
        "20101"
    );
    assert!(bulks[1][0].opts.job_id.clone().unwrap().ends_with("-1-0"));
}

#[tokio::test]
async fn reports_the_oldest_due_age_so_an_unhealthy_worker_is_visible() {
    // Adapted: the logged value is the business function's result.
    let f = fixture().await;
    f.due(1).await;
    let age = chatbotx_business::broadcast::materialize::find_oldest_due_age_ms(&f.db)
        .await
        .unwrap()
        .unwrap();
    assert!(age > 0);
}

#[tokio::test]
async fn skips_the_tick_when_the_previous_one_still_holds_the_lock_without_queueing_behind_it() {
    // Covers "takes the lock without queueing behind a previous tick" too.
    let f = fixture().await;
    f.due(1).await;
    let _held = cron_lock(&f.deps, LOCK_KEY).lock().await;

    let out = tokio::time::timeout(
        Duration::from_secs(5),
        recurring::enqueue_recurring_broadcasts(&f.deps),
    )
    .await
    .expect("does not wait")
    .unwrap();

    assert_eq!(out, tick(0, 0, true));
    assert!(f.bulks().is_empty());
}

#[tokio::test]
async fn adds_nothing_while_the_schedule_queue_is_backlogged() {
    let f = fixture().await;
    f.due(1).await;
    *f.jobs.backlog.lock().unwrap() = MAX_PENDING_BATCHES;
    assert_eq!(MAX_PENDING_BATCHES, 2000);

    let out = recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();

    assert_eq!(out, tick(0, 0, true));
    assert!(f.bulks().is_empty());
}

#[tokio::test]
async fn a_backlog_just_under_the_limit_still_enqueues() {
    let f = fixture().await;
    f.due(1).await;
    *f.jobs.backlog.lock().unwrap() = 1999;
    let out = recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap();
    assert_eq!(out["enqueued"], 1);
}

#[tokio::test]
async fn any_other_failure_propagates_so_the_cron_job_fails_loudly() {
    // Adapted: the failing store is the queue (`addBulk`), not the lock.
    let f = fixture().await;
    f.due(1).await;
    *f.jobs.fail.lock().unwrap() = true;
    let err = recurring::enqueue_recurring_broadcasts(&f.deps)
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("redis down"), "{err:?}");
}

#[tokio::test]
async fn stops_enqueueing_once_the_per_tick_cap_is_reached() {
    // Adapted: a cap of 4 with pages of 2 instead of 200 000 with pages of 5000.
    let f = fixture().await;
    f.due(10).await;
    let out = recurring::enqueue_recurring_broadcasts_with(&f.deps, 2, 4)
        .await
        .unwrap();
    assert_eq!(out["enqueued"], 4);
    assert_eq!(f.bulks().len(), 2);
}

// ---- materializeRecurringBroadcasts ------------------------------------

#[tokio::test]
async fn materializes_each_slot_under_its_own_workspaces_audit_context() {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;

    recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)])
        .await
        .unwrap();

    assert_eq!(f.runs(301).await.len(), 1);
    let audit = &f.audits()[0];
    assert_eq!(audit["workspaceId"], WS.to_string());
    assert_eq!(audit["source"], "broadcastRecurrence:materialize");
    assert_eq!(audit["userId"], "system");
}

#[tokio::test]
async fn the_series_must_belong_to_the_workspace_the_owner_guard_checked() {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;
    recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, OTHER_WS, SLOT)])
        .await
        .unwrap();
    assert!(f.runs(301).await.is_empty());
}

#[tokio::test]
async fn prepares_a_created_run_right_away_with_the_id_the_schedule_tick_uses() {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;

    recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)])
        .await
        .unwrap();

    let run = f.runs(301).await[0].to_string();
    let prepares = f.prepares();
    assert_eq!(prepares.len(), 1);
    assert_eq!(prepares[0].queue, QueueName::Schedule);
    assert_eq!(
        prepares[0].data,
        json!({ "type": "prepareBroadcast", "data": { "broadcastId": run } })
    );
    assert_eq!(
        prepares[0].opts.job_id.as_deref(),
        Some(format!("schedule-prepare-broadcast-{run}").as_str())
    );
    assert!(matches!(
        prepares[0].opts.remove_on_complete,
        Some(Retention::Remove)
    ));
    assert!(matches!(
        prepares[0].opts.remove_on_fail,
        Some(Retention::Remove)
    ));
}

#[tokio::test]
async fn prepares_the_final_run_of_a_series_too() {
    let f = fixture().await;
    let mut last = rule();
    last["end"] = json!({ "kind": "afterOccurrences", "count": 1 });
    f.series(301, WS, SLOT, last).await;
    recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)])
        .await
        .unwrap();
    assert_eq!(f.prepares().len(), 1);
}

#[tokio::test]
async fn prepares_nothing_for_a_skipped_stale_or_empty_slot() {
    let f = fixture().await;
    let mut excluded = rule();
    excluded["excludeDates"] = json!(["2030-10-13"]);
    f.series(301, WS, SLOT, excluded).await;
    f.series(302, WS, SLOT, rule()).await;

    recurring::materialize_recurring_broadcasts(
        &f.deps,
        &[item(1301, WS, SLOT), item(1302, WS, SLOT - DAY)],
    )
    .await
    .unwrap();

    assert!(f.prepares().is_empty());
}

#[tokio::test]
async fn a_blocked_owners_slot_is_skipped_and_the_series_moves_on_never_materialized() {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;
    f.exec(format!(
        r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = {WS}"#
    ))
    .await;

    recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)])
        .await
        .unwrap();

    assert!(f.runs(301).await.is_empty());
    assert!(f.prepares().is_empty());
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "BroadcastRecurrenceOccurrence" WHERE "reason" = 'blockedOwner'"#.into())
            .await,
        Some(1)
    );
    assert_eq!(
        f.int(r#"SELECT "nextRunAt" FROM "BroadcastRecurrence" WHERE "id" = 1301"#.into())
            .await,
        Some(SLOT + DAY)
    );
}

#[tokio::test]
async fn a_failed_prepare_enqueue_is_logged_not_thrown_the_run_is_committed_and_the_tick_is_the_fallback()
 {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;
    *f.jobs.fail.lock().unwrap() = true;

    let out = recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)]).await;

    assert_eq!(out.unwrap(), Value::Null);
    assert_eq!(f.runs(301).await.len(), 1);
}

#[tokio::test]
async fn one_failing_series_does_not_stop_the_others_and_the_job_fails_afterwards_so_it_is_retried()
{
    // Adapted: the failing series has a broken stored rule (the mock threw "deadlock").
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;
    f.series(302, WS, SLOT, json!({ "broken": true })).await;
    f.series(303, WS, SLOT, rule()).await;

    let err = recurring::materialize_recurring_broadcasts(
        &f.deps,
        &[
            item(1301, WS, SLOT),
            item(1302, WS, SLOT),
            item(1303, WS, SLOT),
        ],
    )
    .await
    .unwrap_err();

    assert!(
        matches!(&err, JobError::Retryable(_))
            && err
                .to_string()
                .contains("1 of 3 recurring broadcast slots failed"),
        "{err:?}"
    );
    assert_eq!(f.prepares().len(), 2);
}

#[tokio::test]
async fn an_empty_batch_is_a_no_op() {
    let f = fixture().await;
    let out = recurring::materialize_recurring_broadcasts(&f.deps, &[]).await;
    assert_eq!(out.unwrap(), Value::Null);
    assert!(f.jobs.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_thrown_blocked_owner_check_is_a_per_series_failure_not_a_skip() {
    // Adapted: the owner check fails because its table is gone (the mock rejected).
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;
    f.exec(r#"ALTER TABLE "Workspace" RENAME TO "WorkspaceGone""#.into())
        .await;

    let out = recurring::materialize_recurring_broadcasts(&f.deps, &[item(1301, WS, SLOT)]).await;

    assert!(out.is_err());
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "BroadcastRecurrenceOccurrence""#.into())
            .await,
        Some(0)
    );
    assert!(f.runs(301).await.is_empty());
}

// ---- schedule worker dispatch ------------------------------------------

fn job(data: Value) -> JobCtx {
    JobCtx {
        data,
        ..JobCtx::default()
    }
}

#[tokio::test]
async fn hands_a_materialize_batch_s_items_to_the_materializer_only() {
    let f = fixture().await;
    f.series(301, WS, SLOT, rule()).await;

    let out = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "materializeRecurringBroadcasts", "data": { "items": [item(1301, WS, SLOT)] } })),
    )
    .await;

    assert_eq!(out.unwrap(), Value::Null);
    assert_eq!(f.runs(301).await.len(), 1);
    assert!(f.bulks().is_empty());
}

#[tokio::test]
async fn a_failed_batch_fails_the_job_so_it_is_retried() {
    let f = fixture().await;
    f.series(301, WS, SLOT, json!({ "broken": true })).await;

    let out = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "materializeRecurringBroadcasts", "data": { "items": [item(1301, WS, SLOT)] } })),
    )
    .await;

    assert!(
        matches!(&out, Err(e @ JobError::Retryable(_)) if e.to_string() == "1 of 1 recurring broadcast slots failed"),
        "{out:?}"
    );
}
