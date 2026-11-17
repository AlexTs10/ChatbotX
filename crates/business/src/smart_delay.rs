//! `packages/business/src/smart-delay/service.ts` and
//! `workspace-lifecycle/smart-delay-cleanup.ts` (Q.D5): the
//! `"ContactOnSmartDelay"` rows (wait steps, follow-ups, quick-reply
//! follow-ups, step-input timeouts). SQLite has one writer, so the
//! `FOR UPDATE [SKIP LOCKED]` of the original is the write transaction itself.

use chatbotx_core::flow_config::steps::build_job_id;
use chatbotx_db::Db;
use chatbotx_db::repo::models::ContactOnSmartDelay as Row;
use chatbotx_db::repo::{self, Json, Model};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use chatbotx_jobs::{Enqueuer, QueueName};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// The terminal statuses (`completed`, `failed`, `canceled`).
const ACTIVE: &str = r#""status" NOT IN ('completed', 'failed', 'canceled')"#;

fn columns() -> String {
    Row::columns_sql(None)
}

fn loud(message: &str) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(message))
}

fn ids_sql(ids: &[i64]) -> String {
    vec!["?"; ids.len()].join(", ")
}

/// `create`.
pub fn create_in(conn: &Connection, row: &Row) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "ContactOnSmartDelay" ("id", "workspaceId", "flowId", "flowVersionId",
              "contactInboxId", "conversationId", "nodeId", "stepId", "metadata", "type",
              "createdAt", "triggerAt", "status", "appointmentId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
        params![
            row.id,
            row.workspace_id,
            row.flow_id,
            row.flow_version_id,
            row.contact_inbox_id,
            row.conversation_id,
            row.node_id,
            row.step_id,
            row.metadata,
            row.r#type,
            row.created_at,
            row.trigger_at,
            row.status,
            row.appointment_id,
        ],
    )?;
    Ok(())
}

/// `create`.
pub async fn create(db: &Db, row: Row) -> Result<()> {
    tx::write(db, move |tx| create_in(tx, &row)).await
}

/// The insert of `row` that, on a conflict with the active row of the
/// partial unique key (`target` + `type`), restarts that row with `set`.
fn upsert_in(
    conn: &Connection,
    row: &Row,
    kind: &'static str,
    target: &str,
    set: &str,
    failure: &str,
) -> Result<Row> {
    let sql = format!(
        r#"INSERT INTO "ContactOnSmartDelay" ("id", "workspaceId", "flowId", "flowVersionId",
              "contactInboxId", "conversationId", "nodeId", "stepId", "metadata", "type",
              "createdAt", "triggerAt", "status", "appointmentId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
           ON CONFLICT ({target}) WHERE {ACTIVE} AND "type" = '{kind}'
           DO UPDATE SET {set}, "createdAt" = ?15, "status" = 'pending'
           RETURNING {}"#,
        columns()
    );
    conn.query_row(
        &sql,
        params![
            row.id,
            row.workspace_id,
            row.flow_id,
            row.flow_version_id,
            row.contact_inbox_id,
            row.conversation_id,
            row.node_id,
            row.step_id,
            row.metadata,
            row.r#type,
            row.created_at,
            row.trigger_at,
            row.status,
            row.appointment_id,
            repo::now_ms(),
        ],
        Row::from_row,
    )
    .optional()?
    .ok_or_else(|| loud(failure))
}

/// `upsertFollowUp`: one active follow-up per (workspace, contact inbox, flow, step).
pub async fn upsert_follow_up(db: &Db, row: Row) -> Result<Row> {
    tx::write(db, move |tx| {
        upsert_in(
            tx,
            &row,
            "followUp",
            r#""workspaceId", "contactInboxId", "flowId", "stepId""#,
            r#""conversationId" = excluded."conversationId", "appointmentId" = excluded."appointmentId",
               "flowVersionId" = excluded."flowVersionId", "metadata" = excluded."metadata",
               "nodeId" = excluded."nodeId", "triggerAt" = excluded."triggerAt""#,
            "Failed to upsert follow-up smart delay",
        )
    })
    .await
}

