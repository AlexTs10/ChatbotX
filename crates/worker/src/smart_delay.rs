//! `integration/handlers/smart-delay.ts` (the resume jobs of a smart delay)
//! and `schedule/handlers/scan-smart-delay.ts` (the `scanSmartDelay` cron):
//! a delay due within `ENQUEUE_DELAY_MS` is a delayed `integration` job;
//! later ones wait in `"ContactOnSmartDelay"` until the scanner claims them.

use chatbotx_business::smart_delay::{self as service, job_id};
use chatbotx_core::flow_config::steps::ENQUEUE_DELAY_MS;
use chatbotx_db::repo::models::ContactOnSmartDelay as Row;
use chatbotx_jobs::queues::integration::flow_continuation_job_options;
use chatbotx_jobs::{JobError, JobOptions, NewJob, QueueName};
use serde_json::{Map, Value, json};

use crate::Deps;

/// A resume job (`SmartDelayJobSpec`).
#[derive(Debug, Clone, PartialEq)]
pub struct JobSpec {
    pub name: &'static str,
    pub data: Value,
    /// The resumed run's tier (`flowContinuationJobOptions`).
    pub opts: JobOptions,
}

/// `SmartDelayResumeJobExtras`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResumeExtras {
    pub metadata: Option<Value>,
    /// `"inbox"`.
    pub send_from: Option<String>,
    pub appointment_id: Option<i64>,
}

/// `metadataSchema.safeParse`: the discriminated union's required fields per
/// `type`. Unknown keys are kept (zod strips them; nothing reads them).
// ponytail: datetime fields are checked as strings, not as ISO formats.
fn parse_metadata(value: Option<&Value>) -> Option<Value> {
    let m = value?.as_object()?;
    let text = |key: &str| m.get(key).is_some_and(Value::is_string);
    let optional = |key: &str| m.get(key).is_none_or(Value::is_string);
    let one_of = |key: &str, allowed: &[&str]| {
        m.get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| allowed.contains(&v))
    };
    let required: &[&str] = match m.get("type")?.as_str()? {
        "broadcast" => &["broadcastId", "contactInboxId"],
        "sequenceSchedule" => &[
            "sequenceStepId",
            "sequenceId",
            "dispatchId",
            "contactInboxId",
        ],
        "commentAutomation" if one_of("replyChannel", &["public", "private"]) => {
            &["commentAutomationId", "commentId"]
        }
        "updateStatus" => &[],
        "appointmentWebviewSelection" if optional("appointmentId") => {
            &["stepId", "selectedStartAt"]
        }
        "appointmentAvailabilityRangeSelection" => &["stepId", "startDate", "endDate"],
        "appointmentAvailabilityRangeSkipped" => &["stepId"],
        "getUserDataWebviewSelection" => &["stepId", "challengeId", "selectedValue"],
        "stepInputResolution" if one_of("reason", &["timeout", "user_skipped"]) => {
            &["stepId", "challengeId"]
        }
        _ => return None,
    };
    (optional("stepId") && optional("contactInboxId") && required.iter().all(|k| text(k)))
        .then(|| value.cloned())
        .flatten()
}

fn resume_metadata(row: &Row, extras: Option<&ResumeExtras>) -> Option<Value> {
    extras
        .and_then(|e| e.metadata.clone())
        .or_else(|| parse_metadata(row.metadata.as_deref()))
}

