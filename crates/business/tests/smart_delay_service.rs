//! Ports of `packages/business/__tests__/smart-delay-service.test.ts` and
//! `workspace-lifecycle.smart-delay-cleanup.test.ts` (Q.D5 slice 7).
//! Adapted: the mocked query builder becomes a migrated SQLite database, so
//! each "called with" check reads as the rows the call leaves; the queue is
//! a recording enqueuer.

mod common;

use std::sync::{Arc, Mutex};

use chatbotx_business::smart_delay::{
    self as service, RowDeadline, SMART_DELAY_CANCEL_BATCH_SIZE, StepCount, job_id,
};
use chatbotx_db::Db;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::models::ContactOnSmartDelay as Row;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use common::{OTHER_WS, TestDb, WS, contact, conversation, setup, two_workspaces};
use serde_json::{Value, json};

/// 2026-07-16T00:00:00.000Z.
const T0: i64 = 1_784_160_000_000;
const MINUTE: i64 = 60_000;
const CONVERSATION: i64 = 901;
const OTHER_CONVERSATION: i64 = 902;

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        contact(tx, 102, OTHER_WS)?;
        conversation(tx, CONVERSATION, WS, 101)?;
        conversation(tx, OTHER_CONVERSATION, OTHER_WS, 102)?;
        tx.execute(
            r#"UPDATE "Conversation" SET "additionalAttributes" = ?1 WHERE "id" = ?2"#,
            params![
                json!({ "challenge": { "data": { "challengeId": "challenge-1" } } }).to_string(),
                CONVERSATION
            ],
        )?;
        Ok(())
    })
    .await;
    db
}

/// `smartDelayRow` of the original (`followUp`, pending, due at T0 + 1 min).
fn row(id: i64) -> Row {
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
        r#type: "followUp".into(),
        created_at: T0,
        trigger_at: T0 + MINUTE,
        status: "pending".into(),
        appointment_id: None,
    }
}

fn with(id: i64, kind: &str, status: &str, trigger_at: i64) -> Row {
    Row {
        r#type: kind.into(),
        status: status.into(),
        trigger_at,
        ..row(id)
    }
}

async fn insert(db: &Db, rows: Vec<Row>) {
    for r in rows {
        service::create(db, r).await.expect("create");
    }
}

async fn status(db: &Db, id: i64) -> String {
    service::find_by_id(db, id)
        .await
        .expect("find")
        .expect("row")
        .status
}

fn timeout_metadata() -> Value {
    json!({ "type": "stepInputResolution", "stepId": "step-1", "challengeId": "challenge-1", "reason": "timeout" })
}

fn timeout_row(id: i64) -> Row {
    Row {
        r#type: "stepInputTimeout".into(),
        metadata: Some(Json(timeout_metadata())),
        ..row(id)
    }
}

#[tokio::test]
async fn create_inserts_a_smart_delay_row() {
    let db = db().await;
    service::create(&db, row(1)).await.expect("create");
    assert_eq!(
        service::find_by_id(&db, 1).await.expect("find"),
        Some(row(1))
    );
}

#[tokio::test]
async fn upsert_follow_up_resets_an_active_follow_up_row_through_the_partial_unique_key() {
    let db = db().await;
    insert(&db, vec![with(1, "followUp", "scheduled", T0 + MINUTE)]).await;
    let update = Row {
        node_id: Some("other-node".into()),
        trigger_at: T0 + 2 * MINUTE,
        ..row(2)
    };
    let upserted = service::upsert_follow_up(&db, update)
        .await
        .expect("upsert");
    assert_eq!(
        upserted.id, 1,
        "the active row is restarted, not duplicated"
    );
    assert_eq!(upserted.status, "pending");
    assert_eq!(upserted.trigger_at, T0 + 2 * MINUTE);
    assert_eq!(upserted.node_id.as_deref(), Some("other-node"));
    assert_eq!(upserted.conversation_id, CONVERSATION);
    assert_eq!(upserted.flow_version_id, Some(71));
    assert_eq!(service::find_by_id(&db, 2).await.expect("find"), None);
}

