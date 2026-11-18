//! `packages/business/src/sequence/{service,step-payload,contact-schedule}.ts`
//! (P3.6-D2 `create`; Q.D5 the rest). Queries on `"Sequence"`,
//! `"SequenceStep"`, `"ContactOnSequence"`, `"Folder"`, `"Flow"`; dispatches
//! through `chatbotx_jobs::sequence` (row + schedule entry in the same write
//! transaction). No template locks / delete guards (P3.6-D1). Audit details
//! are returned to the caller. `listStepContactsPage` needs the sequence
//! analytics (Q.D11).

use chatbotx_db::repo::models::{Sequence, SequenceStep};
use chatbotx_db::repo::sequence::{SequenceListInput, SequenceWithCounts, SequenceWithSteps};
use chatbotx_db::repo::{
    self, Model, PaginationInput, get_pagination_with_defaults, new_id, now_ms,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::sequence::dispatch::{
    CreateDispatchParams, SqlDispatchInserter, create_dispatch_in,
};
use chatbotx_jobs::sequence::repo::CreateId;
use chatbotx_jobs::sequence::sqlite_schedule::{schedule_in, wake_producer};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn name_taken(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = DbError::from(error);
    if error.sqlstate() == Some("23505") {
        BusinessError::validation("name", "Name is already taken.")
    } else {
        BusinessError::Repo(repo::Error::Db(error))
    }
}

/// `assertSequenceFolder`: a folder id must be a sequence folder of the
/// workspace (404 "Folder not found"); `None` = the root.
fn assert_sequence_folder(
    conn: &Connection,
    workspace_id: i64,
    folder_id: Option<i64>,
) -> Result<()> {
    let Some(folder_id) = folder_id else {
        return Ok(());
    };
    conn.query_row(
        r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'sequence'"#,
        params![folder_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Folder not found"))
}

/// `create`: a `folderId` must be a sequence folder of the workspace (404
/// "Folder not found"); a taken name is a `name` validation error.
pub async fn create(db: &Db, workspace_id: i64, name: &str, folder_id: Option<i64>) -> Result<i64> {
    let name = name.to_string();
    tx::write(db, move |tx| {
        assert_sequence_folder(tx, workspace_id, folder_id)?;
        let id = new_id();
        tx.execute(
            r#"INSERT INTO "Sequence" ("id", "name", "folderId", "workspaceId") VALUES (?1, ?2, ?3, ?4)"#,
            params![id, name, folder_id, workspace_id],
        )
        .map_err(name_taken)?;
        Ok(id)
    })
    .await
}

/// `list`: the sequences with `stepsCount` / `subscribersCount` and
/// `pageCount`.
pub async fn list(db: &Db, input: SequenceListInput) -> Result<(Vec<SequenceWithCounts>, i64)> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let data = repo::sequence::list_with_counts(conn, &input)?;
        let total = repo::sequence::count(conn, &input)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            0
        };
        Ok((data, page_count))
    })
    .await
}

fn find_sequence(conn: &Connection, workspace_id: i64, id: i64) -> Result<Sequence> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            Sequence::select_sql()
        ),
        params![id, workspace_id],
        Sequence::from_row,
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Sequence not found"))
}

/// `assertOwned`: 404 "Sequence not found".
pub async fn assert_owned(db: &Db, workspace_id: i64, sequence_id: i64) -> Result<Sequence> {
    tx::read(db, move |conn| {
        find_sequence(conn, workspace_id, sequence_id)
    })
    .await
}

/// `findWithSteps`: 404 "Sequence not found".
pub async fn find_with_steps(db: &Db, workspace_id: i64, id: i64) -> Result<SequenceWithSteps> {
    tx::read(db, move |conn| {
        repo::sequence::find_with_steps(conn, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("Sequence not found"))
    })
    .await
}

/// `update` data (`updateSequenceSchema`, all optional; `folder_id:
/// Some(None)` = the root).
#[derive(Debug, Clone, Default)]
pub struct UpdateSequence {
    pub name: Option<String>,
    pub active: Option<bool>,
    pub folder_id: Option<Option<i64>>,
}