/// `upsertQuickReplyFollowUp`: one active follow-up per quick-reply node.
pub async fn upsert_quick_reply_follow_up(db: &Db, row: Row) -> Result<Row> {
    tx::write(db, move |tx| {
        upsert_in(
            tx,
            &row,
            "quickReplyFollowUp",
            r#""workspaceId", "contactInboxId", "flowId", "nodeId""#,
            r#""conversationId" = excluded."conversationId", "appointmentId" = excluded."appointmentId",
               "flowVersionId" = excluded."flowVersionId", "metadata" = excluded."metadata",
               "stepId" = excluded."stepId", "triggerAt" = excluded."triggerAt""#,
            "Failed to upsert quick reply follow-up smart delay",
        )
    })
    .await
}

/// `upsertStepInputTimeout`: arms the one timer of the conversation, unless
/// the conversation is gone or its pending challenge is no longer the one in
/// `metadata.challengeId` (then `None`). The check and the upsert share one
/// write transaction (the original locks the conversation row).
pub async fn upsert_step_input_timeout(db: &Db, row: Row) -> Result<Option<Row>> {
    let challenge_id = row
        .metadata
        .as_ref()
        .and_then(|m| m.get("challengeId"))
        .and_then(|c| c.as_str())
        .map(str::to_owned);
    tx::write(db, move |tx| {
        let pending: Option<Option<String>> = tx
            .query_row(
                r#"SELECT json_extract("additionalAttributes", '$.challenge.data.challengeId')
                   FROM "Conversation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![row.conversation_id, row.workspace_id],
                |r| r.get(0),
            )
            .optional()?;
        if challenge_id.is_none() || pending.flatten() != challenge_id {
            return Ok(None);
        }
        upsert_in(
            tx,
            &row,
            "stepInputTimeout",
            r#""workspaceId", "conversationId""#,
            r#""contactInboxId" = excluded."contactInboxId", "flowId" = excluded."flowId",
               "flowVersionId" = excluded."flowVersionId", "appointmentId" = excluded."appointmentId",
               "metadata" = excluded."metadata", "nodeId" = excluded."nodeId",
               "stepId" = excluded."stepId", "triggerAt" = excluded."triggerAt""#,
            "Failed to upsert step input timeout smart delay",
        )
        .map(Some)
    })
    .await
}

/// `cancelQuickReplyFollowUps`: by contact inbox and/or conversations, except
/// one node / flow; nothing without a contact inbox or conversation.
pub fn cancel_quick_reply_follow_ups_in(
    conn: &Connection,
    workspace_id: i64,
    contact_inbox_id: Option<i64>,
    conversation_ids: &[i64],
    except_node_id: Option<&str>,
    except_flow_id: Option<i64>,
) -> Result<usize> {
    if contact_inbox_id.is_none() && conversation_ids.is_empty() {
        return Ok(0);
    }
    let mut sql = format!(
        r#"UPDATE "ContactOnSmartDelay" SET "status" = 'canceled'
           WHERE "workspaceId" = ? AND "type" = 'quickReplyFollowUp' AND {ACTIVE}"#
    );
    let mut args: Vec<chatbotx_db::rusqlite::types::Value> = vec![workspace_id.into()];
    if let Some(id) = contact_inbox_id {
        sql.push_str(r#" AND "contactInboxId" = ?"#);
        args.push(id.into());
    }
    if !conversation_ids.is_empty() {
        sql.push_str(&format!(
            r#" AND "conversationId" IN ({})"#,
            ids_sql(conversation_ids)
        ));
        args.extend(conversation_ids.iter().map(|&id| id.into()));
    }
    if let Some(node) = except_node_id {
        sql.push_str(r#" AND "nodeId" <> ?"#);
        args.push(node.to_owned().into());
    }
    if let Some(flow) = except_flow_id {
        sql.push_str(r#" AND "flowId" <> ?"#);
        args.push(flow.into());
    }
    Ok(conn.execute(&sql, params_from_iter(args))?)
}

/// `cancelQuickReplyFollowUpsForStaleVersion`.
pub fn cancel_quick_reply_follow_ups_for_stale_version_in(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
    current_flow_version_id: i64,
) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = 'canceled'
                   WHERE "workspaceId" = ?1 AND "type" = 'quickReplyFollowUp' AND {ACTIVE}
                     AND "flowId" = ?2 AND "flowVersionId" IS NOT NULL AND "flowVersionId" <> ?3"#
        ),
        params![workspace_id, flow_id, current_flow_version_id],
    )?)
}

async fn mark_status(db: &Db, id: i64, status: &'static str) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = ?2 WHERE "id" = ?1"#,
            params![id, status],
        )?;
        Ok(())
    })
    .await
}

