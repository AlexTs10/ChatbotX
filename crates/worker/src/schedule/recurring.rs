//! `enqueue-recurring-broadcasts.ts` and `materialize-recurring-broadcasts.ts`:
//! the per-minute fan-out of due repeating broadcasts and the batch job that
//! turns each due slot into a run.

use chatbotx_business::audit::{AuditActor, AuditService, SYSTEM_ACTOR, with_audit_context};
use chatbotx_business::broadcast::materialize::{self, MaterializeInput};
use chatbotx_business::broadcast::send;
use chatbotx_db::repo::broadcast_recurrence::DueRecurrenceCursor;
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::queues::broadcast::{
    MATERIALIZE_BATCH_SIZE, MATERIALIZE_RECURRING_BROADCASTS_PRIORITY, broadcast_prepare_job_id,
    materialize_batch_job_id,
};
use chatbotx_jobs::{JobError, JobOptions, NewJob, QueueName};
use serde_json::{Value, json};

use super::broadcast::{biz, cron_lock, once, queue_err, start_of_minute};
use crate::{Deps, id_of};

pub const LOCK_KEY: &str = "schedule:enqueue-recurring-broadcasts";
pub const DUE_PAGE_SIZE: i64 = 5000;
pub const MAX_RECURRENCES_PER_TICK: usize = 200_000;
/// Batches pending (prioritized) or backing off (delayed) beyond which a tick adds none.
pub const MAX_PENDING_BATCHES: u64 = (MAX_RECURRENCES_PER_TICK / MATERIALIZE_BATCH_SIZE) as u64;

/// `{ scanned, enqueued, skipped }`.
fn tick_result(scanned: usize, enqueued: usize, skipped: bool) -> Value {
    json!({ "scanned": scanned, "enqueued": enqueued, "skipped": skipped })
}

/// `enqueueRecurringBroadcasts`: a held lock (the previous tick) skips the tick.
pub async fn enqueue_recurring_broadcasts(deps: &Deps) -> Result<Value, JobError> {
    enqueue_recurring_broadcasts_with(deps, DUE_PAGE_SIZE, MAX_RECURRENCES_PER_TICK).await
}

/// [`enqueue_recurring_broadcasts`] with the page size and per-tick cap given.
pub async fn enqueue_recurring_broadcasts_with(
    deps: &Deps,
    page_size: i64,
    max_per_tick: usize,
) -> Result<Value, JobError> {
    // ponytail: process-local lock (one server process), as the other crons here.
    let Ok(_guard) = cron_lock(deps, LOCK_KEY).try_lock() else {
        tracing::warn!("enqueueRecurringBroadcasts: previous tick still running, skipped");
        return Ok(tick_result(0, 0, true));
    };
    let pending = deps
        .jobs
        .backlog_count(QueueName::Schedule)
        .await
        .map_err(queue_err)?;
    if pending >= MAX_PENDING_BATCHES {
        let oldest = materialize::find_oldest_due_age_ms(&deps.db)
            .await
            .map_err(biz)?;
        tracing::warn!(
            pending_batches = pending,
            oldest_due_age_ms = oldest,
            "schedule queue is backlogged; tick skipped"
        );
        return Ok(tick_result(0, 0, true));
    }

    let due_at_ms = start_of_minute(now_ms());
    let mut cursor = None;
    let (mut scanned, mut enqueued) = (0, 0);
    let mut page = 0u32;
    while enqueued < max_per_tick {
        let due = materialize::list_due(&deps.db, page_size, cursor)
            .await
            .map_err(biz)?;
        let Some(last) = due.last() else { break };
        scanned += due.len();
        let jobs = due
            .chunks(MATERIALIZE_BATCH_SIZE)
            .enumerate()
            .map(|(index, batch)| {
                let items: Vec<Value> = batch
                    .iter()
                    .map(|s| {
                        json!({
                            "recurrenceId": s.id.to_string(),
                            "workspaceId": s.workspace_id.to_string(),
                            "occurrenceAtMs": s.next_run_at,
                        })
                    })
                    .collect();
                NewJob {
                    name: "materializeRecurringBroadcasts".into(),
                    data: json!({
                        "type": "materializeRecurringBroadcasts",
                        "data": { "items": items },
                    }),
                    opts: JobOptions {
                        priority: Some(MATERIALIZE_RECURRING_BROADCASTS_PRIORITY),
                        ..once(materialize_batch_job_id(due_at_ms, page, index as u32))
                    },
                }
            })
            .collect();
        deps.jobs
            .add_bulk(QueueName::Schedule, jobs)
            .await
            .map_err(queue_err)?;
        enqueued += due.len();
        if (due.len() as i64) < page_size {
            break;
        }
        cursor = Some(DueRecurrenceCursor {
            next_run_at: last.next_run_at,
            id: last.id,
        });
        page += 1;
    }

    // Grows when the schedule worker is unhealthy: the cursor keeps every slot due.
    let oldest = materialize::find_oldest_due_age_ms(&deps.db)
        .await
        .map_err(biz)?;
    tracing::info!(
        scanned,
        enqueued,
        oldest_due_age_ms = oldest,
        "enqueued recurring broadcasts"
    );
    Ok(tick_result(scanned, enqueued, false))
}

