//! Port of `packages/business/src/broadcast/service.ts` (the builder and
//! public API part; the worker part comes with the broadcast jobs).

pub mod audience;
pub mod history;
pub mod plan_policy;
pub mod recurrence;
pub mod send;
pub mod write;

use chatbotx_core::js::to_iso_string;
use chatbotx_db::Db;
use chatbotx_db::repo::broadcast::{
    self as repository, BroadcastListInput, BroadcastWithRelations, IdName, RecurrenceSummary,
};
use chatbotx_db::repo::broadcast_recurrence as recurrence_repo;
use chatbotx_db::repo::models::{Broadcast, BroadcastRecurrence};
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};

use self::plan_policy::RestrictedContext;
use self::recurrence::DisarmReason;
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// Next runs shown on a repeating broadcast's detail.
const UPCOMING_RUNS_IN_DETAIL: usize = 3;

/// `new ChatbotXException(message)` (`systemError`, 400).
pub(crate) fn chatbotx_exception(message: &str) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

/// `isRecurringTemplate` of a row.
pub(crate) fn is_recurring_template(broadcast: &Broadcast) -> bool {
    broadcast.schedules_type == "recurring" && broadcast.occurrence_at.is_none()
}

fn iso(ms: Option<i64>) -> Value {
    ms.and_then(to_iso_string)
        .map_or(Value::Null, Value::String)
}

fn id_name_json(value: &Option<IdName>) -> Value {
    value.as_ref().map_or(
        Value::Null,
        |v| json!({ "id": v.id.to_string(), "name": v.name }),
    )
}

fn recurrence_json(summary: &RecurrenceSummary, upcoming: Option<Vec<String>>) -> Value {
    let mut out = json!({
        "id": summary.id.to_string(),
        "rule": summary.rule.0,
        "nextRunAt": iso(summary.next_run_at),
        "nextSendAt": iso(recurrence::next_send_at_of(
            &summary.rule,
            summary.next_run_at,
            summary.occurrence_count,
        )),
        "occurrenceCount": summary.occurrence_count,
        "lastOccurrenceAt": iso(summary.last_occurrence_at),
        "pausedAt": iso(summary.paused_at),
        "endedAt": iso(summary.ended_at),
    });
    if let Some(upcoming) = upcoming {
        out["upcomingRuns"] = json!(upcoming);
    }
    out
}

/// A broadcast with its relations as the builder resource. `detail` adds the
/// next runs (`findByIdOrName`); a non-template row presents no schedule.
pub(crate) fn with_relations_json(row: &BroadcastWithRelations, detail: bool, list: bool) -> Value {
    let mut out = row.broadcast.to_json();
    let template = is_recurring_template(&row.broadcast);
    out["recurrence"] = match (&row.recurrence, template) {
        (Some(summary), true) => {
            let upcoming = detail.then(|| {
                match (summary.next_run_at, recurrence::stored_rule(&summary.rule)) {
                    (Some(_), Some(rule)) => recurrence::upcoming_runs_of(
                        &rule,
                        summary.occurrence_count,
                        now_ms(),
                        UPCOMING_RUNS_IN_DETAIL,
                    ),
                    _ => Vec::new(),
                }
            });
            recurrence_json(summary, upcoming)
        }
        _ => Value::Null,
    };
    out["parent"] = id_name_json(&row.parent);
    if list {
        out["flow"] = id_name_json(&row.flow);
        out["integrationWhatsapp"] = id_name_json(&row.integration_whatsapp);
        out["integrationMessenger"] = id_name_json(&row.integration_messenger);
    }
    out["targets"] = Value::Array(
        row.targets
            .iter()
            .map(|t| {
                let mut target = t.target.to_json();
                target["inbox"] = id_name_json(&t.inbox);
                target["flow"] = id_name_json(&t.flow);
                target
            })
            .collect(),
    );
    out
}

