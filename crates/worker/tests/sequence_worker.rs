//! Ports of `apps/worker/__tests__/sequence-step-executor.test.ts`,
//! `sequence-dispatch-processor.test.ts` and `sequence-flow.test.ts`
//! (Q.D5 slice 6). Adapted: the mocked repositories, `SchedulerClient`,
//! `advanceEnrollment` and `sendFlowDirect` become a migrated SQLite
//! database (the `SequenceDispatch`, `ContactOnSequence` and
//! `sequence_schedule` rows are the assertions), so each "was called with"
//! reads as the row it writes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::sequence::consumer::{SequenceConsumerServices, StepRow, StepValidation};
use chatbotx_jobs::sequence::sqlite_schedule::{SqliteDispatchSchedule, schedule_in};
use chatbotx_jobs::{JobCtx, SystemClock};
use chatbotx_worker::Deps;
use chatbotx_worker::sequence::{
    ConsumerServices, claim_dispatch, fetch_step, find_dispatch, handle_send_sequence_flow,
    is_dispatch_ready, validate_dispatch, validate_step,
};
use serde_json::{Value, json};

const WS: i64 = 3001;
const OTHER_WS: i64 = 3002;
const CONTACT: i64 = 3101;
const INBOX: i64 = 3201;
const CONTACT_INBOX: i64 = 3301;
const SEQ: i64 = 3401;
const STEP: i64 = 3501;
const FLOW: i64 = 3601;
/// A flow with no published version: sending it fails.
const BROKEN_FLOW: i64 = 3602;
const ENROLLMENT: i64 = 3701;
const DISPATCH: i64 = 3901;
const BUCKET: u32 = 42;

struct Fixture {
    db: Arc<Db>,
    deps: Deps,
    dir: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn exec(&self, sql: &'static str) {
        self.db
            .write(move |tx| {
                tx.execute_batch(sql)?;
                Ok(())
            })
            .await
            .expect("exec");
    }

    /// `(status, lastError, completedAt, lockOwner)` of the dispatch.
    async fn dispatch(&self) -> (String, Option<String>, Option<i64>, Option<String>) {
        self.db
            .read(|conn| {
                Ok(conn.query_row(
                    r#"SELECT "status", "lastError", "completedAt", "lockOwner" FROM "SequenceDispatch" WHERE "id" = ?1"#,
                    [DISPATCH],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?)
            })
            .await
            .expect("dispatch")
    }

    /// `(status, completedAt, currentStep)` of the enrollment.
    async fn enrollment(&self) -> (Option<String>, Option<i64>, i64) {
        self.db
            .read(|conn| {
                Ok(conn.query_row(
                    r#"SELECT "status", "completedAt", "currentStep" FROM "ContactOnSequence" WHERE "id" = ?1"#,
                    [ENROLLMENT],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .await
            .expect("enrollment")
    }

    /// The bucket of the dispatch's `schedule` entry, if any.
    async fn scheduled(&self) -> Option<u32> {
        self.db
            .read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT bucket FROM sequence_schedule WHERE kind = 'schedule' AND dispatch_id = ?1",
                        [DISPATCH],
                        |r| r.get(0),
                    )
                    .optional()?)
            })
            .await
            .expect("scheduled")
    }

    fn services(&self) -> ConsumerServices {
        ConsumerServices {
            db: self.db.clone(),
            schedule: Arc::new(SqliteDispatchSchedule::new(self.db.clone(), None)),
            clock: Arc::new(SystemClock),
        }
    }
}

fn seed(tx: &Connection, status: &str) -> chatbotx_db::rusqlite::Result<()> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@example.test')"#,
        [],
    )?;
    for ws in [WS, OTHER_WS] {
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
            [ws],
        )?;
    }
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
        params![CONTACT, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?1, 'inbox', 'webchat', 'src', ?2)"#,
        params![INBOX, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
           VALUES (?1, ?2, ?2, ?3, 'webchat', 'webchat', 'contact-src')"#,
        params![CONTACT_INBOX, CONTACT, INBOX],
    )?;
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Step flow'), (?3, ?2, 'Broken flow')"#,
        params![FLOW, WS, BROKEN_FLOW],
    )?;
    // A start node with no steps: the run succeeds and sends nothing.
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
           VALUES (3651, ?1, ?2, ?3, '[]', 0, 1, 1)"#,
        params![
            WS,
            FLOW,
            json!([{ "id": "n1", "data": { "isStartNode": true, "details": { "steps": [] } } }])
                .to_string()
        ],
    )?;
    tx.execute(
        r#"UPDATE "Flow" SET "currentVersionId" = 3651 WHERE "id" = ?1"#,
        [FLOW],
    )?;
    tx.execute(
        r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'sequence', ?2)"#,
        params![SEQ, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId", "isActive", "flowId") VALUES (?1, 1, 0, ?2, 1, ?3)"#,
        params![STEP, SEQ, FLOW],
    )?;
    tx.execute(
        r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status") VALUES (?1, ?2, ?3, ?4, 'active')"#,
        params![ENROLLMENT, CONTACT, SEQ, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "SequenceDispatch" ("id", "workspaceId", "sequenceId", "enrollmentId", "stepId", "contactId",
              "contactInboxId", "bucket", "runAtMs", "status", "attempt", "idempotencyKey")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1700000000000, ?9, 0, 'key')"#,
        params![
            DISPATCH,
            WS,
            SEQ,
            ENROLLMENT,
            STEP,
            CONTACT,
            CONTACT_INBOX,
            BUCKET,
            status
        ],
    )?;
    schedule_in(tx, BUCKET, &DISPATCH.to_string(), 1_700_000_000_000)
        .map_err(|e| chatbotx_db::rusqlite::Error::InvalidParameterName(e.to_string()))?;
    Ok(())
}