/// `buildSendFlowResumeJob`: a `sendFlow` from the row's node.
pub fn build_send_flow_resume_job(row: &Row, extras: Option<&ResumeExtras>) -> JobSpec {
    let metadata = resume_metadata(row, extras);
    let appointment_id = extras.and_then(|e| e.appointment_id).or(row.appointment_id);
    let mut data = Map::new();
    data.insert(
        "conversationId".into(),
        json!(row.conversation_id.to_string()),
    );
    data.insert(
        "contactInboxId".into(),
        json!(row.contact_inbox_id.to_string()),
    );
    data.insert("flowId".into(), json!(row.flow_id.to_string()));
    if let Some(version) = row.flow_version_id {
        data.insert("flowVersionId".into(), json!(version.to_string()));
    }
    if let Some(node) = &row.node_id {
        data.insert("nodeId".into(), json!(node));
    }
    if let Some(metadata) = &metadata {
        data.insert("metadata".into(), metadata.clone());
    }
    if let Some(send_from) = extras.and_then(|e| e.send_from.as_deref()) {
        data.insert("sendFrom".into(), json!(send_from));
    }
    if let Some(appointment) = appointment_id {
        data.insert("appointmentId".into(), json!(appointment.to_string()));
    }
    JobSpec {
        name: "sendFlow",
        opts: flow_continuation_job_options(metadata.as_ref()),
        data: json!({ "type": "sendFlow", "data": data }),
    }
}

/// `smartDelayResumeJobFactories[row.type]`: the dedicated resume job,
/// carrying only the row id.
pub fn resume_job(row: &Row, extras: Option<&ResumeExtras>) -> JobSpec {
    let name = match row.r#type.as_str() {
        "followUp" => "resumeFollowUp",
        "quickReplyFollowUp" => "resumeQuickReplyFollowUp",
        "stepInputTimeout" => "resumeStepInputTimeout",
        _ => "resumeWait",
    };
    JobSpec {
        name,
        opts: flow_continuation_job_options(resume_metadata(row, extras).as_ref()),
        data: json!({ "type": name, "data": { "smartDelayId": row.id.to_string() } }),
    }
}

