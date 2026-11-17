//! `handleWait` / `splitTraffic` (`step.ts`), `follow-up.ts`, `smart-delay.ts`
//! (`scheduleSmartDelayResume`, `buildSendFlowResumeJob`) and
//! `wait-resume.ts`, on the `ContactOnSmartDelay` table.
//!
//! ponytail: the `scanSmartDelay` schedule job (rows due later than
//! `ENQUEUE_DELAY_MS`, stuck-row recovery) is the schedule worker (Q.W2);
//! until then only near deadlines are enqueued.

use chatbotx_core::flow_config::compute_follow_up_trigger_at;
use chatbotx_core::flow_config::metadata_schema;
use chatbotx_core::flow_config::{ENQUEUE_DELAY_MS, build_job_id, compute_trigger_at};
use chatbotx_core::utils::id::math_random;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, Row, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::queues::integration::flow_continuation_job_options;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Map, Value, json};

use super::StepCtx;
use crate::flow::{SEND_FLOW, StepStatus, seek_connected_node};
use crate::{Deps, db_fail, id_of};

pub(crate) const RESUME_WAIT: &str = "resumeWait";
pub(crate) const RESUME_FOLLOW_UP: &str = "resumeFollowUp";

/// One `ContactOnSmartDelay` row (`SmartDelayRow`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SmartDelayRow {
    pub id: i64,
    pub workspace_id: i64,
    pub flow_id: i64,
    pub flow_version_id: Option<i64>,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub node_id: Option<String>,
    pub step_id: Option<String>,
    pub metadata: Option<Value>,
    pub kind: String,
    pub created_at: i64,
    pub trigger_at: i64,
    pub status: String,
    pub appointment_id: Option<i64>,
}

const COLUMNS: &str = r#""id", "workspaceId", "flowId", "flowVersionId", "contactInboxId", "conversationId", "nodeId", "stepId", "metadata", "type", "createdAt", "triggerAt", "status", "appointmentId""#;

fn row_of(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<SmartDelayRow> {
    Ok(SmartDelayRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        flow_id: r.get(2)?,
        flow_version_id: r.get(3)?,
        contact_inbox_id: r.get(4)?,
        conversation_id: r.get(5)?,
        node_id: r.get(6)?,
        step_id: r.get(7)?,
        metadata: r
            .get::<_, Option<String>>(8)?
            .and_then(|m| serde_json::from_str(&m).ok()),
        kind: r.get(9)?,
        created_at: r.get(10)?,
        trigger_at: r.get(11)?,
        status: r.get(12)?,
        appointment_id: r.get(13)?,
    })
}

pub(crate) async fn find_by_id(db: &Db, id: i64) -> Result<Option<SmartDelayRow>, DbError> {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                &format!(r#"SELECT {COLUMNS} FROM "ContactOnSmartDelay" WHERE "id" = ?1"#),
                params![id],
                row_of,
            )
            .optional()?)
    })
    .await
}

/// `smartDelayService.claimForRun`: `scheduled` → `to`, once.
async fn claim_for_run(db: &Db, id: i64, to: &'static str) -> Result<bool, DbError> {
    db.write(move |tx| {
        Ok(tx.execute(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = ?1 WHERE "id" = ?2 AND "status" = 'scheduled'"#,
            params![to, id],
        )? > 0)
    })
    .await
}