/// The fixture with the dispatch in `status`.
async fn fixture(status: &'static str) -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-sequence-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    db.write(move |tx| Ok(seed(tx, status)?))
        .await
        .expect("seed");
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(SystemClock))
            .await
            .expect("store"),
    );
    let deps = Deps {
        db: db.clone(),
        jobs: store.clone(),
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
    Fixture { db, deps, dir }
}

fn step(is_active: bool, flow_id: Option<i64>) -> StepRow {
    StepRow {
        id: "step-1".into(),
        sequence_id: "seq-1".into(),
        order: 1,
        is_active,
        flow_id,
    }
}

fn invalid(reason: &str) -> StepValidation {
    StepValidation {
        valid: false,
        reason: Some(reason.into()),
    }
}

fn now_ms() -> i64 {
    chatbotx_db::repo::now_ms()
}

// ---- StepExecutorService ------------------------------------------------

#[tokio::test]
async fn fetch_step_returns_the_step_when_db_finds_a_record() {
    let f = fixture("running").await;
    let found = fetch_step(&f.db, &STEP.to_string()).await.expect("fetch");
    assert_eq!(
        found,
        Some(StepRow {
            id: STEP.to_string(),
            sequence_id: SEQ.to_string(),
            order: 1,
            is_active: true,
            flow_id: Some(FLOW),
        })
    );
}

#[tokio::test]
async fn fetch_step_returns_undefined_when_db_returns_undefined_not_found() {
    let f = fixture("running").await;
    assert_eq!(fetch_step(&f.db, "9999").await.expect("fetch"), None);
}

#[tokio::test]
async fn fetch_step_queries_by_id_with_flow_relation_included() {
    // Adapted: the relation is the join on `Flow`; a step whose flow is gone reads as no flow.
    let f = fixture("running").await;
    f.exec(
        r#"UPDATE "Flow" SET "currentVersionId" = NULL WHERE "id" = 3601;
              DELETE FROM "FlowVersion" WHERE "flowId" = 3601;
              DELETE FROM "Flow" WHERE "id" = 3601;"#,
    )
    .await;
    let found = fetch_step(&f.db, &STEP.to_string()).await.expect("fetch");
    assert_eq!(found.map(|s| s.flow_id), Some(None));
}

#[test]
fn validate_step_returns_invalid_with_step_not_found_reason_when_step_is_undefined() {
    assert_eq!(validate_step(None), invalid("step_not_found"));
}

#[test]
fn validate_step_returns_invalid_with_step_inactive_reason_when_step_is_inactive() {
    assert_eq!(
        validate_step(Some(&step(false, Some(1)))),
        invalid("step_inactive")
    );
}

#[test]
fn validate_step_returns_invalid_with_flow_not_configured_reason_when_step_flow_is_null() {
    assert_eq!(
        validate_step(Some(&step(true, None))),
        invalid("flow_not_configured")
    );
}

#[test]
fn validate_step_returns_valid_when_all_checks_pass() {
    // Adapted: the valid result carries no step; the caller keeps its own.
    assert_eq!(
        validate_step(Some(&step(true, Some(1)))),
        StepValidation {
            valid: true,
            reason: None
        }
    );
}

