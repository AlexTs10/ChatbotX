//! Ports of `apps/worker/__tests__/scan-smart-delay.test.ts` and
//! `smart-delay-resume-job.test.ts` (Q.D5 slice 7). Adapted: the mocked
//! `smartDelayService` becomes a migrated SQLite database and the
//! `integration` queue a recording enqueuer; the scan runs at a fixed `now`
//! with small batch bounds, so a cap test needs tens of rows, not 100 000.
//! Rows a sweep returns to `pending` are due, so the same tick claims them
//! again (the original mocks the claim empty).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_business::smart_delay as service;
use chatbotx_core::flow_config::steps::ENQUEUE_DELAY_MS;
use chatbotx_db::Db;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::models::ContactOnSmartDelay as Row;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use chatbotx_worker::Deps;
use chatbotx_worker::smart_delay::{
    JobSpec, ResumeExtras, ScanLimits, build_send_flow_resume_job, resume_job,
    scan_smart_delay_with, schedule_smart_delay_resume,
};
use serde_json::{Value, json};

const WS: i64 = 4001;
const CONVERSATION: i64 = 4101;
/// 2026-07-16T00:00:00.000Z, the scan's `now`.
const T0: i64 = 1_784_160_000_000;
const MINUTE: i64 = 60_000;

fn added() -> AddResult {
    AddResult {
        job_id: "1".into(),
        created: true,
    }
}

#[derive(Default)]
struct Queue {
    db: Mutex<Option<Arc<Db>>>,
    /// `(job id, row status when removed)`.
    removed: Mutex<Vec<(String, String)>>,
    bulks: Mutex<Vec<Vec<NewJob>>>,
    added: Mutex<Vec<(String, Value, JobOptions)>>,
    fail_add: Mutex<bool>,
    fail_bulk: Mutex<bool>,
    /// Job ids whose removal fails / reports an active job.
    remove_errors: Mutex<Vec<String>>,
    remove_active: Mutex<Vec<String>>,
    /// Row completed by its in-flight resume when its job is removed.
    complete_on_remove: Mutex<Option<i64>>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        assert_eq!(queue, QueueName::Integration);
        let name = name.to_owned();
        Box::pin(async move {
            if *self.fail_add.lock().unwrap() {
                return Err(JobsError::Other("redis down".into()));
            }
            self.added.lock().unwrap().push((name, data, opts));
            Ok(added())
        })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        assert_eq!(queue, QueueName::Integration);
        Box::pin(async move {
            if *self.fail_bulk.lock().unwrap() {
                return Err(JobsError::Other("redis down".into()));
            }
            let n = jobs.len();
            self.bulks.lock().unwrap().push(jobs);
            Ok(vec![added(); n])
        })
    }

    fn remove_job<'a>(
        &'a self,
        _queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            let db = self.db.lock().unwrap().clone().expect("db");
            let id: i64 = job_id.split('-').nth(2).unwrap().parse().unwrap();
            let status = service::find_by_id(&db, id)
                .await
                .unwrap()
                .map(|r| r.status)
                .unwrap_or_default();
            self.removed
                .lock()
                .unwrap()
                .push((job_id.to_owned(), status));
            if *self.complete_on_remove.lock().unwrap() == Some(id) {
                service::mark_completed(&db, id).await.unwrap();
            }
            if self
                .remove_errors
                .lock()
                .unwrap()
                .iter()
                .any(|j| j == job_id)
            {
                return Err(JobsError::Other("job is active".into()));
            }
            Ok(!self
                .remove_active
                .lock()
                .unwrap()
                .iter()
                .any(|j| j == job_id))
        })
    }
}

struct Fixture {
    db: Arc<Db>,
    queue: Arc<Queue>,
    deps: Deps,
    dir: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn insert(&self, rows: Vec<Row>) {
        for row in rows {
            service::create(&self.db, row).await.expect("create");
        }
    }

    async fn status(&self, id: i64) -> Option<String> {
        service::find_by_id(&self.db, id)
            .await
            .expect("find")
            .map(|r| r.status)
    }

    async fn exec(&self, sql: &'static str) {
        self.db
            .write(move |tx| {
                tx.execute_batch(sql)?;
                Ok(())
            })
            .await
            .expect("exec");
    }