/// `markScheduled`.
pub async fn mark_scheduled(db: &Db, id: i64) -> Result<()> {
    mark_status(db, id, "scheduled").await
}

/// `markCompleted`.
pub async fn mark_completed(db: &Db, id: i64) -> Result<()> {
    mark_status(db, id, "completed").await
}

/// `markCanceled`.
pub async fn mark_canceled(db: &Db, id: i64) -> Result<()> {
    mark_status(db, id, "canceled").await
}

/// `findById`.
pub async fn find_by_id(db: &Db, id: i64) -> Result<Option<Row>> {
    tx::read(db, move |conn| {
        Ok(repo::find_one::<Row>(conn, r#""id" = ?1"#, &[&id])?)
    })
    .await
}

/// One `countByFlowStep` group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepCount {
    pub step_id: String,
    pub status: String,
    pub total: i64,
}

/// `countByFlowStep`: rows per (step, status) of a flow; rows without a step
/// are left out.
pub async fn count_by_flow_step(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<StepCount>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT "stepId", "status", count(*) FROM "ContactOnSmartDelay"
                   WHERE "workspaceId" = ?1 AND "flowId" = ?2 AND "stepId" IS NOT NULL
                   GROUP BY "stepId", "status" ORDER BY "stepId", "status""#,
        )?;
        let rows = stmt
            .query_map(params![workspace_id, flow_id], |r| {
                Ok(StepCount {
                    step_id: r.get(0)?,
                    status: r.get(1)?,
                    total: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `claimDueRows`: up to `limit` pending rows due by `window_until` (oldest
/// first) become `scheduled`.
pub async fn claim_due_rows(db: &Db, window_until: i64, limit: i64) -> Result<Vec<Row>> {
    tx::write(db, move |tx| {
        let sql = format!(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = 'scheduled'
               WHERE "status" = 'pending' AND "id" IN (
                 SELECT "id" FROM "ContactOnSmartDelay"
                 WHERE "status" = 'pending' AND "triggerAt" <= ?1
                 ORDER BY "triggerAt" LIMIT ?2)
               RETURNING {}"#,
            columns()
        );
        let mut rows = repo::query_models::<Row>(tx, &sql, &[&window_until, &limit])?;
        rows.sort_by_key(|r| (r.trigger_at, r.id));
        Ok(rows)
    })
    .await
}

/// `claimForRun`: a `scheduled` row moves to `to` (`completed` / `canceled`);
/// with `observed_trigger_at` only while the row still has that deadline.
pub async fn claim_for_run(
    db: &Db,
    id: i64,
    to: &'static str,
    observed_trigger_at: Option<i64>,
) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx
            .execute(
                r#"UPDATE "ContactOnSmartDelay" SET "status" = ?2
                   WHERE "id" = ?1 AND "status" = 'scheduled' AND (?3 IS NULL OR "triggerAt" = ?3)"#,
                params![id, to, observed_trigger_at],
            )?
            > 0)
    })
    .await
}

/// `requeueClaimedRun`: a row claimed as `completed` goes back to `scheduled`.
pub async fn requeue_claimed_run(db: &Db, id: i64) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx.execute(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = 'scheduled'
                   WHERE "id" = ?1 AND "status" = 'completed'"#,
            [id],
        )? > 0)
    })
    .await
}

/// `{ id, triggerAt }` of a row (the deterministic job id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowDeadline {
    pub id: i64,
    pub trigger_at: i64,
}