#[test]
fn validate_step_checks_is_active_before_flow() {
    assert_eq!(
        validate_step(Some(&step(false, None))),
        invalid("step_inactive")
    );
}

// ---- DispatchProcessorService -------------------------------------------

#[tokio::test]
async fn fetch_dispatch_returns_the_dispatch_when_the_repository_finds_a_record() {
    let f = fixture("pending").await;
    let found = f
        .services()
        .fetch_dispatch(&DISPATCH.to_string(), &WS.to_string())
        .await
        .expect("fetch")
        .expect("found");
    assert_eq!(found.id, DISPATCH.to_string());
    assert_eq!(found.bucket, BUCKET);
    assert_eq!(found.run_at_ms, 1_700_000_000_000);
}

#[tokio::test]
async fn fetch_dispatch_returns_null_when_the_repository_returns_null_not_found() {
    let f = fixture("pending").await;
    let found = f
        .services()
        .fetch_dispatch("9999", &WS.to_string())
        .await
        .expect("fetch");
    assert_eq!(found, None);
}

#[tokio::test]
async fn fetch_dispatch_returns_null_and_logs_error_when_the_repository_throws() {
    // Adapted: a malformed id makes the query fail; the log line is not captured.
    let f = fixture("pending").await;
    let found = f
        .services()
        .fetch_dispatch(&DISPATCH.to_string(), "not-a-number")
        .await
        .expect("no error escapes");
    assert_eq!(found, None);
}

#[tokio::test]
async fn fetch_dispatch_queries_with_id_status_and_workspace_id() {
    let f = fixture("running").await;
    let (id, ws, other) = (DISPATCH.to_string(), WS.to_string(), OTHER_WS.to_string());
    assert!(
        find_dispatch(&f.db, &id, "pending", &ws)
            .await
            .expect("q")
            .is_none()
    );
    assert!(
        find_dispatch(&f.db, &id, "running", &other)
            .await
            .expect("q")
            .is_none()
    );
    let found = find_dispatch(&f.db, &id, "running", &ws)
        .await
        .expect("q")
        .expect("found");
    assert_eq!(found.status, "running");
}

#[test]
fn validate_dispatch_returns_false_when_status_is_running() {
    assert!(!validate_dispatch(Some("running")));
}

#[test]
fn validate_dispatch_returns_false_when_status_is_failed() {
    assert!(!validate_dispatch(Some("failed")));
}

#[test]
fn validate_dispatch_returns_false_when_status_is_completed() {
    assert!(!validate_dispatch(Some("completed")));
}

#[test]
fn validate_dispatch_returns_true_when_dispatch_is_pending() {
    assert!(validate_dispatch(Some("pending")));
}

#[test]
fn validate_dispatch_returns_false_when_dispatch_is_null() {
    assert!(!validate_dispatch(None));
}

#[test]
fn is_dispatch_ready_returns_true_when_run_at_ms_is_in_the_past() {
    let now = now_ms();
    assert!(is_dispatch_ready(now - 10_000, now));
}

#[test]
fn is_dispatch_ready_returns_true_within_the_1_second_tolerance_window() {
    let now = now_ms();
    assert!(is_dispatch_ready(now + 800, now));
}

#[test]
fn is_dispatch_ready_returns_false_beyond_the_tolerance_window() {
    let now = now_ms();
    assert!(!is_dispatch_ready(now + 60_000, now));
}

#[tokio::test]
async fn lock_dispatch_returns_true_when_the_repository_acquires_the_lock() {
    let f = fixture("pending").await;
    let services = f.services();
    let row = services
        .fetch_dispatch(&DISPATCH.to_string(), &WS.to_string())
        .await
        .expect("fetch")
        .expect("found");
    assert!(services.lock_dispatch(&row).await.expect("lock"));
}

#[tokio::test]
async fn lock_dispatch_returns_false_when_the_repository_fails_to_acquire_the_lock() {
    let f = fixture("running").await;
    let claimed = claim_dispatch(&f.db, &DISPATCH.to_string(), &WS.to_string(), "w".into(), 1)
        .await
        .expect("claim");
    assert!(!claimed);
}