    async fn scan(&self, limits: ScanLimits) -> Value {
        scan_smart_delay_with(&self.deps, T0, limits)
            .await
            .expect("scan")
    }

    fn removed(&self) -> Vec<(String, String)> {
        self.queue.removed.lock().unwrap().clone()
    }

    fn bulks(&self) -> Vec<Vec<NewJob>> {
        self.queue.bulks.lock().unwrap().clone()
    }
}

async fn fixture() -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-smart-delay-{}-{}",
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
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (4201, ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "additionalAttributes") VALUES (?1, ?2, 4201, ?3)"#,
            params![
                CONVERSATION,
                WS,
                json!({ "challenge": { "data": { "challengeId": "challenge-1" } } }).to_string()
            ],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let queue = Arc::new(Queue::default());
    *queue.db.lock().unwrap() = Some(db.clone());
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(chatbotx_jobs::SystemClock))
            .await
            .expect("store"),
    );
    let deps = Deps {
        db: db.clone(),
        jobs: queue.clone(),
        waiter: store,
        realtime: None,
        channels: chatbotx_worker::Channels::new(),
        app_url: "http://app.test".into(),
        variables: None,
        ai: Arc::new(chatbotx_worker::ai::Ai::live()),
        files: None,
        webhook_http: Arc::new(chatbotx_worker::webhook::RecordingWebhookHttp::default()),
        events: None,
        push: None,
    };
    Fixture {
        db,
        queue,
        deps,
        dir,
    }
}

/// `makeRow` of the original: a scheduled `waitNode` row due at T0 + 1 min.
fn row(id: i64, kind: &str, status: &str, trigger_at: i64) -> Row {
    Row {
        id,
        workspace_id: WS,
        flow_id: 61,
        flow_version_id: Some(71),
        contact_inbox_id: 81,
        conversation_id: CONVERSATION,
        node_id: Some("next-node".into()),
        step_id: Some("step-1".into()),
        metadata: None,
        r#type: kind.into(),
        created_at: T0,
        trigger_at,
        status: status.into(),
        appointment_id: None,
    }
}

fn due(id: i64) -> Row {
    row(id, "waitNode", "pending", T0 + MINUTE)
}

fn stuck(id: i64, trigger_at: i64) -> Row {
    row(id, "waitNode", "scheduled", trigger_at)
}

fn counts(scanned: usize, enqueued: usize) -> Value {
    json!({ "scanned": scanned, "enqueued": enqueued })
}

fn limits(batch_size: i64) -> ScanLimits {
    ScanLimits {
        batch_size,
        ..ScanLimits::default()
    }
}

fn job_id(id: i64, trigger_at: i64) -> String {
    format!("smart-delay-{id}-{trigger_at}")
}

// ---- scanSmartDelay -------------------------------------------------------