/// Sets `status` from one of `from` (`markScheduled` / `resetToPending` /
/// `requeueClaimedRun`).
async fn set_status(
    db: &Db,
    id: i64,
    from: &'static [&'static str],
    to: &'static str,
) -> Result<bool, DbError> {
    db.write(move |tx| {
        let marks = from.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
        Ok(tx.execute(
            &format!(r#"UPDATE "ContactOnSmartDelay" SET "status" = ?1 WHERE "id" = ?2 AND "status" IN ({marks})"#),
            params![to, id],
        )? > 0)
    })
    .await
}

/// `buildSendFlowResumeJob`: the `sendFlow` data rebuilt from the row (a
/// metadata that no longer parses is dropped).
pub(crate) fn send_flow_resume_data(row: &SmartDelayRow) -> Map<String, Value> {
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
    if let Some(metadata) = resume_metadata(row) {
        data.insert("metadata".into(), metadata);
    }
    if let Some(appointment) = row.appointment_id {
        data.insert("appointmentId".into(), json!(appointment.to_string()));
    }
    data
}

fn resume_metadata(row: &SmartDelayRow) -> Option<Value> {
    let parsed = metadata_schema().safe_parse(row.metadata.as_ref()?);
    parsed.success.then_some(parsed.data).flatten()
}

/// `scheduleSmartDelayResume` input.
pub(crate) struct Schedule {
    pub kind: &'static str,
    pub trigger_at: i64,
    pub node_id: String,
    pub step_id: String,
}

/// `scheduleSmartDelayResume`: the row first (the scanner recovers a crash),
/// then the resume job when the deadline is near.
pub(crate) async fn schedule_resume(
    ctx: &StepCtx<'_, '_>,
    input: Schedule,
) -> Result<(), JobError> {
    let run = ctx.run;
    let metadata = run.metadata().cloned();
    let row = SmartDelayRow {
        id: new_id(),
        workspace_id: run.conversation.workspace_id,
        flow_id: run.flow_version.flow_id,
        flow_version_id: (!run.use_latest_flow_version).then_some(run.flow_version.id),
        contact_inbox_id: run.contact_inbox.id,
        conversation_id: run.conversation.id,
        node_id: Some(input.node_id),
        step_id: Some(input.step_id),
        metadata: metadata.clone(),
        kind: input.kind.into(),
        created_at: now_ms(),
        trigger_at: input.trigger_at,
        status: "pending".into(),
        appointment_id: run.data.get("appointmentId").and_then(id_of),
    };
    let insert = row.clone();
    let persisted = run
        .deps
        .db
        .write(move |tx| {
            let r = &insert;
            let metadata = r.metadata.as_ref().map(Value::to_string);
            let upsert = if r.kind == "followUp" {
                r#" ON CONFLICT ("workspaceId", "contactInboxId", "flowId", "stepId")
                    WHERE "status" NOT IN ('completed', 'failed', 'canceled') AND "type" = 'followUp'
                    DO UPDATE SET "conversationId" = excluded."conversationId", "appointmentId" = excluded."appointmentId",
                      "createdAt" = excluded."createdAt", "flowVersionId" = excluded."flowVersionId",
                      "metadata" = excluded."metadata", "nodeId" = excluded."nodeId", "status" = 'pending',
                      "triggerAt" = excluded."triggerAt""#
            } else {
                ""
            };
            Ok(tx.query_row(
                &format!(
                    r#"INSERT INTO "ContactOnSmartDelay" ({COLUMNS})
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending', ?13){upsert}
                       RETURNING {COLUMNS}"#
                ),
                params![
                    r.id,
                    r.workspace_id,
                    r.flow_id,
                    r.flow_version_id,
                    r.contact_inbox_id,
                    r.conversation_id,
                    r.node_id,
                    r.step_id,
                    metadata,
                    r.kind,
                    r.created_at,
                    r.trigger_at,
                    r.appointment_id
                ],
                row_of,
            )?)
        })
        .await
        .map_err(db_fail)?;
    let diff = persisted.trigger_at - now_ms();
    if diff > ENQUEUE_DELAY_MS as i64 {
        return Ok(());
    }
    let db = &run.deps.db;
    let enqueued = async {
        set_status(db, persisted.id, &["pending", "scheduled"], "scheduled")
            .await
            .map_err(|e| e.to_string())?;
        let name = if persisted.kind == "followUp" {
            RESUME_FOLLOW_UP
        } else {
            RESUME_WAIT
        };
        let trigger =
            chrono::DateTime::from_timestamp_millis(persisted.trigger_at).unwrap_or_default();
        run.deps
            .jobs
            .add(
                QueueName::Integration,
                name,
                json!({ "type": name, "data": { "smartDelayId": persisted.id.to_string() } }),
                JobOptions {
                    delay_ms: Some(u64::try_from(diff.max(0)).unwrap_or(0)),
                    job_id: Some(build_job_id(&persisted.id.to_string(), trigger)),
                    ..flow_continuation_job_options(metadata.as_ref())
                },
            )
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(err) = enqueued {
        if let Err(reset) = set_status(db, persisted.id, &["scheduled"], "pending").await {
            tracing::warn!(err = %reset, row_id = persisted.id, "Failed to reset immediate smart delay after enqueue failure");
        }
        tracing::warn!(err = %err, row_id = persisted.id, "Failed to immediately enqueue smart delay; scanner will pick it up");
    }
    Ok(())
}

/// The delay steps; `None` = not a delay step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    Ok(Some(match step_type {
        "wait" => handle_wait(ctx).await?,
        "followUp" => handle_follow_up(ctx).await?,
        "splitTraffic" => {
            split_traffic(ctx).await?;
            StepStatus::Success
        }
        _ => return Ok(None),
    }))
}