/// `materializeRecurringBroadcasts`: each series in its own transaction; the
/// job fails afterwards if any did, so it is retried (each slot is idempotent).
pub async fn materialize_recurring_broadcasts(
    deps: &Deps,
    items: &[Value],
) -> Result<Value, JobError> {
    let mut failures = 0;
    // ponytail: sequential, not `MATERIALIZE_ITEM_CONCURRENCY` at a time: SQLite
    // has one writer, so parallel transactions would only queue.
    for item in items {
        if let Err(err) = materialize_one(deps, item).await {
            failures += 1;
            tracing::error!(
                err = %err,
                recurrence_id = item["recurrenceId"].as_str().unwrap_or_default(),
                "Failed to materialize a recurring broadcast slot"
            );
        }
    }
    if failures > 0 {
        return Err(JobError::retryable(format!(
            "{failures} of {} recurring broadcast slots failed",
            items.len()
        )));
    }
    Ok(Value::Null)
}

async fn materialize_one(deps: &Deps, item: &Value) -> Result<(), String> {
    let (Some(recurrence_id), Some(workspace_id), Some(occurrence_at)) = (
        id_of(&item["recurrenceId"]),
        id_of(&item["workspaceId"]),
        item["occurrenceAtMs"].as_i64(),
    ) else {
        return Err("bad materialize item".into());
    };
    let input = MaterializeInput {
        recurrence_id,
        workspace_id,
        occurrence_at,
    };
    let actor = AuditActor {
        user_id: Some(SYSTEM_ACTOR.into()),
        workspace_id: Some(workspace_id.to_string()),
        source: Some("broadcastRecurrence:materialize".into()),
        ..AuditActor::default()
    };
    with_audit_context(actor, async {
        // A blocked owner's slot is skipped (the series moves on) rather than left
        // due, which would keep it at the head of every scan.
        if send::is_blocked_workspace(&deps.db, workspace_id)
            .await
            .map_err(|e| e.to_string())?
        {
            return materialize::skip_occurrence(&deps.db, input, "blockedOwner")
                .await
                .map_err(|e| e.to_string());
        }
        let audit = AuditService::new(deps.jobs.clone());
        let result = materialize::materialize_occurrence(&deps.db, &audit, input)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(id) = result.created_broadcast_id {
            enqueue_prepare(deps, id).await;
        }
        Ok(())
    })
    .await
}

/// Prepares the new run now instead of at the next `enqueueBroadcast` tick (the
/// fallback); the same job id, so a run is never prepared twice. Best effort.
async fn enqueue_prepare(deps: &Deps, broadcast_id: i64) {
    let id = broadcast_id.to_string();
    if let Err(err) = deps
        .jobs
        .add(
            QueueName::Schedule,
            "prepareBroadcast",
            json!({ "type": "prepareBroadcast", "data": { "broadcastId": id } }),
            once(broadcast_prepare_job_id(&id)),
        )
        .await
    {
        tracing::warn!(
            err = %err,
            broadcast_id,
            "Could not enqueue prepare for a new run; the next schedule tick will"
        );
    }
}