#[tokio::test]
async fn upsert_step_input_timeout_keys_the_conflict_on_one_active_timer_per_conversation_and_restarts_it()
 {
    let db = db().await;
    insert(
        &db,
        vec![Row {
            status: "scheduled".into(),
            ..timeout_row(10)
        }],
    )
    .await;
    let restart = Row {
        flow_id: 62,
        contact_inbox_id: 82,
        node_id: Some("node-2".into()),
        step_id: Some("step-2".into()),
        trigger_at: T0 + 3 * MINUTE,
        ..timeout_row(11)
    };
    let before = chatbotx_db::repo::now_ms();
    let armed = service::upsert_step_input_timeout(&db, restart)
        .await
        .expect("upsert")
        .expect("armed");
    assert_eq!(armed.id, 10);
    assert_eq!(
        (
            armed.flow_id,
            armed.node_id.as_deref(),
            armed.step_id.as_deref()
        ),
        (62, Some("node-2"), Some("step-2"))
    );
    assert_eq!(armed.metadata.as_deref(), Some(&timeout_metadata()));
    assert_eq!(
        (armed.status.as_str(), armed.trigger_at),
        ("pending", T0 + 3 * MINUTE)
    );
    assert!(armed.created_at >= before);
}

#[tokio::test]
async fn upsert_step_input_timeout_only_treats_non_terminal_step_input_timeout_rows_as_the_active_key()
 {
    let db = db().await;
    insert(
        &db,
        vec![
            Row {
                status: "completed".into(),
                ..timeout_row(10)
            },
            with(12, "waitNode", "pending", T0),
        ],
    )
    .await;
    let armed = service::upsert_step_input_timeout(&db, timeout_row(11))
        .await
        .expect("upsert")
        .expect("armed");
    assert_eq!(
        armed.id, 11,
        "a terminal timer and other types do not conflict"
    );
    assert_eq!(status(&db, 10).await, "completed");
}

#[tokio::test]
async fn upsert_step_input_timeout_arms_nothing_when_its_pending_challenge_was_replaced() {
    let db = db().await;
    let replaced = Row {
        metadata: Some(Json(
            json!({ "type": "stepInputResolution", "challengeId": "older-challenge" }),
        )),
        ..timeout_row(11)
    };
    assert_eq!(
        service::upsert_step_input_timeout(&db, replaced)
            .await
            .expect("upsert"),
        None
    );
    assert_eq!(service::find_by_id(&db, 11).await.expect("find"), None);
}

#[tokio::test]
async fn upsert_step_input_timeout_arms_nothing_when_the_conversation_is_gone() {
    let db = db().await;
    let gone = Row {
        conversation_id: 999,
        ..timeout_row(11)
    };
    assert_eq!(
        service::upsert_step_input_timeout(&db, gone)
            .await
            .expect("upsert"),
        None
    );
}

#[tokio::test]
async fn upsert_step_input_timeout_fails_loudly_when_the_upsert_returns_no_row() {
    // Adapted: a trigger that ignores the insert stands in for an empty RETURNING.
    let db = db().await;
    setup(&db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "ignore_insert" BEFORE INSERT ON "ContactOnSmartDelay"
               BEGIN SELECT RAISE(IGNORE); END;"#,
        )?;
        Ok(())
    })
    .await;
    let err = service::upsert_step_input_timeout(&db, timeout_row(11))
        .await
        .expect_err("fails");
    assert!(
        err.message()
            .contains("Failed to upsert step input timeout smart delay"),
        "{}",
        err.message()
    );
}

#[tokio::test]
async fn find_by_id_returns_the_matching_row_or_null() {
    let db = db().await;
    insert(&db, vec![row(1)]).await;
    assert_eq!(
        service::find_by_id(&db, 1).await.expect("find"),
        Some(row(1))
    );
    assert_eq!(service::find_by_id(&db, 404).await.expect("find"), None);
}

