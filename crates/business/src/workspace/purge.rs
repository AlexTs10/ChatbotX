//! `workspaceService.purgeDueScheduled` (Q.D11 slice 9): deletes the
//! workspaces whose scheduled deletion is due (or whose purge started).
//! Per workspace: fence, deletion record, runtime freeze, payment providers,
//! heavy-data drain, post and contact-activity drains, then one write
//! transaction that deletes the row and closes the deletion log.
//!
//! Not here: `disconnectWorkspaceIntegrations` and
//! `disconnectWorkspaceChannels` (the integration and connection-state
//! units), caches, `FOR UPDATE SKIP LOCKED` and statement timeouts (SQLite
//! has one writer).

use std::collections::BTreeSet;
use std::sync::Arc;

use chatbotx_core::js::to_iso_string;
use chatbotx_db::repo::resource_deletion_log::{
    complete_pending_by_id, find_pending_id_by_resource,
};
use chatbotx_db::repo::workspace_deletion_record::{
    self as records, NewDeletionRecord, PeriodCount,
};
use chatbotx_db::repo::{self, now_ms, template_uninstall};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_db::{Db, DbError};
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt as _};
use serde_json::{Value, json};

use super::lifecycle::{self, PurgeError};
use super::{record_log, workspace_key};
use crate::user_quota::{self, QuotaMetric};
use crate::{Result, tx};

/// `PURGE_WORKSPACE_TEARDOWN_CONCURRENCY`.
pub const PURGE_WORKSPACE_TEARDOWN_CONCURRENCY: usize = 5;
/// `PURGE_WORKSPACE_MAX_DURATION_MS`.
pub const PURGE_WORKSPACE_MAX_DURATION_MS: i64 = 45 * 60 * 1000;
/// `CONTACT_ACTIVITY_TABLES` (drain order).
const CONTACT_ACTIVITY_TABLES: &[&str] =
    &["ContactActivity", "ContactKeywordMessage", "ContactState"];

/// Lets go of a workspace's payment providers before the cascade (passed in
/// by the worker).
pub type PaymentTeardown = Arc<dyn Fn(i64) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// `purgeDueScheduled` props. The batch sizes and the clock (`Date.now` of
/// the deadline checks) are options so a test can stop a drain early.
#[derive(Clone)]
pub struct PurgeOptions {
    pub chunk_size: i64,
    pub max_chunks: u32,
    pub teardown_payment_providers: Option<PaymentTeardown>,
    /// `contactInboxPostService.purgeWorkspace` (batch size, max batches).
    pub post_batches: (i64, u32),
    /// `contactActivityService.purgeWorkspace` (batch size, max batches).
    pub activity_batches: (i64, u32),
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Default for PurgeOptions {
    fn default() -> PurgeOptions {
        PurgeOptions {
            chunk_size: 500,
            max_chunks: 20,
            teardown_payment_providers: None,
            post_batches: (1000, 100),
            activity_batches: (5000, 2000),
            clock: Arc::new(now_ms),
        }
    }
}

/// A due workspace (`DueWorkspace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueWorkspace {
    pub id: i64,
    pub owner_id: i64,
    pub tenant_id: i64,
}

const DUE: &str = r#"("purgeStartedAt" IS NOT NULL OR ("scheduledDeletionAt" IS NOT NULL AND "scheduledDeletionAt" < ?1))"#;

fn due_row(row: &chatbotx_db::rusqlite::Row<'_>) -> chatbotx_db::rusqlite::Result<DueWorkspace> {
    Ok(DueWorkspace {
        id: row.get(0)?,
        owner_id: row.get(1)?,
        tenant_id: row.get(2)?,
    })
}