/// `scheduleSmartDelayResume`: persists `row` (the type's insert / upsert),
/// then enqueues its resume job when it is due within `ENQUEUE_DELAY_MS`. A
/// failed enqueue returns the row to `pending` for the scanner; never fails
/// after the row is persisted.
pub async fn schedule_smart_delay_resume(
    deps: &Deps,
    row: Row,
    extras: ResumeExtras,
) -> Result<(), JobError> {
    let (conversation_id, kind) = (row.conversation_id, row.r#type.clone());
    let persisted = match kind.as_str() {
        "followUp" => Some(service::upsert_follow_up(&deps.db, row).await),
        "quickReplyFollowUp" => Some(service::upsert_quick_reply_follow_up(&deps.db, row).await),
        "stepInputTimeout" => service::upsert_step_input_timeout(&deps.db, row)
            .await
            .transpose(),
        _ => {
            let created = row.clone();
            Some(service::create(&deps.db, row).await.map(|()| created))
        }
    };
    let Some(persisted) = persisted.transpose().map_err(crate::db_fail)? else {
        tracing::info!(
            conversation_id,
            r#type = %kind,
            "Smart delay not armed: superseded before it was persisted"
        );
        return Ok(());
    };
    let diff_ms = persisted.trigger_at - chatbotx_db::repo::now_ms();
    if diff_ms > ENQUEUE_DELAY_MS as i64 {
        return Ok(());
    }
    let enqueued = async {
        service::mark_scheduled(&deps.db, persisted.id)
            .await
            .map_err(|e| e.to_string())?;
        let job = resume_job(&persisted, Some(&extras));
        deps.jobs
            .add(
                QueueName::Integration,
                job.name,
                job.data,
                JobOptions {
                    delay_ms: Some(diff_ms.max(0) as u64),
                    job_id: Some(job_id(persisted.id, persisted.trigger_at)),
                    ..job.opts
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(err) = enqueued {
        if let Err(reset) = service::reset_to_pending(&deps.db, vec![persisted.id], None).await {
            tracing::warn!(err = %reset, row_id = persisted.id, "Failed to reset immediate smart delay after enqueue failure");
        }
        tracing::warn!(err = %err, row_id = persisted.id, "Failed to immediately enqueue smart delay; scanner will pick it up");
    }
    Ok(())
}

/// `requeueClaimedRunOrLog`: re-opens a claimed row after its resume failed;
/// never fails.
pub async fn requeue_claimed_run_or_log(deps: &Deps, smart_delay_id: i64, message: &str) {
    match service::requeue_claimed_run(&deps.db, smart_delay_id).await {
        Ok(true) => {}
        Ok(false) => tracing::error!(smart_delay_id, "{message}"),
        Err(err) => tracing::error!(err = %err, smart_delay_id, "{message}"),
    }
}

// ---- scanSmartDelay ------------------------------------------------------

/// The scanner's batch bounds (`SCAN_BATCH_SIZE`, `MAX_SWEEP_BATCHES_PER_RUN`,
/// `MAX_CLAIM_BATCHES_PER_RUN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    pub batch_size: i64,
    pub max_sweep_batches: usize,
    pub max_claim_batches: usize,
}

impl Default for ScanLimits {
    fn default() -> ScanLimits {
        ScanLimits {
            batch_size: 500,
            max_sweep_batches: 20,
            max_claim_batches: 200,
        }
    }
}

/// `STUCK_SCHEDULED_GRACE_MS`.
pub const STUCK_SCHEDULED_GRACE_MS: i64 = 10 * 60 * 1000;

/// Scheduled rows whose wake-up never ran: their stale job is removed first
/// (its id would make the re-add a no-op), then the row goes back to
/// `pending` (only while still `scheduled`).
async fn sweep_stuck_scheduled(
    deps: &Deps,
    older_than: i64,
    limits: ScanLimits,
) -> Result<(), String> {
    let (mut swept, mut failed_removals, mut last_batch) = (0, 0, 0);
    for _ in 0..limits.max_sweep_batches {
        let stuck = service::list_stuck_scheduled(&deps.db, older_than, limits.batch_size)
            .await
            .map_err(|e| e.to_string())?;
        last_batch = stuck.len();
        if stuck.is_empty() {
            break;
        }
        for row in &stuck {
            // An active job is not removed (`Ok(false)`): the CAS below decides.
            if deps
                .jobs
                .remove_job(QueueName::Integration, &job_id(row.id, row.trigger_at))
                .await
                .is_err()
            {
                failed_removals += 1;
            }
        }
        swept += service::reset_to_pending(
            &deps.db,
            stuck.iter().map(|r| r.id).collect(),
            Some(older_than),
        )
        .await
        .map_err(|e| e.to_string())?;
        if (stuck.len() as i64) < limits.batch_size {
            break;
        }
    }
    if swept > 0 {
        tracing::warn!(
            count = swept,
            failed_removals,
            "Reset stuck scheduled smart delay rows to pending"
        );
    }
    if last_batch as i64 == limits.batch_size {
        tracing::warn!(
            max_batches = limits.max_sweep_batches,
            batch_size = limits.batch_size,
            "Smart delay sweep hit its per-run batch cap; backlog remains for the next tick"
        );
    }
    Ok(())
}

/// Rows without a node end here (`completed`); the others become their
/// resume jobs in one `addBulk`, or go back to `pending` if it fails.
async fn enqueue_claimed_batch(deps: &Deps, claimed: &[Row], now: i64) -> usize {
    let (terminal, enqueueable): (Vec<&Row>, Vec<&Row>) =
        claimed.iter().partition(|r| r.node_id.is_none());
    if !terminal.is_empty() {
        let mut failed = 0;
        for row in &terminal {
            if service::claim_for_run(&deps.db, row.id, "completed", None)
                .await
                .is_err()
            {
                failed += 1;
            }
        }
        if failed > 0 {
            tracing::error!(
                count = failed,
                "Failed to complete some terminal smart delay rows; sweep will retry them"
            );
        }
        let ids: Vec<i64> = terminal.iter().map(|r| r.id).collect();
        tracing::info!(ids = ?ids, "Smart delay rows without nodeId marked completed (terminal wait)");
    }
    if enqueueable.is_empty() {
        return 0;
    }
    let jobs = enqueueable
        .iter()
        .map(|row| {
            let job = resume_job(row, None);
            NewJob {
                name: job.name.into(),
                data: job.data,
                opts: JobOptions {
                    job_id: Some(job_id(row.id, row.trigger_at)),
                    delay_ms: Some((row.trigger_at - now).max(0) as u64),
                    ..job.opts
                },
            }
        })
        .collect();
    let ids: Vec<i64> = enqueueable.iter().map(|r| r.id).collect();
    match deps.jobs.add_bulk(QueueName::Integration, jobs).await {
        Ok(_) => enqueueable.len(),
        Err(err) => {
            tracing::error!(err = %err, ids = ?ids, "Failed to enqueue smart delay batch, resetting to pending for retry");
            if let Err(reset) = service::reset_to_pending(&deps.db, ids.clone(), None).await {
                tracing::error!(err = %reset, ids = ?ids, "Failed to reset smart delay batch status to pending");
            }
            0
        }
    }
}

/// `endOfMinute(now + ENQUEUE_DELAY_MS)`.
fn window_until(now: i64) -> i64 {
    let at = now + ENQUEUE_DELAY_MS as i64;
    at - at.rem_euclid(60_000) + 59_999
}

/// `scanSmartDelay`: `{ scanned, enqueued }`.
pub async fn scan_smart_delay(deps: &Deps) -> Result<Value, JobError> {
    scan_smart_delay_with(deps, chatbotx_db::repo::now_ms(), ScanLimits::default()).await
}

/// [`scan_smart_delay`] at `now` with the batch bounds given.
pub async fn scan_smart_delay_with(
    deps: &Deps,
    now: i64,
    limits: ScanLimits,
) -> Result<Value, JobError> {
    // A sweep hiccup must never block claiming fresh due rows in the same tick.
    if let Err(err) = sweep_stuck_scheduled(deps, now - STUCK_SCHEDULED_GRACE_MS, limits).await {
        tracing::error!(err = %err, "Smart delay sweep failed; continuing with claim");
    }
    let until = window_until(now);
    let (mut scanned, mut enqueued, mut last_batch) = (0, 0, 0);
    for _ in 0..limits.max_claim_batches {
        let claimed = service::claim_due_rows(&deps.db, until, limits.batch_size)
            .await
            .map_err(crate::db_fail)?;
        last_batch = claimed.len();
        if claimed.is_empty() {
            break;
        }
        scanned += claimed.len();
        enqueued += enqueue_claimed_batch(deps, &claimed, now).await;
        if (claimed.len() as i64) < limits.batch_size {
            break;
        }
    }
    if last_batch as i64 == limits.batch_size {
        tracing::warn!(
            scanned,
            max_batches = limits.max_claim_batches,
            "Smart delay claim hit its per-run batch cap; backlog remains for the next tick"
        );
    }
    Ok(json!({ "scanned": scanned, "enqueued": enqueued }))
}

#[cfg(test)]
mod tests {
    use super::{parse_metadata, window_until};
    use serde_json::json;

    #[test]
    fn metadata_needs_the_fields_of_its_type() {
        let ok = |v: serde_json::Value| parse_metadata(Some(&v)).is_some();
        assert!(ok(json!({ "type": "updateStatus" })));
        assert!(ok(
            json!({ "type": "broadcast", "broadcastId": "1", "contactInboxId": "2" })
        ));
        assert!(!ok(json!({ "type": "broadcast", "broadcastId": "1" })));
        assert!(!ok(
            json!({ "type": "stepInputResolution", "stepId": "s", "challengeId": "c", "reason": "invalid_input_attempts" })
        ));
        assert!(!ok(json!({ "type": "updateStatus", "stepId": 3 })));
        assert!(!ok(json!({ "type": "other" })));
    }

    #[test]
    fn the_claim_window_ends_at_the_end_of_the_minute_five_minutes_ahead() {
        // 2026-07-16T00:00:00Z -> 2026-07-16T00:05:59.999Z
        assert_eq!(window_until(1_784_160_000_000), 1_784_160_359_999);
    }
}