#[tokio::test]
async fn count_by_flow_step_returns_grouped_status_counts_by_step() {
    let db = db().await;
    let mut rows = Vec::new();
    let mut id = 100;
    for (step, state, n) in [
        (Some("step-1"), "pending", 2),
        (Some("step-1"), "scheduled", 3),
        (Some("step-2"), "completed", 4),
        (None, "completed", 9),
    ] {
        for _ in 0..n {
            id += 1;
            rows.push(Row {
                step_id: step.map(str::to_owned),
                ..with(id, "waitNode", state, T0)
            });
        }
    }
    // Another flow and another workspace are not counted.
    rows.push(Row {
        flow_id: 62,
        ..with(200, "waitNode", "pending", T0)
    });
    rows.push(Row {
        workspace_id: OTHER_WS,
        conversation_id: OTHER_CONVERSATION,
        ..with(201, "waitNode", "pending", T0)
    });
    insert(&db, rows).await;
    let count = |step: &str, status: &str, total| StepCount {
        step_id: step.into(),
        status: status.into(),
        total,
    };
    assert_eq!(
        service::count_by_flow_step(&db, WS, 61)
            .await
            .expect("count"),
        vec![
            count("step-1", "pending", 2),
            count("step-1", "scheduled", 3),
            count("step-2", "completed", 4)
        ]
    );
}

#[tokio::test]
async fn mark_completed_and_mark_canceled_update_the_row_status() {
    let db = db().await;
    insert(
        &db,
        vec![
            with(1, "waitNode", "pending", T0),
            with(2, "waitNode", "pending", T0),
        ],
    )
    .await;
    service::mark_completed(&db, 1).await.expect("mark");
    service::mark_canceled(&db, 2).await.expect("mark");
    assert_eq!(
        (status(&db, 1).await, status(&db, 2).await),
        ("completed".into(), "canceled".into())
    );
}