/// `ContactCustomField` value of the contact (`findValue`).
async fn custom_field_value(ctx: &StepCtx<'_, '_>, field_id: i64) -> Option<String> {
    let contact_id = ctx.contact_id();
    let found = ctx
        .run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
                    params![contact_id, field_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await;
    found.unwrap_or_else(|err| {
        tracing::error!(err = %err, custom_field_id = field_id, "Failed to query custom field for wait step");
        None
    })
}

/// `handleWait`.
async fn handle_wait(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    if run.target_id.is_empty() {
        return Ok(StepStatus::Skip);
    }
    let field_value = match ctx.step.get("outputFieldId").and_then(id_of) {
        Some(id) if ctx.str("dateType") == Some("dynamic") => custom_field_value(ctx, id).await,
        _ => None,
    };
    let Some(trigger_at) =
        compute_trigger_at(ctx.step, now_ms(), math_random(), field_value.as_deref())
    else {
        return Ok(super::step_error(ctx, "Unable to compute wait triggerAt"));
    };
    let Some(node_id) = seek_connected_node(&run.flow_version.edges.0, &run.target_id) else {
        return Ok(StepStatus::Skip);
    };
    schedule_resume(
        ctx,
        Schedule {
            kind: "waitNode",
            trigger_at: trigger_at.timestamp_millis(),
            node_id,
            step_id: ctx.str("id").unwrap_or("").to_owned(),
        },
    )
    .await?;
    Ok(StepStatus::Wait)
}

/// `handleFollowUp`.
async fn handle_follow_up(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    if run.target_id.is_empty() {
        return Ok(StepStatus::Skip);
    }
    let Some(node_id) = seek_connected_node(&run.flow_version.edges.0, &run.target_id) else {
        return Ok(StepStatus::Skip);
    };
    let trigger_at = compute_follow_up_trigger_at(ctx.step, &chatbotx_kv::SystemClock);
    schedule_resume(
        ctx,
        Schedule {
            kind: "followUp",
            trigger_at: trigger_at.timestamp_millis(),
            node_id,
            step_id: ctx.str("id").unwrap_or("").to_owned(),
        },
    )
    .await?;
    Ok(StepStatus::Wait)
}

/// `splitTraffic`: one weighted case, the node on `<targetId>-case-<i>`.
async fn split_traffic(ctx: &StepCtx<'_, '_>) -> Result<(), JobError> {
    let cases = ctx
        .step
        .get("cases")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if ctx.run.target_id.is_empty() || cases.is_empty() {
        return Ok(());
    }
    let weight = |c: &Value| c.get("value").and_then(Value::as_f64).unwrap_or(0.0);
    let total: f64 = cases.iter().map(weight).sum();
    let bucket = math_random() * total;
    let mut cumulative = 0.0;
    let mut selected = 0;
    for (i, case) in cases.iter().enumerate() {
        cumulative += weight(case);
        if bucket < cumulative {
            selected = i;
            break;
        }
    }
    let handle = format!("{}-case-{selected}", ctx.run.target_id);
    let Some(node_id) = seek_connected_node(&ctx.run.flow_version.edges.0, &handle) else {
        return Ok(());
    };
    let job = ctx.send_flow_job(
        &node_id,
        &[
            "metadata",
            "isBulkBroadcast",
            "sendFrom",
            "sentByUserId",
            "nodeVisits",
            "triggerMessageId",
            "triggerMessageCreatedAt",
            "startedByContactMessage",
            "commentAnchor",
            "appointmentId",
        ],
    );
    ctx.enqueue(job).await
}

fn due_row(row: Option<SmartDelayRow>, kind: &str) -> Option<SmartDelayRow> {
    row.filter(|r| {
        r.kind == kind && r.status == "scheduled" && r.node_id.is_some() && r.trigger_at <= now_ms()
    })
}

/// `runWaitResume`: claims the row, then runs the connected node in this
/// job (its execution key); a failed run reopens the row and retries.
pub(crate) async fn run_wait_resume(
    deps: &Deps,
    data: &Value,
    flow_execution_key: String,
) -> Result<(), JobError> {
    let Some(id) = data.get("smartDelayId").and_then(id_of) else {
        return Ok(());
    };
    let Some(row) = due_row(find_by_id(&deps.db, id).await.map_err(db_fail)?, "waitNode") else {
        return Ok(());
    };
    if !claim_for_run(&deps.db, row.id, "completed")
        .await
        .map_err(db_fail)?
    {
        return Ok(());
    }
    let resume = Value::Object(send_flow_resume_data(&row));
    if let Err(err) = crate::flow::run_flow_node(deps, &resume, flow_execution_key).await {
        match set_status(&deps.db, row.id, &["completed"], "scheduled").await {
            Ok(true) => {}
            Ok(false) => tracing::error!(
                smart_delay_id = row.id,
                "Failed to requeue a claimed wait smart delay after flow failure"
            ),
            Err(e) => {
                tracing::error!(err = %e, smart_delay_id = row.id, "Failed to requeue a claimed wait smart delay after flow failure")
            }
        }
        return Err(err);
    }
    Ok(())
}

/// `runFollowUpResume`: a reply since the row was armed cancels it; else
/// the flow goes on at the connected node.
pub(crate) async fn run_follow_up_resume(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(id) = data.get("smartDelayId").and_then(id_of) else {
        return Ok(());
    };
    let Some(row) = due_row(find_by_id(&deps.db, id).await.map_err(db_fail)?, "followUp") else {
        return Ok(());
    };
    let (ws, contact_inbox_id, since) = (row.workspace_id, row.contact_inbox_id, row.created_at);
    let replied = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT 1 FROM "ContactInbox" ci JOIN "Contact" c ON c."id" = ci."contactId"
                       WHERE ci."id" = ?1 AND c."workspaceId" = ?2 AND ci."lastIncomingMessageAt" > ?3"#,
                    params![contact_inbox_id, ws, since],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })
        .await
        .map_err(db_fail)?;
    if replied {
        if claim_for_run(&deps.db, row.id, "canceled")
            .await
            .map_err(db_fail)?
        {
            tracing::info!(
                smart_delay_id = row.id,
                conversation_id = row.conversation_id,
                "Follow-up canceled: contact replied before the timer expired"
            );
        }
        return Ok(());
    }
    if !claim_for_run(&deps.db, row.id, "completed")
        .await
        .map_err(db_fail)?
    {
        return Ok(());
    }
    let data = send_flow_resume_data(&row);
    let opts = flow_continuation_job_options(data.get("metadata"));
    deps.jobs
        .add(
            QueueName::Integration,
            SEND_FLOW,
            json!({ "type": SEND_FLOW, "data": Value::Object(data) }),
            opts,
        )
        .await
        .map(|_| ())
        .map_err(db_fail)
}
