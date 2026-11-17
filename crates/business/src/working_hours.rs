//! `packages/business/src/working-hours/service.ts` (the CRUD part; the
//! on-shift / duration helpers live in the worker). Inputs arrive already
//! checked by the request schemas (`features/working-hours/schema/action.ts`).
//!
//! ponytail: no read cache (`withCache` + tag invalidation): SQLite reads are
//! local, so every read is fresh and nothing needs invalidating.

use chatbotx_db::Db;
use chatbotx_db::repo::models::WorkingHoursDateOverride;
use chatbotx_db::repo::working_hours::{
    self as repo, AddDateOverrideResult, DateOverrideInput, IntervalInput, ReplaceMembersResult,
    WorkingHoursWithRelations,
};
use chatbotx_db::repo::{iso, partials::working_hours::WORKING_HOURS_MAX_DATE_OVERRIDES};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

const NOT_FOUND: &str = "Working hours not found";
const HOLIDAY_NOT_FOUND: &str = "Holiday not found";

/// `updateWorkingHoursRequest` (checked): `None` keeps the stored value.
#[derive(Debug, Clone, Default)]
pub struct Update {
    pub name: Option<String>,
    pub intervals: Option<Vec<IntervalInput>>,
    pub date_overrides: Option<Vec<DateOverrideInput>>,
}

fn date_override_json(row: &WorkingHoursDateOverride) -> Value {
    json!({ "date": row.date, "name": row.name, "ranges": row.ranges.0 })
}

/// `toHolidayResource`.
pub fn holiday_json(row: &WorkingHoursDateOverride) -> Value {
    let mut out = json!({ "id": row.id.to_string() });
    out.as_object_mut().expect("object").extend(
        date_override_json(row)
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );
    out["createdAt"] = Value::String(iso(row.created_at));
    out["updatedAt"] = Value::String(iso(row.updated_at));
    out
}

/// `toWorkingHoursResource` (`dateOverrides` only when given).
pub fn resource(
    row: &WorkingHoursWithRelations,
    overrides: Option<&[WorkingHoursDateOverride]>,
) -> Value {
    let mut intervals: Vec<(i64, i64, i64)> = row
        .intervals
        .iter()
        .map(|i| (i.weekday, i.start_minute, i.end_minute))
        .collect();
    intervals.sort_unstable();
    // `.sort()` on strings: lexicographic.
    let mut members: Vec<String> = row.members.iter().map(|m| m.user_id.to_string()).collect();
    members.sort();
    let mut out = json!({
        "id": row.working_hours.id.to_string(),
        "name": row.working_hours.name,
        "intervals": intervals
            .iter()
            .map(|(weekday, start, end)| json!({ "weekday": weekday, "startMinute": start, "endMinute": end }))
            .collect::<Vec<_>>(),
        "memberUserIds": members,
    });
    if let Some(overrides) = overrides {
        out["dateOverrides"] = overrides.iter().map(date_override_json).collect();
    }
    out["createdAt"] = Value::String(iso(row.working_hours.created_at));
    out["updatedAt"] = Value::String(iso(row.working_hours.updated_at));
    out
}

fn members_applied(result: ReplaceMembersResult) -> Result<()> {
    match result {
        ReplaceMembersResult::Ok => Ok(()),
        ReplaceMembersResult::NotFound => Err(BusinessError::not_found(NOT_FOUND)),
        ReplaceMembersResult::InvalidUsers(_) => Err(BusinessError::validation(
            "userIds",
            "One or more userIds are not members of this workspace",
        )),
    }
}

fn find_in(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    id: i64,
) -> Result<WorkingHoursWithRelations> {
    repo::find_by_id(conn, workspace_id, id)?.ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

/// `listByWorkspace`.
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<WorkingHoursWithRelations>> {
    tx::read(db, move |c| Ok(repo::list_by_workspace(c, workspace_id)?)).await
}

/// `findByIdOrFail` + `listDateOverrides` (the `get` / `update` output).
pub async fn get(db: &Db, workspace_id: i64, id: i64) -> Result<Value> {
    tx::read(db, move |c| {
        let row = find_in(c, workspace_id, id)?;
        let overrides = repo::list_date_overrides(c, workspace_id, Some(id))?;
        Ok(resource(&row, Some(&overrides)))
    })
    .await
}

/// `create`: the resource and the audit detail.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    intervals: Vec<IntervalInput>,
    user_ids: Vec<i64>,
) -> Result<(Value, String)> {
    let mut user_ids = user_ids;
    user_ids.sort_unstable();
    user_ids.dedup();
    tx::write(db, move |c| {
        let row = repo::create(c, workspace_id, &name)?;
        if !intervals.is_empty() {
            repo::replace_intervals(c, workspace_id, row.id, &intervals)?;
        }
        if !user_ids.is_empty() {
            members_applied(repo::replace_members(c, workspace_id, row.id, &user_ids)?)?;
        }
        let detail = format!("created working hours \"{}\" (#{})", row.name, row.id);
        Ok((resource(&find_in(c, workspace_id, row.id)?, None), detail))
    })
    .await
}