#[tokio::test]
async fn claim_due_rows_claims_a_bounded_batch_up_to_the_window() {
    let db = db().await;
    insert(
        &db,
        vec![
            with(1, "waitNode", "pending", T0 + 2 * MINUTE),
            with(2, "waitNode", "pending", T0 + MINUTE),
            with(3, "waitNode", "pending", T0 + 10 * MINUTE),
            with(4, "waitNode", "completed", T0),
        ],
    )
    .await;
    let window = T0 + 5 * MINUTE;
    let first = service::claim_due_rows(&db, window, 1)
        .await
        .expect("claim");
    assert_eq!(
        first
            .iter()
            .map(|r| (r.id, r.status.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "scheduled")]
    );
    let rest = service::claim_due_rows(&db, window, 500)
        .await
        .expect("claim");
    assert_eq!(rest.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
    assert_eq!(status(&db, 3).await, "pending", "beyond the window");
    assert_eq!(status(&db, 4).await, "completed");
}

#[tokio::test]
async fn claim_for_run_returns_true_only_when_the_scheduled_row_is_claimed() {
    let db = db().await;
    insert(
        &db,
        vec![
            with(1, "waitNode", "scheduled", T0),
            with(2, "waitNode", "pending", T0),
        ],
    )
    .await;
    assert!(
        service::claim_for_run(&db, 1, "completed", None)
            .await
            .expect("claim")
    );
    assert_eq!(status(&db, 1).await, "completed");
    assert!(
        !service::claim_for_run(&db, 1, "canceled", None)
            .await
            .expect("claim")
    );
    assert!(
        !service::claim_for_run(&db, 2, "canceled", None)
            .await
            .expect("claim")
    );
}

#[tokio::test]
async fn claim_for_run_ties_the_claim_to_the_deadline_the_caller_observed_so_a_re_armed_timer_is_not_consumed()
 {
    let db = db().await;
    insert(
        &db,
        vec![with(1, "stepInputTimeout", "scheduled", T0 + MINUTE)],
    )
    .await;
    assert!(
        !service::claim_for_run(&db, 1, "completed", Some(T0))
            .await
            .expect("claim")
    );
    assert_eq!(status(&db, 1).await, "scheduled");
    assert!(
        service::claim_for_run(&db, 1, "completed", Some(T0 + MINUTE))
            .await
            .expect("claim")
    );
}

#[tokio::test]
async fn claim_for_run_without_an_observed_deadline_does_not_filter_on_it() {
    let db = db().await;
    insert(&db, vec![with(1, "waitNode", "scheduled", T0 + 7 * MINUTE)]).await;
    assert!(
        service::claim_for_run(&db, 1, "completed", None)
            .await
            .expect("claim")
    );
}

#[tokio::test]
async fn requeue_claimed_run_only_restores_rows_claimed_as_completed() {
    let db = db().await;
    insert(
        &db,
        vec![
            with(1, "waitNode", "completed", T0),
            with(2, "waitNode", "canceled", T0),
        ],
    )
    .await;
    assert!(service::requeue_claimed_run(&db, 1).await.expect("requeue"));
    assert_eq!(status(&db, 1).await, "scheduled");
    assert!(!service::requeue_claimed_run(&db, 2).await.expect("requeue"));
    assert_eq!(status(&db, 2).await, "canceled");
}

#[tokio::test]
async fn list_stuck_scheduled_returns_a_bounded_batch_of_overdue_scheduled_rows() {
    let db = db().await;
    let older_than = T0 + 10 * MINUTE;
    insert(
        &db,
        vec![
            with(2, "waitNode", "scheduled", T0 + MINUTE),
            with(1, "waitNode", "scheduled", T0),
            with(3, "waitNode", "scheduled", T0 + 11 * MINUTE),
            with(4, "waitNode", "pending", T0),
        ],
    )
    .await;
    let deadline = |id, trigger_at| RowDeadline { id, trigger_at };
    assert_eq!(
        service::list_stuck_scheduled(&db, older_than, 500)
            .await
            .expect("list"),
        vec![deadline(1, T0), deadline(2, T0 + MINUTE)]
    );
    assert_eq!(
        service::list_stuck_scheduled(&db, older_than, 1)
            .await
            .expect("list"),
        vec![deadline(1, T0)]
    );
}

#[tokio::test]
async fn reset_to_pending_only_resets_rows_still_scheduled_and_reports_the_real_count() {
    let db = db().await;
    let before = T0 + 5 * MINUTE;
    insert(
        &db,
        vec![
            with(1, "waitNode", "scheduled", T0),
            // Completed by its resume between the read and this write.
            with(2, "waitNode", "completed", T0),
            // Rescheduled to a fresh deadline in the same window.
            with(3, "waitNode", "scheduled", T0 + 6 * MINUTE),
        ],
    )
    .await;
    assert_eq!(
        service::reset_to_pending(&db, vec![1, 2, 3], Some(before))
            .await
            .expect("reset"),
        1
    );
    assert_eq!(
        (
            status(&db, 1).await,
            status(&db, 2).await,
            status(&db, 3).await
        ),
        ("pending".into(), "completed".into(), "scheduled".into())
    );
}

#[tokio::test]
async fn reset_to_pending_skips_empty_batches() {
    let db = db().await;
    assert_eq!(
        service::reset_to_pending(&db, vec![], None)
            .await
            .expect("reset"),
        0
    );
}

#[tokio::test]
async fn cancel_active_for_workspace_cancels_a_bounded_batch_of_the_workspace_s_live_rows() {
    let db = db().await;
    insert(
        &db,
        vec![
            with(1, "waitNode", "pending", T0 + MINUTE),
            with(2, "waitNode", "scheduled", T0 + 2 * MINUTE),
            with(3, "waitNode", "completed", T0),
            Row {
                workspace_id: OTHER_WS,
                conversation_id: OTHER_CONVERSATION,
                ..with(4, "waitNode", "pending", T0)
            },
        ],
    )
    .await;
    let one = db
        .write(|tx| {
            service::cancel_active_for_workspace_in(tx, WS, 1)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .expect("cancel");
    assert_eq!(one.len(), 1, "bounded by the limit");
    let rest = db
        .write(|tx| {
            service::cancel_active_for_workspace_in(tx, WS, 500)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .expect("cancel");
    let mut canceled: Vec<RowDeadline> = one.into_iter().chain(rest).collect();
    canceled.sort_by_key(|r| r.id);
    assert_eq!(
        canceled,
        vec![
            RowDeadline {
                id: 1,
                trigger_at: T0 + MINUTE
            },
            RowDeadline {
                id: 2,
                trigger_at: T0 + 2 * MINUTE
            }
        ]
    );
    assert_eq!(status(&db, 3).await, "completed");
    assert_eq!(status(&db, 4).await, "pending");
}

// ---- cancelSmartDelaysForWorkspace -------------------------------------

#[derive(Default)]
struct Queue {
    removed: Mutex<Vec<String>>,
    fail: bool,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        _: QueueName,
        _: &str,
        _: Value,
        _: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }

    fn remove_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            assert_eq!(queue, QueueName::Integration);
            self.removed.lock().unwrap().push(job_id.to_owned());
            if self.fail {
                Err(JobsError::Other("redis down".into()))
            } else {
                Ok(true)
            }
        })
    }
}

fn deadline() -> i64 {
    chatbotx_db::repo::now_ms() + MINUTE
}

#[tokio::test]
async fn cancels_nothing_and_touches_no_queue_when_the_workspace_has_no_live_rows() {
    let db = db().await;
    let queue = Arc::new(Queue::default());
    assert_eq!(
        service::cancel_smart_delays_for_workspace(&db, queue.as_ref(), WS, deadline())
            .await
            .expect("cancel"),
        0
    );
    assert!(queue.removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn removes_the_deterministic_delayed_job_for_every_canceled_row() {
    let db = db().await;
    insert(&db, vec![with(1, "waitNode", "scheduled", T0 + MINUTE)]).await;
    let queue = Arc::new(Queue::default());
    assert_eq!(
        service::cancel_smart_delays_for_workspace(&db, queue.as_ref(), WS, deadline())
            .await
            .expect("cancel"),
        1
    );
    assert_eq!(
        *queue.removed.lock().unwrap(),
        vec![format!("smart-delay-1-{}", T0 + MINUTE)]
    );
    assert_eq!(
        job_id(1, T0 + MINUTE),
        format!("smart-delay-1-{}", T0 + MINUTE)
    );
}

#[tokio::test]
async fn keeps_draining_while_batches_come_back_full() {
    let db = db().await;
    let n = SMART_DELAY_CANCEL_BATCH_SIZE + 3;
    insert(
        &db,
        (1..=n)
            .map(|id| with(id, "waitNode", "pending", T0 + id))
            .collect(),
    )
    .await;
    let queue = Arc::new(Queue::default());
    assert_eq!(
        service::cancel_smart_delays_for_workspace(&db, queue.as_ref(), WS, deadline())
            .await
            .expect("cancel"),
        n as usize
    );
    assert_eq!(queue.removed.lock().unwrap().len(), n as usize);
}

#[tokio::test]
async fn still_reports_the_cancellation_when_removing_the_delayed_job_fails() {
    let db = db().await;
    insert(&db, vec![with(1, "waitNode", "scheduled", T0 + MINUTE)]).await;
    let queue = Arc::new(Queue {
        fail: true,
        ..Queue::default()
    });
    assert_eq!(
        service::cancel_smart_delays_for_workspace(&db, queue.as_ref(), WS, deadline())
            .await
            .expect("cancel"),
        1
    );
    assert_eq!(status(&db, 1).await, "canceled");
}