#[tokio::test]
async fn lock_dispatch_claims_by_id_and_workspace_with_a_lock_owner() {
    let f = fixture("pending").await;
    let other = claim_dispatch(
        &f.db,
        &DISPATCH.to_string(),
        &OTHER_WS.to_string(),
        "w".into(),
        1,
    )
    .await
    .expect("claim");
    assert!(!other, "another workspace's claim misses");
    let services = f.services();
    let row = services
        .fetch_dispatch(&DISPATCH.to_string(), &WS.to_string())
        .await
        .expect("fetch")
        .expect("found");
    assert!(services.lock_dispatch(&row).await.expect("lock"));
    let (status, _, _, owner) = f.dispatch().await;
    assert_eq!(status, "running");
    assert!(owner.is_some_and(|o| !o.is_empty()));
}

// ---- handleSendSequenceFlow ---------------------------------------------

fn data(overrides: Value) -> Value {
    let mut data = json!({
        "dispatchId": DISPATCH.to_string(),
        "workspaceId": WS.to_string(),
        "stepId": STEP.to_string(),
        "bucket": BUCKET,
        "contactId": CONTACT.to_string(),
        "sequenceId": SEQ.to_string(),
        "enrollmentId": ENROLLMENT.to_string(),
        "metadata": {},
    });
    if let (Some(base), Some(extra)) = (data.as_object_mut(), overrides.as_object()) {
        base.extend(extra.clone());
    }
    data
}

fn job(data: Value, attempts_made: u32) -> JobCtx {
    JobCtx {
        id: 7,
        job_id: "job-1".into(),
        data: json!({ "type": "sendSequenceFlow", "data": data }),
        attempts: Some(3),
        attempts_made,
        ..JobCtx::default()
    }
}

/// The happy-path fixture: a running dispatch and the contact's conversation.
async fn sendable() -> Fixture {
    let f = fixture("running").await;
    f.exec(r#"INSERT INTO "Conversation" ("id", "contactId", "workspaceId") VALUES (3801, 3101, 3001);"#)
        .await;
    f
}

#[tokio::test]
async fn calls_send_flow_direct_then_marks_dispatch_completed() {
    let f = sendable().await;
    let before = now_ms();
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("sent");
    let (status, _, completed_at, _) = f.dispatch().await;
    assert_eq!(status, "completed");
    assert!(completed_at.is_some_and(|at| at >= before));
}

#[tokio::test]
async fn sends_the_flow_of_the_step() {
    // Adapted: `sendFlowDirect({ flowId })` shows as the step's flow being run.
    let f = sendable().await;
    f.exec(r#"UPDATE "SequenceStep" SET "flowId" = 3602 WHERE "id" = 3501;"#)
        .await;
    let err = handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect_err("the broken flow fails");
    assert!(err.to_string().contains("FlowVersion not found"), "{err}");
}

#[tokio::test]
async fn calls_advance_enrollment_with_correct_enrollment_and_step_info() {
    let f = sendable().await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("sent");
    let (_, _, sent_at, _) = f.dispatch().await;
    // The only step was sent: the enrollment completes at step order + 1.
    assert_eq!(f.enrollment().await, (Some("completed".into()), sent_at, 2));
}

#[tokio::test]
async fn removes_the_dispatch_from_the_schedule_bucket() {
    let f = sendable().await;
    assert_eq!(f.scheduled().await, Some(BUCKET));
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("sent");
    assert_eq!(f.scheduled().await, None);
}

#[tokio::test]
async fn re_delivery_skips_send_flow_direct_and_reuses_the_existing_sent_at() {
    let completed_at = 1_735_725_600_000; // 2025-01-01T10:00:00Z
    let f = sendable().await;
    // A send of the broken flow would fail: success proves no send.
    f.exec(
        r#"UPDATE "SequenceStep" SET "flowId" = 3602 WHERE "id" = 3501;
              UPDATE "SequenceDispatch" SET "completedAt" = 1735725600000 WHERE "id" = 3901;"#,
    )
    .await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("not sent again");
    assert_eq!(f.enrollment().await.1, Some(completed_at));
}

#[tokio::test]
async fn dispatch_not_found_returns_early_without_touching_db_scheduler_or_advance_enrollment() {
    let f = fixture("pending").await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("no-op");
    assert_eq!(f.dispatch().await, ("pending".into(), None, None, None));
    assert_eq!(f.enrollment().await, (Some("active".into()), None, 0));
    assert_eq!(f.scheduled().await, Some(BUCKET));
}

async fn inactive_step() -> Fixture {
    let f = fixture("running").await;
    f.exec(r#"UPDATE "SequenceStep" SET "isActive" = 0 WHERE "id" = 3501;"#)
        .await;
    f
}

#[tokio::test]
async fn invalid_step_marks_dispatch_canceled_with_the_validation_reason() {
    let f = inactive_step().await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("canceled");
    let (status, reason, _, _) = f.dispatch().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("canceled", Some("step_inactive"))
    );
}

#[tokio::test]
async fn invalid_step_still_calls_advance_enrollment_so_enrollment_progresses_past_the_dead_step() {
    let f = inactive_step().await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("canceled");
    assert_eq!(f.enrollment().await.0.as_deref(), Some("completed"));
}

#[tokio::test]
async fn invalid_step_removes_dispatch_from_schedule() {
    let f = inactive_step().await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("canceled");
    assert_eq!(f.scheduled().await, None);
}

#[tokio::test]
async fn invalid_step_does_not_call_send_flow_direct() {
    // No conversation: a send would fail, so success proves no send.
    let f = inactive_step().await;
    handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect("canceled");
    assert_eq!(f.dispatch().await.2, None);
}

fn missing_step() -> Value {
    data(json!({ "stepId": "9999" }))
}

#[tokio::test]
async fn step_not_found_marks_dispatch_canceled() {
    let f = fixture("running").await;
    handle_send_sequence_flow(&f.deps, &job(missing_step(), 0))
        .await
        .expect("canceled");
    let (status, reason, _, _) = f.dispatch().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("canceled", Some("step_not_found"))
    );
}

#[tokio::test]
async fn step_not_found_does_not_call_advance_enrollment() {
    let f = fixture("running").await;
    handle_send_sequence_flow(&f.deps, &job(missing_step(), 0))
        .await
        .expect("canceled");
    assert_eq!(f.enrollment().await, (Some("active".into()), None, 0));
}

#[tokio::test]
async fn step_not_found_still_removes_dispatch_from_schedule() {
    let f = fixture("running").await;
    handle_send_sequence_flow(&f.deps, &job(missing_step(), 0))
        .await
        .expect("canceled");
    assert_eq!(f.scheduled().await, None);
}

const SEND_FAILED: &str = "Conversation not found for contact 3101";

#[tokio::test]
async fn final_attempt_failure_marks_dispatch_failed_via_terminal_cleanup() {
    // Adapted: no conversation makes `sendFlowDirect` throw.
    let f = fixture("running").await;
    let err = handle_send_sequence_flow(&f.deps, &job(data(json!({})), 2))
        .await
        .expect_err("fails");
    assert_eq!(err.to_string(), SEND_FAILED);
    let (status, reason, _, _) = f.dispatch().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("failed", Some(SEND_FAILED))
    );
}