/// `update`: the resource with its date overrides and the audit detail.
pub async fn update(db: &Db, workspace_id: i64, id: i64, data: Update) -> Result<(Value, String)> {
    tx::write(db, move |c| {
        let current = find_in(c, workspace_id, id)?;
        if let Some(intervals) = &data.intervals
            && !repo::replace_intervals(c, workspace_id, id, intervals)?
        {
            return Err(BusinessError::not_found(NOT_FOUND));
        }
        if let Some(overrides) = &data.date_overrides
            && !repo::replace_date_overrides(c, workspace_id, Some(id), overrides)?
        {
            return Err(BusinessError::not_found(NOT_FOUND));
        }
        if let Some(name) = &data.name
            && *name != current.working_hours.name
        {
            repo::rename(c, workspace_id, id, name)?;
        }
        let detail = format!(
            "updated working hours \"{}\" (#{id})",
            current.working_hours.name
        );
        let row = find_in(c, workspace_id, id)?;
        let overrides = repo::list_date_overrides(c, workspace_id, Some(id))?;
        Ok((resource(&row, Some(&overrides)), detail))
    })
    .await
}

/// `delete`: the audit detail.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<String> {
    tx::write(db, move |c| {
        let current = find_in(c, workspace_id, id)?;
        if !repo::delete(c, workspace_id, id)? {
            return Err(BusinessError::not_found(NOT_FOUND));
        }
        Ok(format!(
            "deleted working hours \"{}\" (#{id})",
            current.working_hours.name
        ))
    })
    .await
}

/// `replaceMembers`: the resource and the audit detail.
pub async fn replace_members(
    db: &Db,
    workspace_id: i64,
    id: i64,
    user_ids: Vec<i64>,
) -> Result<(Value, String)> {
    let mut user_ids = user_ids;
    user_ids.sort_unstable();
    user_ids.dedup();
    tx::write(db, move |c| {
        members_applied(repo::replace_members(c, workspace_id, id, &user_ids)?)?;
        let detail = format!("set {} member(s) on working hours (#{id})", user_ids.len());
        Ok((resource(&find_in(c, workspace_id, id)?, None), detail))
    })
    .await
}

/// `listDateOverrides({ workingHoursId: null })`: the workspace holidays.
pub async fn list_holidays(db: &Db, workspace_id: i64) -> Result<Vec<WorkingHoursDateOverride>> {
    tx::read(db, move |c| {
        Ok(repo::list_date_overrides(c, workspace_id, None)?)
    })
    .await
}

/// `addHoliday`: the holiday and the audit detail.
pub async fn add_holiday(
    db: &Db,
    workspace_id: i64,
    data: DateOverrideInput,
) -> Result<(WorkingHoursDateOverride, String)> {
    tx::write(db, move |c| {
        match repo::add_date_override(c, workspace_id, None, &data)? {
            AddDateOverrideResult::Ok(row) => {
                let detail = format!("added holiday {}", data.date);
                Ok((row, detail))
            }
            AddDateOverrideResult::LimitReached => Err(BusinessError::validation(
                "date",
                format!("At most {WORKING_HOURS_MAX_DATE_OVERRIDES} holidays per workspace"),
            )),
            AddDateOverrideResult::Duplicate => Err(BusinessError::validation(
                "date",
                format!("A holiday already exists on {}", data.date),
            )),
            // A workspace-wide holiday has no parent schedule.
            AddDateOverrideResult::NotFound => Err(BusinessError::Repo(
                chatbotx_db::repo::Error::msg("Unexpected add holiday result: notFound"),
            )),
        }
    })
    .await
}

/// `updateHoliday`: `name` `Some(None)` clears it.
pub async fn update_holiday(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<Option<String>>,
    ranges: Option<Value>,
) -> Result<(WorkingHoursDateOverride, String)> {
    tx::write(db, move |c| {
        let row = repo::update_date_override(
            c,
            workspace_id,
            None,
            id,
            name.as_ref().map(|n| n.as_deref()),
            ranges.as_ref(),
        )?
        .ok_or_else(|| BusinessError::not_found(HOLIDAY_NOT_FOUND))?;
        let detail = format!("updated holiday {}", row.date);
        Ok((row, detail))
    })
    .await
}

/// `deleteHoliday`: the audit detail.
pub async fn delete_holiday(db: &Db, workspace_id: i64, id: i64) -> Result<String> {
    tx::write(db, move |c| {
        if !repo::delete_date_override(c, workspace_id, None, id)? {
            return Err(BusinessError::not_found(HOLIDAY_NOT_FOUND));
        }
        Ok(format!("deleted holiday (#{id})"))
    })
    .await
}