/// `update`: a partial write of the changed keys; `None` = nothing changed
/// (no audit), else the audit detail ("enabled / disabled / updated a
/// sequence (#id)").
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: UpdateSequence,
) -> Result<Option<String>> {
    tx::write(db, move |tx| {
        crate::template::guard::assert_content_editable(tx, workspace_id, "sequence", &[id])?;
        let sequence = find_sequence(tx, workspace_id, id)?;
        if let Some(folder_id) = data.folder_id
            && folder_id != sequence.folder_id
        {
            assert_sequence_folder(tx, workspace_id, folder_id)?;
        }
        let mut changed: Vec<&str> = Vec::new();
        if data.name.as_ref().is_some_and(|n| *n != sequence.name) {
            changed.push("name");
        }
        if data.active.is_some_and(|a| a != sequence.active) {
            changed.push("active");
        }
        if data.folder_id.is_some_and(|f| f != sequence.folder_id) {
            changed.push("folderId");
        }
        if changed.is_empty() {
            return Ok(None);
        }
        // `.set(data)`: every given key is written (unchanged ones too).
        let name = data.name.clone().unwrap_or(sequence.name.clone());
        let active = data.active.unwrap_or(sequence.active);
        let folder_id = data.folder_id.unwrap_or(sequence.folder_id);
        let updated = tx
            .execute(
                r#"UPDATE "Sequence" SET "name" = ?1, "active" = ?2, "folderId" = ?3, "updatedAt" = ?4
                   WHERE "id" = ?5 AND "workspaceId" = ?6"#,
                params![name, active, folder_id, now_ms(), id, workspace_id],
            )
            .map_err(name_taken)?;
        if updated == 0 {
            return Ok(None);
        }
        Ok(Some(if changed == ["active"] {
            if active {
                format!("enabled a sequence (#{id})")
            } else {
                format!("disabled a sequence (#{id})")
            }
        } else {
            format!("updated a sequence (#{id})")
        }))
    })
    .await
}

/// `delete`: 404 "Sequence not found"; the steps and enrollments cascade.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        crate::template::guard::assert_deletable(tx, workspace_id, "sequence", &[id])?;
        find_sequence(tx, workspace_id, id)?;
        tx.execute(
            r#"DELETE FROM "Sequence" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `listLabelsByIds`: `(id, name)` of the existing sequences among `ids`.
pub async fn list_labels_by_ids(
    db: &Db,
    workspace_id: i64,
    ids: Vec<i64>,
) -> Result<Vec<(i64, String)>> {
    tx::read(db, move |conn| {
        let mut out = Vec::new();
        for id in ids {
            if let Ok(sequence) = find_sequence(conn, workspace_id, id) {
                out.push((sequence.id, sequence.name));
            }
        }
        Ok(out)
    })
    .await
}

// ---- steps ------------------------------------------------------------------

/// `SEQUENCE_STEP_DELAY_UNITS`.
pub const DELAY_UNITS: &[&str] = &["immediate", "minutes", "hours", "days", "specificTime"];

/// `isStoredDelayConsistent`.
pub fn is_stored_delay_consistent(unit: &str, delay_days: i64, delay_minutes: i64) -> bool {
    match unit {
        "immediate" | "specificTime" => delay_days == 0 && delay_minutes == 0,
        "minutes" => delay_days == 0 && delay_minutes > 0,
        "hours" => delay_days == 0 && delay_minutes > 0 && delay_minutes % 60 == 0,
        "days" => delay_minutes == 0 && delay_days > 0,
        _ => false,
    }
}

/// `SequenceStepPayloadInput` (absent = `None`; a nullable key sent as
/// `null` = `Some(None)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepPayload {
    pub order: i64,
    pub delay_days: Option<i64>,
    pub delay_minutes: Option<i64>,
    pub delay_unit: Option<String>,
    pub flow_id: Option<i64>,
    /// Unix ms.
    pub specific_date_time: Option<Option<i64>>,
    pub is_active: Option<bool>,
    pub anytime: Option<bool>,
    pub send_time_start: Option<Option<String>>,
    pub send_time_end: Option<Option<String>>,
    pub send_days: Option<Vec<String>>,
}