#[tokio::test]
async fn returns_zero_counts_when_no_smart_delay_rows_are_due() {
    let f = fixture().await;
    f.insert(vec![row(1, "waitNode", "pending", T0 + 10 * MINUTE)])
        .await;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(0, 0));
    assert!(f.removed().is_empty());
    assert!(f.bulks().is_empty());
    assert_eq!(f.status(1).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn removes_stale_wake_up_jobs_before_resetting_stuck_rows_to_pending() {
    let f = fixture().await;
    f.insert(vec![stuck(1, T0 - 20 * MINUTE), stuck(2, T0 - 19 * MINUTE)])
        .await;
    // Adapted: the reset rows are due, so this tick re-enqueues them.
    assert_eq!(f.scan(ScanLimits::default()).await, counts(2, 2));
    assert_eq!(
        f.removed(),
        vec![
            (job_id(1, T0 - 20 * MINUTE), "scheduled".into()),
            (job_id(2, T0 - 19 * MINUTE), "scheduled".into()),
        ],
        "removed while the rows were still scheduled"
    );
}

#[tokio::test]
async fn keeps_claiming_batches_until_a_partial_batch_signals_the_backlog_is_drained() {
    let f = fixture().await;
    f.insert((1..=6).map(due).collect()).await;
    assert_eq!(f.scan(limits(5)).await, counts(6, 6));
    assert_eq!(
        f.bulks().iter().map(Vec::len).collect::<Vec<_>>(),
        vec![5, 1]
    );
}

#[tokio::test]
async fn sweeps_stuck_rows_in_batches_until_a_partial_batch() {
    let f = fixture().await;
    f.insert((1..=6).map(|id| stuck(id, T0 - 20 * MINUTE + id)).collect())
        .await;
    f.scan(limits(5)).await;
    assert_eq!(f.removed().len(), 6);
}

#[tokio::test]
async fn reports_only_rows_the_cas_actually_reset_when_one_completed_mid_sweep() {
    let f = fixture().await;
    f.insert(vec![stuck(1, T0 - 20 * MINUTE), stuck(2, T0 - 19 * MINUTE)])
        .await;
    *f.queue.complete_on_remove.lock().unwrap() = Some(2);
    assert_eq!(f.scan(ScanLimits::default()).await, counts(1, 1));
    assert_eq!(f.status(2).await.as_deref(), Some("completed"));
}

#[tokio::test]
async fn treats_an_unremovable_active_job_as_fulfilled_and_defers_to_the_cas() {
    let f = fixture().await;
    f.insert(vec![stuck(1, T0 - 20 * MINUTE)]).await;
    f.queue
        .remove_active
        .lock()
        .unwrap()
        .push(job_id(1, T0 - 20 * MINUTE));
    assert_eq!(f.scan(ScanLimits::default()).await, counts(1, 1));
}

#[tokio::test]
async fn continues_claiming_when_the_sweep_itself_fails() {
    // Adapted: the sweep fails at its reset (a trigger refuses `pending`).
    let f = fixture().await;
    f.insert(vec![stuck(1, T0 - 20 * MINUTE), due(2)]).await;
    f.exec(
        r#"CREATE TRIGGER "no_reset" BEFORE UPDATE OF "status" ON "ContactOnSmartDelay"
           WHEN NEW."status" = 'pending' BEGIN SELECT RAISE(ABORT, 'db unavailable'); END;"#,
    )
    .await;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(1, 1));
    assert_eq!(f.status(1).await.as_deref(), Some("scheduled"));
}

#[tokio::test]
async fn warns_when_the_sweep_exhausts_its_per_run_batch_cap() {
    // Adapted: the warning is not captured; the rows left are.
    let f = fixture().await;
    f.insert((1..=5).map(|id| stuck(id, T0 - 20 * MINUTE + id)).collect())
        .await;
    let capped = ScanLimits {
        batch_size: 2,
        max_sweep_batches: 2,
        max_claim_batches: 200,
    };
    assert_eq!(f.scan(capped).await, counts(4, 4));
    assert_eq!(f.removed().len(), 4);
    assert_eq!(f.status(5).await.as_deref(), Some("scheduled"));
}