/// `purgeDueScheduled`: the number of workspaces deleted. A failing
/// workspace is logged and left for the next run; it never fails the run.
pub async fn purge_due_scheduled(db: &Db, options: PurgeOptions) -> Result<i64> {
    let deadline_at = (options.clock)() + PURGE_WORKSPACE_MAX_DURATION_MS;
    let mut total = 0;
    let mut reconciles = BTreeSet::new();
    for _ in 0..options.max_chunks {
        if (options.clock)() >= deadline_at {
            tracing::info!(
                duration_ms = PURGE_WORKSPACE_MAX_DURATION_MS,
                "workspace-purge: deadline reached, deferring remaining work"
            );
            break;
        }
        let limit = options.chunk_size;
        let claimed = tx::read(db, move |c| {
            let sql = format!(
                r#"SELECT "id", "ownerId", "tenantId" FROM "Workspace" WHERE {DUE}
                   ORDER BY COALESCE("purgeStartedAt", "scheduledDeletionAt") ASC, "id" ASC LIMIT ?2"#
            );
            let rows = c
                .prepare(&sql)?
                .query_map(params![now_ms(), limit], due_row)?
                .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
        if claimed.is_empty() {
            break;
        }
        let deleted: Vec<DueWorkspace> = stream::iter(claimed.iter().copied())
            .map(|workspace| teardown_due_workspace(db, workspace, &options, deadline_at))
            .buffered(PURGE_WORKSPACE_TEARDOWN_CONCURRENCY)
            .filter_map(|done| async move { done })
            .collect()
            .await;
        // A chunk that tore nothing down is a systemic failure: stop.
        if deleted.is_empty() {
            break;
        }
        total += deleted.len() as i64;
        reconciles.extend(deleted.iter().map(|w| (w.owner_id, w.tenant_id)));
        tracing::info!(
            deleted = deleted.len(),
            "workspace-purge: workspaces purged"
        );
        if (claimed.len() as i64) < options.chunk_size {
            break;
        }
    }
    for (owner_id, tenant_id) in reconciles {
        if let Err(err) =
            user_quota::reconcile_owner_after_teardown(db, owner_id, Some(tenant_id)).await
        {
            tracing::error!(%err, owner_id, tenant_id, "workspace-purge: failed to reconcile owner quota");
        }
    }
    Ok(total)
}

fn incomplete(workspace_id: i64, table: &str) -> PurgeError {
    PurgeError::Incomplete {
        workspace_id,
        table: table.to_owned(),
    }
}

fn failed(err: impl std::fmt::Display) -> PurgeError {
    PurgeError::Db(DbError::other(err.to_string()))
}

/// `teardownDueWorkspace`: the workspace when it is gone; `None` when it is
/// deferred (logged).
async fn teardown_due_workspace(
    db: &Db,
    workspace: DueWorkspace,
    options: &PurgeOptions,
    deadline_at: i64,
) -> Option<DueWorkspace> {
    match teardown(db, workspace, options, deadline_at).await {
        Ok(done) => done,
        Err(PurgeError::Incomplete {
            workspace_id,
            table,
        }) => {
            tracing::info!(
                table,
                workspace_id,
                "workspace-purge: heavy data remains, deferring to next run"
            );
            None
        }
        Err(PurgeError::Db(err)) => {
            tracing::error!(%err, workspace_id = workspace.id, "workspace-purge: teardown failed, deferring to next run");
            None
        }
    }
}

/// The result of the final transaction.
enum Final {
    Deleted,
    NotDeleted,
    Uninstall,
}

async fn teardown(
    db: &Db,
    workspace: DueWorkspace,
    options: &PurgeOptions,
    deadline_at: i64,
) -> std::result::Result<Option<DueWorkspace>, PurgeError> {
    let id = workspace.id;
    let past_deadline = || (options.clock)() >= deadline_at;
    if past_deadline() {
        return Err(incomplete(id, "deadline"));
    }
    let Some(fenced) = acquire_purge_fence(db, id).await.map_err(failed)? else {
        return Ok(None);
    };
    let created = tx::write(db, move |c| Ok(record_for_workspace(c, fenced)?))
        .await
        .map_err(failed)?;
    if created == Some(true) {
        tracing::info!(
            owner_id = fenced.owner_id,
            workspace_id = id,
            "workspace-purge: deletion record written"
        );
    }
    lifecycle::freeze_workspace_runtime(db, id, deadline_at).await?;
    if let Some(teardown) = &options.teardown_payment_providers
        && let Err(err) = teardown(id).await
    {
        tracing::error!(%err, workspace_id = id, "workspace-purge: failed to disconnect payment providers");
    }
    lifecycle::purge_workspace_heavy_data(db, id, deadline_at, None).await?;
    if past_deadline() {
        return Err(incomplete(id, "deadline"));
    }
    // `contactInboxPostService.purgeWorkspace`: complete on a short batch,
    // otherwise when no row is left after the last batch.
    let (size, batches) = options.post_batches;
    if !drain(db, "ContactInboxPost", id, size, batches, true).await? {
        return Err(incomplete(id, "ContactInboxPost"));
    }
    // `contactActivityService.purgeWorkspace`: every table is checked.
    let (size, batches) = options.activity_batches;
    for table in CONTACT_ACTIVITY_TABLES {
        if !drain(db, table, id, size, batches, false).await? {
            return Err(incomplete(id, table));
        }
    }
    let pending = tx::read(db, move |c| {
        Ok(find_pending_id_by_resource(c, &workspace_key(id))?)
    })
    .await
    .map_err(failed)?;
    if past_deadline() {
        return Err(incomplete(id, "deadline"));
    }
    let outcome = tx::write(db, move |c| final_delete(c, id, pending))
        .await
        .map_err(failed)?;
    match outcome {
        Final::Uninstall => return Err(incomplete(id, "TemplateUninstall")),
        Final::NotDeleted => return Err(incomplete(id, "Workspace")),
        Final::Deleted => {}
    }
    // Best effort: the reconcile after the run re-derives the count.
    let owner = workspace.owner_id;
    if let Err(err) = tx::write(db, move |c| {
        Ok(user_quota::release(c, owner, QuotaMetric::Workspaces)?)
    })
    .await
    {
        tracing::warn!(%err, workspace_id = id, owner_id = owner, "workspace-purge: workspace quota release failed");
    }
    Ok(Some(fenced))
}

/// Deletes up to `batches` batches of `size` rows of the workspace from
/// `table` (one of the post / activity tables); `true` when none is left.
/// `short_batch_completes`: the post drain stops on a short batch without a
/// final check (as the original).
async fn drain(
    db: &Db,
    table: &'static str,
    workspace_id: i64,
    size: i64,
    batches: u32,
    short_batch_completes: bool,
) -> std::result::Result<bool, PurgeError> {
    for _ in 0..batches {
        let deleted = tx::write(db, move |c| {
            Ok(c.execute(
                &format!(
                    r#"DELETE FROM "{table}" WHERE rowid IN (SELECT rowid FROM "{table}" WHERE "workspaceId" = ?1 LIMIT ?2)"#
                ),
                params![workspace_id, size],
            )?)
        })
        .await
        .map_err(failed)?;
        if (deleted as i64) < size {
            if short_batch_completes {
                return Ok(true);
            }
            break;
        }
    }
    let left = tx::read(db, move |c| {
        Ok(c.query_row(
            &format!(r#"SELECT 1 FROM "{table}" WHERE "workspaceId" = ?1 LIMIT 1"#),
            [workspace_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
    })
    .await
    .map_err(failed)?;
    Ok(!left)
}

/// The final transaction: no live template uninstall, delete the row, mark
/// the deletion record purged and complete the deletion log.
fn final_delete(c: &Connection, id: i64, pending: Option<i64>) -> Result<Final> {
    if template_uninstall::has_live_uninstall_touching_workspace(c, id)? {
        return Ok(Final::Uninstall);
    }
    let deleted = c
        .query_row(
            r#"DELETE FROM "Workspace" WHERE "id" = ?1 RETURNING "name", "deletionRequestedById", "deletionRequestedAt""#,
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?)),
        )
        .optional()?;
    let Some((name, requested_by, requested_at)) = deleted else {
        return Ok(Final::NotDeleted);
    };
    records::mark_purged(c, id)?;
    // `completeWorkspaceDeletionLog`.
    match pending {
        Some(log_id) => {
            if complete_pending_by_id(c, log_id, now_ms())? == 0 {
                tracing::warn!(
                    workspace_id = id,
                    pending_log_id = log_id,
                    "workspace-purge: deletion log row was no longer pending"
                );
            }
        }
        None => record_log(
            c,
            id,
            &name,
            "completed",
            requested_by,
            None,
            requested_at.unwrap_or_else(now_ms),
        )?,
    }
    Ok(Final::Deleted)
}

/// `acquirePurgeFence`: the workspace when it is still due; sets
/// `purgeStartedAt` once (a started purge keeps its fence).
pub async fn acquire_purge_fence(db: &Db, workspace_id: i64) -> Result<Option<DueWorkspace>> {
    tx::write(db, move |c| {
        let sql = format!(
            r#"SELECT "id", "ownerId", "tenantId", "purgeStartedAt" FROM "Workspace" WHERE "id" = ?2 AND {DUE}"#
        );
        let found = c
            .query_row(&sql, params![now_ms(), workspace_id], |r| {
                Ok((due_row(r)?, r.get::<_, Option<i64>>(3)?))
            })
            .optional()?;
        let Some((workspace, started)) = found else {
            return Ok(None);
        };
        if started.is_none() {
            c.execute(
                r#"UPDATE "Workspace" SET "purgeStartedAt" = ?2 WHERE "id" = ?1"#,
                params![workspace_id, now_ms()],
            )?;
        }
        Ok(Some(workspace))
    })
    .await
}

fn iso(ms: i64) -> Value {
    json!(to_iso_string(ms))
}

fn opt_iso(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, iso)
}

fn period_json(periods: &[PeriodCount]) -> Value {
    periods
        .iter()
        .map(|p| json!({ "count": p.count, "periodEnd": iso(p.period_end), "periodStart": iso(p.period_start) }))
        .collect()
}

/// `workspaceDeletionRecordService.recordForWorkspace`: `None` when the
/// workspace is gone, else whether a record was created.
fn record_for_workspace(c: &Connection, workspace: DueWorkspace) -> repo::Result<Option<bool>> {
    let Some(sources) = records::read_snapshot_sources(c, workspace.owner_id, workspace.id)? else {
        return Ok(None);
    };
    let plan = sources.quota.as_ref().map(|q| {
        json!({
            "periodEnd": opt_iso(q.period_end),
            "periodStart": opt_iso(q.period_start),
            "planName": q.plan_name,
            "planStatus": q.plan_status,
        })
    });
    let owner_quota = sources.quota.as_ref().map(|q| {
        json!({
            "botMessagesUsed": q.bot_messages_used,
            "macUsed": q.mac_used,
            "monthlyBotMessagesUsed": q.monthly_bot_messages_used,
            "periodEnd": opt_iso(q.period_end),
            "periodStart": opt_iso(q.period_start),
            "planName": q.plan_name,
            "planStatus": q.plan_status,
        })
    });
    let usage_snapshot = json!({
        "botMessages": period_json(&sources.bot_messages),
        "channels": sources.inboxes.len(),
        "contacts": sources.contacts,
        "mac": period_json(&sources.mac),
        "ownerQuota": owner_quota,
        "teamMembers": sources.members.len(),
    });
    let ws = &sources.workspace;
    let owner = match &sources.owner {
        Some((email, name, tenant_id)) => json!({
            "email": email, "id": workspace.owner_id.to_string(), "name": name,
            "tenantId": tenant_id.to_string(),
        }),
        None => json!({
            "email": null, "id": workspace.owner_id.to_string(), "name": null, "tenantId": null,
        }),
    };
    let details = json!({
        "channels": sources.inboxes.iter().map(|i| json!({
            "channel": i.channel, "id": i.id.to_string(), "name": i.name, "status": i.status,
        })).collect::<Vec<_>>(),
        "members": sources.members.iter().map(|m| json!({
            "email": m.email, "name": m.name, "role": m.role, "userId": m.user_id.to_string(),
        })).collect::<Vec<_>>(),
        "owner": owner,
        "plan": plan,
        "requestedBy": sources.requested_by.as_ref().map(|(email, name)| json!({
            "email": email, "id": ws.deletion_requested_by_id.map(|id| id.to_string()),
            "name": name, "via": "ui",
        })),
        "workspace": {
            "createdAt": iso(ws.created_at), "id": ws.id.to_string(), "language": ws.language,
            "logo": ws.logo, "name": ws.name, "timezone": ws.timezone,
        },
    });
    let created = records::insert_if_absent(
        c,
        &NewDeletionRecord {
            deletion_requested_at: ws.deletion_requested_at,
            deletion_requested_by_email: sources
                .requested_by
                .as_ref()
                .map(|(email, _)| email.clone()),
            deletion_requested_by_id: ws.deletion_requested_by_id,
            details: repo::Json(details),
            owner_email: sources.owner.as_ref().map(|(email, _, _)| email.clone()),
            owner_id: workspace.owner_id,
            scheduled_deletion_at: ws.scheduled_deletion_at,
            tenant_id: workspace.tenant_id,
            usage_snapshot: repo::Json(usage_snapshot),
            workspace_id: workspace.id,
            workspace_name: ws.name.clone(),
        },
    )?;
    Ok(Some(created))
}