#[tokio::test]
async fn final_attempt_failure_removes_dispatch_from_schedule_during_terminal_cleanup() {
    let f = fixture("running").await;
    let _ = handle_send_sequence_flow(&f.deps, &job(data(json!({})), 2)).await;
    assert_eq!(f.scheduled().await, None);
}

#[tokio::test]
async fn final_attempt_failure_rethrows_the_error_so_the_queue_can_retry() {
    let f = fixture("running").await;
    let err = handle_send_sequence_flow(&f.deps, &job(data(json!({})), 2))
        .await
        .expect_err("fails");
    assert!(err.to_string().contains(SEND_FAILED));
}

#[tokio::test]
async fn non_final_attempt_failure_rethrows_without_marking_dispatch_failed() {
    let f = fixture("running").await;
    let err = handle_send_sequence_flow(&f.deps, &job(data(json!({})), 0))
        .await
        .expect_err("fails");
    assert_eq!(err.to_string(), SEND_FAILED);
    assert_eq!(f.dispatch().await.0, "running");
    assert_eq!(f.scheduled().await, Some(BUCKET));
}

#[tokio::test]
async fn non_final_attempt_failure_logs_the_error_with_attempt_and_is_final_attempt_info() {
    // Adapted: the log line is not captured; the attempt math it reports is.
    let first = job(data(json!({})), 0);
    assert_eq!(
        (first.attempts_made + 1, first.is_final_attempt()),
        (1, false)
    );
    assert!(job(data(json!({})), 2).is_final_attempt());
}

#[tokio::test]
async fn the_integration_queue_routes_send_sequence_flow_to_the_handler() {
    let f = sendable().await;
    chatbotx_worker::integration::handle(f.deps.clone(), job(data(json!({})), 0))
        .await
        .expect("handled");
    assert_eq!(f.dispatch().await.0, "completed");
}