#[tokio::test]
async fn warns_when_the_claim_loop_exhausts_its_per_run_batch_cap() {
    // Adapted: the warning is not captured; the backlog left is.
    let f = fixture().await;
    f.insert((1..=7).map(due).collect()).await;
    let capped = ScanLimits {
        batch_size: 2,
        max_sweep_batches: 20,
        max_claim_batches: 3,
    };
    assert_eq!(f.scan(capped).await, counts(6, 6));
    assert_eq!(f.status(7).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn completes_remaining_enqueueable_rows_when_a_terminal_row_update_fails() {
    // Adapted: a trigger makes the terminal row's completion fail.
    let f = fixture().await;
    f.insert(vec![
        Row {
            node_id: None,
            ..due(1)
        },
        due(2),
    ])
    .await;
    f.exec(
        r#"CREATE TRIGGER "no_complete" BEFORE UPDATE OF "status" ON "ContactOnSmartDelay"
           WHEN NEW."status" = 'completed' BEGIN SELECT RAISE(ABORT, 'db timeout'); END;"#,
    )
    .await;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(2, 1));
    assert_eq!(f.bulks().len(), 1);
    assert_eq!(f.status(1).await.as_deref(), Some("scheduled"));
}

#[tokio::test]
async fn still_resets_stuck_rows_when_removing_a_stale_job_fails() {
    let f = fixture().await;
    f.insert(vec![stuck(1, T0 - 20 * MINUTE), stuck(2, T0 - 19 * MINUTE)])
        .await;
    f.queue
        .remove_errors
        .lock()
        .unwrap()
        .push(job_id(1, T0 - 20 * MINUTE));
    assert_eq!(f.scan(ScanLimits::default()).await, counts(2, 2));
}

#[tokio::test]
async fn enqueues_wait_rows_as_resume_wait_and_follow_up_rows_as_resume_follow_up() {
    let f = fixture().await;
    f.insert(vec![
        row(1, "waitNode", "pending", T0 + MINUTE),
        row(2, "followUp", "pending", T0 + MINUTE),
        // The window ends at 00:05:59.999.
        row(3, "waitNode", "pending", T0 + 6 * MINUTE - 1),
        row(4, "waitNode", "pending", T0 + 6 * MINUTE),
    ])
    .await;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(3, 3));
    let job = |id: i64, name: &str, delay: i64| NewJob {
        name: name.into(),
        data: json!({ "type": name, "data": { "smartDelayId": id.to_string() } }),
        opts: JobOptions {
            job_id: Some(job_id(id, T0 + delay)),
            delay_ms: Some(delay as u64),
            ..JobOptions::default()
        },
    };
    assert_eq!(
        f.bulks(),
        vec![vec![
            job(1, "resumeWait", MINUTE),
            job(2, "resumeFollowUp", MINUTE),
            job(3, "resumeWait", 6 * MINUTE - 1),
        ]]
    );
    assert_eq!(f.status(4).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn resets_a_failed_enqueue_batch_to_pending() {
    let f = fixture().await;
    f.insert(vec![due(1)]).await;
    *f.queue.fail_bulk.lock().unwrap() = true;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(1, 0));
    assert_eq!(f.status(1).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn logs_terminal_rows_without_enqueueing_them() {
    let f = fixture().await;
    f.insert(vec![Row {
        node_id: None,
        ..due(1)
    }])
    .await;
    assert_eq!(f.scan(ScanLimits::default()).await, counts(1, 0));
    assert_eq!(f.status(1).await.as_deref(), Some("completed"));
    assert!(f.bulks().is_empty());
}

// ---- resume jobs ------------------------------------------------------------

fn resume_row(kind: &str) -> Row {
    row(1, kind, "scheduled", T0 + MINUTE)
}

fn broadcast(kind: &str) -> Row {
    Row {
        metadata: Some(Json(
            json!({ "type": "broadcast", "broadcastId": "b-1", "contactInboxId": "ci-1" }),
        )),
        ..resume_row(kind)
    }
}

#[test]
fn never_emits_started_by_contact_message_so_a_delayed_resume_is_not_message_driven() {
    let extras = ResumeExtras {
        send_from: Some("inbox".into()),
        appointment_id: Some(9),
        ..ResumeExtras::default()
    };
    let with_extras = build_send_flow_resume_job(&resume_row("waitNode"), Some(&extras));
    assert!(
        with_extras.data["data"]
            .get("startedByContactMessage")
            .is_none()
    );
    assert_eq!(with_extras.data["data"]["sendFrom"], "inbox");
    let plain = build_send_flow_resume_job(&resume_row("waitNode"), None);
    assert!(plain.data["data"].get("startedByContactMessage").is_none());
}

#[test]
fn every_smart_delay_type_can_be_resumed() {
    for (kind, name) in [
        ("waitNode", "resumeWait"),
        ("followUp", "resumeFollowUp"),
        ("quickReplyFollowUp", "resumeQuickReplyFollowUp"),
        ("stepInputTimeout", "resumeStepInputTimeout"),
    ] {
        assert_eq!(resume_job(&resume_row(kind), None).name, name);
    }
}

#[test]
fn resumes_through_the_dedicated_job_carrying_only_the_row_id() {
    assert_eq!(
        resume_job(&resume_row("stepInputTimeout"), None),
        JobSpec {
            name: "resumeStepInputTimeout",
            opts: JobOptions::default(),
            data: json!({ "type": "resumeStepInputTimeout", "data": { "smartDelayId": "1" } }),
        }
    );
}

/// `scheduleSmartDelayResume({ type: stepInputTimeout, .. })` due in `in_ms`.
async fn schedule(f: &Fixture, in_ms: i64, challenge: &str) -> Row {
    let row = service::new_row(
        "stepInputTimeout",
        WS,
        61,
        Some(71),
        CONVERSATION,
        81,
        Some("node-1".into()),
        Some("step-1".into()),
        Some(json!({
            "type": "stepInputResolution",
            "stepId": "step-1",
            "challengeId": challenge,
            "reason": "timeout",
        })),
        chatbotx_db::repo::now_ms() + in_ms,
        None,
    );
    schedule_smart_delay_resume(&f.deps, row.clone(), ResumeExtras::default())
        .await
        .expect("schedule");
    row
}

#[tokio::test]
async fn persists_through_the_one_timer_per_conversation_upsert_resuming_the_step_s_node() {
    // Adapted: due in 10 min (not 1 min) so the row stays as persisted, not enqueued.
    let f = fixture().await;
    let row = schedule(&f, 10 * MINUTE, "challenge-1").await;
    let stored = service::find_by_id(&f.db, row.id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(
        (
            stored.r#type.as_str(),
            stored.node_id.as_deref(),
            stored.step_id.as_deref(),
            stored.conversation_id,
            stored.status.as_str()
        ),
        (
            "stepInputTimeout",
            Some("node-1"),
            Some("step-1"),
            CONVERSATION,
            "pending"
        )
    );
    assert_eq!(
        stored
            .metadata
            .as_deref()
            .and_then(|m| m.get("challengeId")),
        Some(&json!("challenge-1"))
    );
}

#[tokio::test]
async fn a_near_deadline_is_enqueued_with_its_delay() {
    let f = fixture().await;
    let row = schedule(&f, 30_000, "challenge-1").await;
    assert_eq!(f.status(row.id).await.as_deref(), Some("scheduled"));
    let added = f.queue.added.lock().unwrap().clone();
    assert_eq!(added.len(), 1);
    let (name, data, opts) = &added[0];
    assert_eq!(name, "resumeStepInputTimeout");
    assert_eq!(data["type"], "resumeStepInputTimeout");
    assert!(opts.delay_ms.is_some_and(|d| d <= 30_000));
    assert!(
        opts.job_id
            .as_deref()
            .is_some_and(|j| j.starts_with("smart-delay-"))
    );
}

#[tokio::test]
async fn a_far_deadline_is_left_to_the_scanner_not_parked_in_the_queue() {
    let f = fixture().await;
    let row = schedule(&f, ENQUEUE_DELAY_MS as i64 + MINUTE, "challenge-1").await;
    assert_eq!(f.status(row.id).await.as_deref(), Some("pending"));
    assert!(f.queue.added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_arm_skipped_for_a_replaced_question_enqueues_nothing() {
    let f = fixture().await;
    let row = schedule(&f, 30_000, "older-challenge").await;
    assert_eq!(f.status(row.id).await, None);
    assert!(f.queue.added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_enqueue_returns_the_row_to_pending_so_the_scanner_recovers_it() {
    let f = fixture().await;
    *f.queue.fail_add.lock().unwrap() = true;
    let row = schedule(&f, 30_000, "challenge-1").await;
    assert_eq!(f.status(row.id).await.as_deref(), Some("pending"));
}

#[test]
fn a_live_flow_s_resume_gets_no_priority_a_broadcast_s_keeps_the_scheduled_send_tier() {
    assert_eq!(
        build_send_flow_resume_job(&resume_row("waitNode"), None).opts,
        JobOptions::default()
    );
    assert_eq!(
        build_send_flow_resume_job(&broadcast("waitNode"), None)
            .opts
            .priority,
        Some(5)
    );
}

fn same_priority_rule(kind: &str, name: &str) {
    let live = resume_job(&resume_row(kind), None);
    let scheduled = resume_job(&broadcast(kind), None);
    assert_eq!(live.name, name);
    assert_eq!(live.opts, JobOptions::default());
    assert_eq!(
        scheduled.opts,
        JobOptions {
            priority: Some(5),
            ..JobOptions::default()
        }
    );
}

#[test]
fn the_wait_node_resume_job_carries_the_same_priority_rule() {
    same_priority_rule("waitNode", "resumeWait");
}

#[test]
fn the_follow_up_resume_job_carries_the_same_priority_rule() {
    same_priority_rule("followUp", "resumeFollowUp");
}

#[test]
fn the_quick_reply_follow_up_resume_job_carries_the_same_priority_rule() {
    same_priority_rule("quickReplyFollowUp", "resumeQuickReplyFollowUp");
}