/// `listStuckScheduled`: `scheduled` rows due before `older_than`, oldest first.
pub async fn list_stuck_scheduled(
    db: &Db,
    older_than: i64,
    limit: i64,
) -> Result<Vec<RowDeadline>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT "id", "triggerAt" FROM "ContactOnSmartDelay"
                   WHERE "status" = 'scheduled' AND "triggerAt" < ?1
                   ORDER BY "triggerAt" LIMIT ?2"#,
        )?;
        let rows = stmt
            .query_map(params![older_than, limit], |r| {
                Ok(RowDeadline {
                    id: r.get(0)?,
                    trigger_at: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `resetToPending`: only rows still `scheduled` (and, with
/// `trigger_at_before`, due before it) go back to `pending`; returns how many.
pub async fn reset_to_pending(
    db: &Db,
    ids: Vec<i64>,
    trigger_at_before: Option<i64>,
) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    tx::write(db, move |tx| {
        let sql = format!(
            r#"UPDATE "ContactOnSmartDelay" SET "status" = 'pending'
               WHERE "status" = 'scheduled' AND (?1 IS NULL OR "triggerAt" < ?1) AND "id" IN ({})"#,
            ids_sql(&ids)
        );
        let mut args: Vec<chatbotx_db::rusqlite::types::Value> = vec![trigger_at_before.into()];
        args.extend(ids.iter().map(|&id| id.into()));
        Ok(tx.execute(&sql, params_from_iter(args))?)
    })
    .await
}

/// `cancelActiveForWorkspace`: up to `limit` pending / scheduled rows of the
/// workspace become `canceled`.
pub fn cancel_active_for_workspace_in(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<RowDeadline>> {
    let mut stmt = conn.prepare_cached(
        r#"UPDATE "ContactOnSmartDelay" SET "status" = 'canceled'
               WHERE "id" IN (SELECT "id" FROM "ContactOnSmartDelay"
                 WHERE "workspaceId" = ?1 AND "status" IN ('pending', 'scheduled') LIMIT ?2)
               RETURNING "id", "triggerAt""#,
    )?;
    let rows = stmt
        .query_map(params![workspace_id, limit], |r| {
            Ok(RowDeadline {
                id: r.get(0)?,
                trigger_at: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `buildJobId(row.id, row.triggerAt)`.
pub fn job_id(id: i64, trigger_at: i64) -> String {
    build_job_id(
        &id.to_string(),
        chrono::DateTime::from_timestamp_millis(trigger_at).unwrap_or_default(),
    )
}

/// `SMART_DELAY_CANCEL_BATCH_SIZE`.
pub const SMART_DELAY_CANCEL_BATCH_SIZE: i64 = 500;
const MAX_CANCEL_BATCHES: usize = 400;

/// `cancelSmartDelaysForWorkspace` (`drainInBatches`): cancels every
/// still-firable row of the workspace in batches and removes their delayed
/// `integration` jobs (best effort: a canceled row's job is inert). Past
/// `deadline_at` or the batch cap the purge is incomplete (an error).
pub async fn cancel_smart_delays_for_workspace(
    db: &Db,
    jobs: &dyn Enqueuer,
    workspace_id: i64,
    deadline_at: i64,
) -> Result<usize> {
    let incomplete = || {
        loud(&format!(
            "Workspace {workspace_id} purge incomplete: ContactOnSmartDelay"
        ))
    };
    let mut total = 0;
    for _ in 0..MAX_CANCEL_BATCHES {
        if repo::now_ms() >= deadline_at {
            return Err(incomplete());
        }
        let rows = tx::write(db, move |tx| {
            cancel_active_for_workspace_in(tx, workspace_id, SMART_DELAY_CANCEL_BATCH_SIZE)
        })
        .await?;
        total += rows.len();
        if rows.is_empty() {
            return Ok(total);
        }
        let mut failed = 0;
        for row in &rows {
            if jobs
                .remove_job(QueueName::Integration, &job_id(row.id, row.trigger_at))
                .await
                .is_err()
            {
                failed += 1;
            }
        }
        if failed > 0 {
            tracing::warn!(
                failed_count = failed,
                workspace_id,
                "workspace-freeze: failed to remove smart-delay jobs"
            );
        }
        if (rows.len() as i64) < SMART_DELAY_CANCEL_BATCH_SIZE {
            return Ok(total);
        }
    }
    Err(incomplete())
}

/// A new pending row (`createId()` id, `createdAt` now).
#[allow(clippy::too_many_arguments)]
pub fn new_row(
    r#type: &str,
    workspace_id: i64,
    flow_id: i64,
    flow_version_id: Option<i64>,
    conversation_id: i64,
    contact_inbox_id: i64,
    node_id: Option<String>,
    step_id: Option<String>,
    metadata: Option<serde_json::Value>,
    trigger_at: i64,
    appointment_id: Option<i64>,
) -> Row {
    Row {
        id: repo::new_id(),
        workspace_id,
        flow_id,
        flow_version_id,
        contact_inbox_id,
        conversation_id,
        node_id,
        step_id,
        metadata: metadata.map(Json),
        r#type: r#type.to_owned(),
        created_at: repo::now_ms(),
        trigger_at,
        status: "pending".to_owned(),
        appointment_id,
    }
}