/// `list`: one page with relations and `pageCount`.
pub async fn list(db: &Db, input: BroadcastListInput) -> Result<Value> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let rows = repository::list_with_relations(conn, &input)?;
        let total = repository::count(conn, &input)?;
        Ok(json!({
            "data": rows.iter().map(|row| with_relations_json(row, false, true)).collect::<Vec<_>>(),
            "pageCount": (total + pagination.limit - 1) / pagination.limit,
        }))
    })
    .await
}

/// `findByIdOrName`: the detail resource; 404 `Broadcast not found`.
pub async fn find_by_id_or_name(db: &Db, workspace_id: i64, id_or_name: String) -> Result<Value> {
    tx::read(db, move |conn| {
        let row = repository::find_by_id_or_name(conn, workspace_id, &id_or_name)?
            .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?;
        Ok(with_relations_json(&row, true, false))
    })
    .await
}

/// `listAudience`: the recipients of a live broadcast with their contact.
pub async fn list_audience(
    db: &Db,
    workspace_id: i64,
    id_or_name: String,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput { page, per_page });
        let broadcast = repository::find_by_id_or_name(conn, workspace_id, &id_or_name)?
            .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?;
        let id = broadcast.broadcast.id;
        let rows = repository::list_audience(conn, id, pagination.limit, pagination.offset)?;
        let total = repository::count_audience(conn, id)?;
        let data = rows
            .iter()
            .map(|row| {
                let c = &row.contact;
                json!({
                    "contactId": row.row.contact_id.to_string(),
                    "contact": {
                        "id": c.id.to_string(),
                        "firstName": c.first_name,
                        "lastName": c.last_name,
                        "fullName": c.full_name,
                        "email": c.email,
                        "phoneNumber": c.phone_number,
                        "avatar": c.avatar,
                        "gender": c.gender,
                    },
                    "sent": row.row.sent,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "data": data, "pageCount": (total + pagination.limit - 1) / pagination.limit }))
    })
    .await
}

/// `listExistingIds`: live broadcasts of the workspace among `ids`.
pub fn list_existing_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Broadcast> = repo::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) AND \"deletedAt\" IS NULL",
        &[&workspace_id, &json!(ids).to_string()],
    )?;
    Ok(rows.into_iter().map(|b| b.id).collect())
}

/// `update`: renames a live broadcast (404 otherwise).
pub async fn update(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    name: String,
) -> Result<()> {
    tx::write(db, move |tx| {
        let found = repo::find_one::<Broadcast>(
            tx,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&id, &workspace_id],
        )?;
        if found.is_none() {
            return Err(BusinessError::not_found("Broadcast not found"));
        }
        tx.execute(
            r#"UPDATE "Broadcast" SET "name" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
            chatbotx_db::rusqlite::params![name, now_ms(), id],
        )?;
        Ok(())
    })
    .await?;
    audit
        .audit("update", &format!("updated a broadcast (#{id})"))
        .await;
    Ok(())
}