/// `sendTimeStart || null`.
fn blank_null(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

fn send_days_json(days: &[String]) -> String {
    serde_json::to_string(days).unwrap_or_else(|_| "[]".to_string())
}

/// `buildCreateData` + insert.
fn insert_step(conn: &Connection, sequence_id: i64, data: &StepPayload) -> Result<i64> {
    let id = new_id();
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "SequenceStep" ("id", "createdAt", "updatedAt", "sequenceId", "order", "delayDays", "delayMinutes", "delayUnit", "flowId", "specificDateTime", "isActive", "anytime", "sendTimeStart", "sendTimeEnd", "sendDays")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
        params![
            id,
            now,
            sequence_id,
            data.order,
            data.delay_days.unwrap_or(1),
            data.delay_minutes.unwrap_or(0),
            data.delay_unit.clone().unwrap_or_else(|| "days".to_string()),
            data.flow_id,
            data.specific_date_time.flatten(),
            data.is_active.unwrap_or(true),
            data.anytime.unwrap_or(true),
            blank_null(data.send_time_start.clone().flatten()),
            blank_null(data.send_time_end.clone().flatten()),
            data.send_days.as_deref().map(send_days_json),
        ],
    )
    .map_err(repo_err)?;
    Ok(id)
}

/// `buildUpdateData` + update: `order` always, the other keys when given.
fn update_step_row(conn: &Connection, step_id: i64, data: &StepPayload) -> Result<()> {
    use chatbotx_db::rusqlite::types::Value as Sql;
    let mut sets: Vec<(&str, Sql)> = vec![("order", Sql::Integer(data.order))];
    if let Some(v) = data.delay_days {
        sets.push(("delayDays", Sql::Integer(v)));
    }
    if let Some(v) = data.delay_minutes {
        sets.push(("delayMinutes", Sql::Integer(v)));
    }
    if let Some(v) = &data.delay_unit {
        sets.push(("delayUnit", Sql::Text(v.clone())));
    }
    if let Some(v) = data.flow_id {
        sets.push(("flowId", Sql::Integer(v)));
    }
    if let Some(v) = data.specific_date_time {
        sets.push(("specificDateTime", v.map_or(Sql::Null, Sql::Integer)));
    }
    if let Some(v) = data.is_active {
        sets.push(("isActive", Sql::Integer(i64::from(v))));
    }
    if let Some(v) = data.anytime {
        sets.push(("anytime", Sql::Integer(i64::from(v))));
    }
    if let Some(v) = &data.send_time_start {
        sets.push((
            "sendTimeStart",
            blank_null(v.clone()).map_or(Sql::Null, Sql::Text),
        ));
    }
    if let Some(v) = &data.send_time_end {
        sets.push((
            "sendTimeEnd",
            blank_null(v.clone()).map_or(Sql::Null, Sql::Text),
        ));
    }
    if let Some(v) = &data.send_days {
        sets.push(("sendDays", Sql::Text(send_days_json(v))));
    }
    sets.push(("updatedAt", Sql::Integer(now_ms())));
    let assignments = sets
        .iter()
        .enumerate()
        .map(|(i, (column, _))| format!(r#""{column}" = ?{}"#, i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let mut binds: Vec<Sql> = sets.into_iter().map(|(_, v)| v).collect();
    binds.push(Sql::Integer(step_id));
    conn.execute(
        &format!(
            r#"UPDATE "SequenceStep" SET {assignments} WHERE "id" = ?{}"#,
            binds.len()
        ),
        chatbotx_db::rusqlite::params_from_iter(binds.iter()),
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `assertStepFlowInWorkspace`: 404 "Flow not found".
fn assert_step_flow(conn: &Connection, workspace_id: i64, flow_id: Option<i64>) -> Result<()> {
    let Some(flow_id) = flow_id else {
        return Ok(());
    };
    conn.query_row(
        r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![flow_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Flow not found"))
}

/// The step `step_id` of the workspace (and of `sequence_id` when given):
/// 404 "Step not found".
fn find_step(
    conn: &Connection,
    workspace_id: i64,
    step_id: i64,
    sequence_id: Option<i64>,
) -> Result<SequenceStep> {
    let step = conn
        .query_row(
            &format!(
                r#"{} WHERE "id" = ?1 AND "sequenceId" IN (SELECT "id" FROM "Sequence" WHERE "workspaceId" = ?2)"#,
                SequenceStep::select_sql()
            ),
            params![step_id, workspace_id],
            SequenceStep::from_row,
        )
        .optional()
        .map_err(repo_err)?
        .ok_or_else(|| BusinessError::not_found("Step not found"))?;
    if sequence_id.is_some_and(|id| id != step.sequence_id) {
        return Err(BusinessError::not_found("Step not found"));
    }
    Ok(step)
}

/// `shouldRecalculateOnUpdate`.
fn should_recalculate_on_update(data: &StepPayload, previous_order: i64) -> bool {
    data.delay_days.is_some()
        || data.delay_minutes.is_some()
        || data.delay_unit.is_some()
        || data.is_active.is_some()
        || data.order != previous_order
}

/// `upsertStep`: updates `step_id` (pinned to the sequence) or creates a
/// step, then recalculates the contact schedules when timing changed. The
/// caller checked the sequence (`assertOwned`). Returns the step id.
pub async fn upsert_step(
    db: &Db,
    workspace_id: i64,
    sequence_id: i64,
    step_id: Option<i64>,
    data: StepPayload,
) -> Result<i64> {
    let id = tx::write(db, move |tx| {
        assert_step_flow(tx, workspace_id, data.flow_id)?;
        let now = now_ms();
        if let Some(step_id) = step_id {
            let step = find_step(tx, workspace_id, step_id, Some(sequence_id))?;
            update_step_row(tx, step_id, &data)?;
            if should_recalculate_on_update(&data, step.order) {
                handle_step_update_impact(tx, sequence_id, workspace_id, step_id, data.order, now)?;
            }
            return Ok(step_id);
        }
        let id = insert_step(tx, sequence_id, &data)?;
        handle_step_creation_impact(tx, sequence_id, workspace_id, data.order, now)?;
        Ok(id)
    })
    .await?;
    wake_producer();
    Ok(id)
}

/// `deleteStep`: 404 "Step not found"; then recalculates every active
/// enrollment of the sequence.
pub async fn delete_step(
    db: &Db,
    workspace_id: i64,
    step_id: i64,
    sequence_id: Option<i64>,
) -> Result<()> {
    tx::write(db, move |tx| {
        let step = find_step(tx, workspace_id, step_id, sequence_id)?;
        tx.execute(
            r#"DELETE FROM "SequenceStep" WHERE "id" = ?1"#,
            params![step_id],
        )
        .map_err(repo_err)?;
        recalculate_all_contacts_in_sequence(tx, step.sequence_id, workspace_id, now_ms())
    })
    .await
}

// ---- contact schedule (`contact-schedule.ts`) -------------------------------

/// A step as the schedule reads it.
struct ActiveStep {
    id: i64,
    order: i64,
    delay_days: i64,
    delay_minutes: i64,
    delay_unit: Option<String>,
    specific_date_time: Option<i64>,
}

fn delay_ms(days: i64, minutes: i64) -> i64 {
    days * 86_400_000 + minutes * 60_000
}

fn active_steps(conn: &Connection, sequence_id: i64) -> Result<Vec<ActiveStep>> {
    let mut stmt = conn
        .prepare(
            r#"SELECT "id", "order", "delayDays", "delayMinutes", "delayUnit", "specificDateTime"
               FROM "SequenceStep" WHERE "sequenceId" = ?1 AND "isActive" = 1 ORDER BY "order" ASC, "id" ASC"#,
        )
        .map_err(repo_err)?;
    let rows = stmt
        .query_map(params![sequence_id], |r| {
            Ok(ActiveStep {
                id: r.get(0)?,
                order: r.get(1)?,
                delay_days: r.get(2)?,
                delay_minutes: r.get(3)?,
                delay_unit: r.get(4)?,
                specific_date_time: r.get(5)?,
            })
        })
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(rows)
}

/// The delay of the steps up to `up_to_order`: `Fixed(at)` for a
/// specific-time target, `Relative(ms)` else; `None` = no such step (-1).
enum Delay {
    Fixed(i64),
    Relative(i64),
}

fn cumulative_delay(steps: &[ActiveStep], up_to_order: i64) -> Option<Delay> {
    let upto: Vec<&ActiveStep> = steps.iter().filter(|s| s.order <= up_to_order).collect();
    let target = upto.last()?;
    if target.delay_unit.as_deref() == Some("specificTime")
        && let Some(at) = target.specific_date_time
    {
        return Some(Delay::Fixed(at));
    }
    Some(Delay::Relative(
        upto.iter()
            .map(|s| delay_ms(s.delay_days, s.delay_minutes))
            .sum(),
    ))
}

const ACTIVE_AT_STEP: &str = r#""sequenceId" = ?1 AND "workspaceId" = ?2 AND "currentStep" = ?3 AND "status" = 'active' AND "completedAt" IS NULL"#;

/// `recalculateNextRunAtForStep`.
fn recalculate_for_step(
    conn: &Connection,
    sequence_id: i64,
    workspace_id: i64,
    step_order: i64,
    now: i64,
) -> Result<()> {
    let steps = active_steps(conn, sequence_id)?;
    let Some(next) = steps.iter().find(|s| s.order > step_order) else {
        conn.execute(
            &format!(
                r#"UPDATE "ContactOnSequence" SET "status" = 'completed', "completedAt" = ?4, "nextRunAt" = NULL, "nextStepId" = NULL, "updatedAt" = ?4 WHERE {ACTIVE_AT_STEP}"#
            ),
            params![sequence_id, workspace_id, step_order, now],
        )
        .map_err(repo_err)?;
        return Ok(());
    };
    // `NOW() + INTERVAL '<delay> milliseconds'`.
    let next_run_at = match cumulative_delay(&steps, next.order) {
        None => None,
        Some(Delay::Fixed(at)) => Some(at),
        Some(Delay::Relative(ms)) => Some(now + ms),
    };
    conn.execute(
        &format!(
            r#"UPDATE "ContactOnSequence" SET "nextRunAt" = ?4, "nextStepId" = ?5, "updatedAt" = ?6 WHERE {ACTIVE_AT_STEP}"#
        ),
        params![sequence_id, workspace_id, step_order, next_run_at, next.id, now],
    )
    .map_err(repo_err)?;
    Ok(())
}

fn distinct_current_steps(
    conn: &Connection,
    sql_filter: &str,
    binds: &[&dyn chatbotx_db::rusqlite::ToSql],
) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare(&format!(
            r#"SELECT DISTINCT "currentStep" FROM "ContactOnSequence" WHERE "sequenceId" = ?1 AND "workspaceId" = ?2 AND "status" = 'active' AND "completedAt" IS NULL {sql_filter} ORDER BY "currentStep""#
        ))
        .map_err(repo_err)?;
    let rows = stmt
        .query_map(binds, |r| r.get::<_, i64>(0))
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(rows)
}

/// `recalculateAllContactsInSequence`.
pub fn recalculate_all_contacts_in_sequence(
    conn: &Connection,
    sequence_id: i64,
    workspace_id: i64,
    now: i64,
) -> Result<()> {
    for step in distinct_current_steps(conn, "", &[&sequence_id, &workspace_id])? {
        recalculate_for_step(conn, sequence_id, workspace_id, step, now)?;
    }
    Ok(())
}

/// `createAndScheduleDispatch`: one dispatch per contact inbox, each added
/// to the schedule.
fn create_and_schedule_dispatch(
    conn: &Connection,
    workspace_id: i64,
    sequence_id: i64,
    contact_id: i64,
    step_id: i64,
    enrollment_id: i64,
    run_at_ms: i64,
) -> Result<()> {
    let inboxes = chatbotx_jobs::sequence::contacts::get_contact_inboxes_in(
        conn,
        None,
        &workspace_id.to_string(),
        &contact_id.to_string(),
    )
    .map_err(|e| BusinessError::Repo(repo::Error::msg(format!("{e:?}"))))?;
    for inbox in inboxes {
        let dispatch = create_dispatch_in(
            conn,
            &SqlDispatchInserter,
            &CreateId,
            &CreateDispatchParams {
                workspace_id: workspace_id.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: inbox.id.clone(),
                enrollment_id: enrollment_id.to_string(),
                run_at_ms,
                sequence_id: sequence_id.to_string(),
                step_id: step_id.to_string(),
            },
        )
        .map_err(|e| BusinessError::Repo(repo::Error::msg(format!("{e:?}"))))?;
        schedule_in(conn, dispatch.bucket, &dispatch.id, dispatch.run_at_ms)
            .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;
    }
    Ok(())
}

/// `reactivateCompletedContactsForNewStep`.
fn reactivate_completed(
    conn: &Connection,
    sequence_id: i64,
    workspace_id: i64,
    new_step_order: i64,
    now: i64,
) -> Result<()> {
    let completed: Vec<(i64, i64, i64, i64)> = {
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "contactId", "currentStep", "enrolledAt" FROM "ContactOnSequence"
                   WHERE "sequenceId" = ?1 AND "workspaceId" = ?2 AND "status" = 'completed' AND "currentStep" <= ?3 ORDER BY "id""#,
            )
            .map_err(repo_err)?;
        stmt.query_map(params![sequence_id, workspace_id, new_step_order], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?
    };
    if completed.is_empty() {
        return Ok(());
    }
    let steps = active_steps(conn, sequence_id)?;
    if steps.is_empty() {
        return Ok(());
    }
    for (enrollment_id, contact_id, current_step, enrolled_at) in completed {
        let Some(next) = steps.iter().find(|s| s.order >= current_step) else {
            continue;
        };
        let next_run_at = match cumulative_delay(&steps, next.order) {
            Some(Delay::Fixed(at)) => Some(at),
            Some(Delay::Relative(ms)) if ms > 0 => Some(enrolled_at + ms),
            _ => None,
        };
        conn.execute(
            r#"UPDATE "ContactOnSequence" SET "status" = 'active', "completedAt" = NULL, "nextStepId" = ?1, "nextRunAt" = ?2, "updatedAt" = ?3
               WHERE "id" = ?4 AND "workspaceId" = ?5"#,
            params![next.id, next_run_at, now, enrollment_id, workspace_id],
        )
        .map_err(repo_err)?;
        if let Some(run_at) = next_run_at {
            create_and_schedule_dispatch(
                conn,
                workspace_id,
                sequence_id,
                contact_id,
                next.id,
                enrollment_id,
                run_at,
            )?;
        }
    }
    Ok(())
}

/// `handleStepCreationImpact`.
pub fn handle_step_creation_impact(
    conn: &Connection,
    sequence_id: i64,
    workspace_id: i64,
    new_step_order: i64,
    now: i64,
) -> Result<()> {
    for step in distinct_current_steps(
        conn,
        r#"AND "currentStep" <= ?3"#,
        &[&sequence_id, &workspace_id, &new_step_order],
    )? {
        recalculate_for_step(conn, sequence_id, workspace_id, step, now)?;
    }
    reactivate_completed(conn, sequence_id, workspace_id, new_step_order, now)
}

/// `handleStepUpdateImpact`.
pub fn handle_step_update_impact(
    conn: &Connection,
    sequence_id: i64,
    workspace_id: i64,
    updated_step_id: i64,
    updated_step_order: i64,
    now: i64,
) -> Result<()> {
    let mut steps = distinct_current_steps(
        conn,
        r#"AND "nextStepId" = ?3"#,
        &[&sequence_id, &workspace_id, &updated_step_id],
    )?;
    for step in distinct_current_steps(
        conn,
        r#"AND "currentStep" < ?3"#,
        &[&sequence_id, &workspace_id, &updated_step_order],
    )? {
        if !steps.contains(&step) {
            steps.push(step);
        }
    }
    for step in steps {
        recalculate_for_step(conn, sequence_id, workspace_id, step, now)?;
    }
    reactivate_completed(conn, sequence_id, workspace_id, updated_step_order, now)
}