/// One guarded `UPDATE ... RETURNING "id"`: `Some(id)` when a row moved.
pub(crate) fn guarded_update(
    conn: &Connection,
    set: &str,
    scope: &str,
    params: &[SqlValue],
) -> Result<Option<i64>> {
    let sql = format!(
        r#"UPDATE "Broadcast" SET {set}, "updatedAt" = {now} WHERE {scope} RETURNING "id""#,
        now = now_ms()
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(chatbotx_db::rusqlite::params_from_iter(params.iter()))?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// `transitionScope(workspaceId, broadcastId, fromStatus)` over `?1` id, `?2` workspace.
fn transition_scope(from_status: &str) -> String {
    format!(
        r#""id" = ?1 AND "workspaceId" = ?2 AND "status" = '{from_status}' AND "deletedAt" IS NULL"#
    )
}

fn ids(workspace_id: i64, broadcast_id: i64) -> Vec<SqlValue> {
    vec![
        SqlValue::Integer(broadcast_id),
        SqlValue::Integer(workspace_id),
    ]
}

/// `lockForWrite` of one series.
pub(crate) fn lock_series(
    conn: &Connection,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<Option<BroadcastRecurrence>> {
    Ok(
        recurrence_repo::lock_for_write(conn, workspace_id, &[broadcast_id])?
            .into_iter()
            .next(),
    )
}

/// `transitionRecurringTemplate`: lock the series, then move the template row.
fn transition_recurring_template(
    conn: &Connection,
    workspace_id: i64,
    broadcast_id: i64,
    from: &[&str],
    set: &str,
    failure: &str,
) -> Result<(BroadcastRecurrence, Broadcast)> {
    let series = lock_series(conn, workspace_id, broadcast_id)?
        .ok_or_else(|| chatbotx_exception(failure))?;
    let from = from
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let scope = format!(
        r#""id" = ?1 AND "workspaceId" = ?2 AND "status" IN ({from}) AND "schedulesType" = 'recurring' AND "occurrenceAt" IS NULL AND "deletedAt" IS NULL"#
    );
    guarded_update(conn, set, &scope, &ids(workspace_id, broadcast_id))?
        .ok_or_else(|| chatbotx_exception(failure))?;
    let row = repo::find_one::<Broadcast>(conn, r#""id" = ?1"#, &[&broadcast_id])?
        .ok_or_else(|| chatbotx_exception(failure))?;
    Ok((series, row))
}

/// `moveToDraft`: `scheduled` -> `draft` (bumps `resumeCount`), else a
/// repeating broadcast back to draft with its series stopped.
pub async fn move_to_draft(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<Value> {
    let id = tx::write(db, move |tx| {
        let scope = format!(r#"{} AND "occurrenceAt" IS NULL"#, transition_scope("scheduled"));
        let set = r#""status" = 'draft', "contactCount" = NULL, "handoffCompletedAt" = NULL, "resumeCount" = "resumeCount" + 1"#;
        if let Some(id) = guarded_update(tx, set, &scope, &ids(workspace_id, broadcast_id))? {
            return Ok(id);
        }
        let (series, row) = transition_recurring_template(
            tx,
            workspace_id,
            broadcast_id,
            &["recurring", "cancelled"],
            r#""status" = 'draft', "resumeCount" = "resumeCount" + 1"#,
            "Broadcast is no longer scheduled",
        )?;
        recurrence::disarm(tx, &series, DisarmReason::Draft, recurrence_repo::select_db_now(tx)?)?;
        Ok(row.id)
    })
    .await?;
    audit
        .audit(
            "broadcast_moved_to_draft",
            &format!("moved broadcast (#{id}) to draft"),
        )
        .await;
    Ok(json!({ "id": id.to_string() }))
}

/// `stopSending`: `sending` -> `cancelled`, else pauses a repeating broadcast.
pub async fn stop_sending(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<Value> {
    let (id, paused) = tx::write(db, move |tx| {
        if let Some(id) = guarded_update(
            tx,
            r#""status" = 'cancelled'"#,
            &transition_scope("sending"),
            &ids(workspace_id, broadcast_id),
        )? {
            return Ok((id, false));
        }
        let (series, row) = transition_recurring_template(
            tx,
            workspace_id,
            broadcast_id,
            &["recurring"],
            r#""status" = 'cancelled'"#,
            "Broadcast is not in progress",
        )?;
        recurrence::disarm(
            tx,
            &series,
            DisarmReason::Paused,
            recurrence_repo::select_db_now(tx)?,
        )?;
        Ok((row.id, true))
    })
    .await?;
    let verb = if paused { "paused" } else { "stopped" };
    audit
        .audit("broadcast_stopped", &format!("{verb} a broadcast (#{id})"))
        .await;
    Ok(json!({ "id": id.to_string() }))
}

/// The restriction a repeating template is checked against (rate only).
pub(crate) fn template_restriction(workspace_id: i64, channel: &str) -> Option<RestrictedContext> {
    if !plan_policy::applies_to_channel(channel) {
        return None;
    }
    plan_policy::restriction_for(&plan_policy::resolve_for_workspace(workspace_id), channel)
}

/// `activateRecurringTemplate`: arm the series, flip the template to `recurring`.
pub(crate) fn activate_recurring_template(
    conn: &Connection,
    workspace_id: i64,
    scope: &str,
    scope_params: &[SqlValue],
    broadcast: &Broadcast,
    submitted_send_rate: Option<Option<i64>>,
    series: &BroadcastRecurrence,
) -> Result<i64> {
    let now = recurrence_repo::select_db_now(conn)?;
    let rate = chatbotx_db::repo::partials::broadcast::resolve_activation_send_rate(
        submitted_send_rate,
        broadcast.send_rate_per_minute,
    );
    let restriction = template_restriction(workspace_id, &broadcast.channel);
    let armed = recurrence::arm(conn, series, rate, restriction.as_ref(), now)?;
    let rate_patch = match armed.send_rate_override {
        Some(rate) => Some(Some(rate)),
        None => submitted_send_rate,
    };
    let mut set = format!(
        r#""status" = 'recurring', "schedulesType" = 'recurring', "schedulesAt" = {}"#,
        armed.next_run_at
    );
    if let Some(rate) = rate_patch {
        set.push_str(&format!(
            r#", "sendRatePerMinute" = {}"#,
            rate.map_or("NULL".to_owned(), |r| r.to_string())
        ));
    }
    guarded_update(conn, &set, scope, scope_params)?
        .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))
}

/// `resumeSending`: a paused repeating broadcast restarts its schedule
/// (`nextRunAt`); else `cancelled` -> `sending` for a prepared broadcast.
pub async fn resume_sending(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    broadcast_id: i64,
    send_rate_per_minute: Option<Option<i64>>,
) -> Result<Value> {
    let result = tx::write(db, move |tx| {
        let found = repo::find_one::<Broadcast>(
            tx,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&broadcast_id, &workspace_id],
        )?;
        if found.as_ref().is_some_and(is_recurring_template) {
            let (series, row) = transition_recurring_template(
                tx,
                workspace_id,
                broadcast_id,
                &["cancelled"],
                r#""status" = 'recurring'"#,
                "Broadcast is not stopped",
            )?;
            let id = activate_recurring_template(
                tx,
                workspace_id,
                r#""id" = ?1"#,
                &[SqlValue::Integer(row.id)],
                &row,
                send_rate_per_minute,
                &series,
            )?;
            let row = repo::find_one::<Broadcast>(tx, r#""id" = ?1"#, &[&id])?
                .ok_or_else(|| chatbotx_exception("Broadcast is not stopped"))?;
            return Ok(json!({ "id": id.to_string(), "nextRunAt": iso(Some(row.schedules_at)) }));
        }
        let scope = format!(r#"{} AND "contactCount" IS NOT NULL"#, transition_scope("cancelled"));
        let ctx = plan_policy::resolve_for_workspace(workspace_id);
        let mut rate_patch = send_rate_per_minute;
        if plan_policy::has_restrictions(&ctx)
            && let Some(stopped) = found.as_ref().filter(|b| b.status == "cancelled" && b.contact_count.is_some())
            && let Some(restriction) = plan_policy::restriction_for(&ctx, &stopped.channel)
        {
            let rate = chatbotx_db::repo::partials::broadcast::resolve_activation_send_rate(
                send_rate_per_minute,
                stopped.send_rate_per_minute,
            );
            rate_patch = Some(Some(plan_policy::assert_activation_allowed(
                tx,
                workspace_id,
                &stopped.channel,
                &restriction,
                rate,
                Some(stopped.id),
            )?));
        }
        let mut set = r#""status" = 'sending', "handoffCompletedAt" = NULL, "resumeCount" = "resumeCount" + 1"#.to_owned();
        if let Some(rate) = rate_patch {
            set.push_str(&format!(
                r#", "sendRatePerMinute" = {}"#,
                rate.map_or("NULL".to_owned(), |r| r.to_string())
            ));
        }
        let id = guarded_update(tx, &set, &scope, &ids(workspace_id, broadcast_id))?
            .ok_or_else(|| chatbotx_exception("Broadcast is not stopped"))?;
        Ok(json!({ "id": id.to_string() }))
    })
    .await?;
    let id = result["id"].as_str().unwrap_or_default().to_owned();
    audit
        .audit("broadcast_resumed", &format!("resumed a broadcast (#{id})"))
        .await;
    Ok(result)
}

/// `softDeleteBroadcasts`: stamps `deletedAt` (never on a `sending` row);
/// a deleted series is stopped and its idle runs deleted too.
/// Returns `(deletedCount, requestedCount)`.
pub async fn soft_delete_broadcasts(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    ids: Vec<i64>,
) -> Result<(usize, usize)> {
    let requested = ids.len();
    if requested == 0 {
        return Ok((0, 0));
    }
    let deleted = tx::write(db, move |tx| {
        let series = recurrence_repo::lock_for_write(tx, workspace_id, &ids)?;
        let now = now_ms();
        let mut stmt = tx.prepare(
            r#"UPDATE "Broadcast" SET "deletedAt" = ?1, "updatedAt" = ?1
               WHERE "workspaceId" = ?2 AND "id" IN (SELECT value FROM json_each(?3))
                 AND "status" <> 'sending' AND "deletedAt" IS NULL RETURNING "id""#,
        )?;
        let deleted = stmt
            .query_map(
                chatbotx_db::rusqlite::params![now, workspace_id, json!(ids).to_string()],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo::Error::from)?;
        drop(stmt);
        let retired: Vec<&BroadcastRecurrence> = series
            .iter()
            .filter(|s| deleted.contains(&s.broadcast_id))
            .collect();
        if !retired.is_empty() {
            let db_now = recurrence_repo::select_db_now(tx)?;
            for series in &retired {
                recurrence::disarm(tx, series, DisarmReason::Deleted, db_now)?;
            }
            let parents =
                json!(retired.iter().map(|s| s.broadcast_id).collect::<Vec<_>>()).to_string();
            tx.execute(
                r#"UPDATE "Broadcast" SET "deletedAt" = ?1, "updatedAt" = ?1
                   WHERE "parentBroadcastId" IN (SELECT value FROM json_each(?2))
                     AND "status" <> 'sending' AND "deletedAt" IS NULL"#,
                chatbotx_db::rusqlite::params![db_now, parents],
            )?;
        }
        Ok(deleted.len())
    })
    .await?;
    if deleted > 0 {
        audit
            .audit("delete", &format!("deleted {deleted} broadcast(s)"))
            .await;
    }
    Ok((deleted, requested))
}

/// `OPTION_LIST_LIMIT`.
const OPTION_LIST_LIMIT: i64 = 500;

/// `listOptions`: `{ id, name }` of the channel's broadcasts, newest first.
pub async fn list_options(db: &Db, workspace_id: i64, channel: String) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "id", "name" FROM "Broadcast" WHERE "workspaceId" = ?1 AND "channel" = ?2
               AND "deletedAt" IS NULL ORDER BY "createdAt" DESC LIMIT ?3"#,
        )?;
        let rows = stmt
            .query_map(chatbotx_db::rusqlite::params![workspace_id, channel, OPTION_LIST_LIMIT], |row| {
                Ok(json!({ "id": row.get::<_, i64>(0)?.to_string(), "name": row.get::<_, String>(1)? }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo::Error::from)?;
        Ok(rows)
    })
    .await
}
