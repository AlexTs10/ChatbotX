//! Port of `repositories/working-hours/repository.ts` (P3.3). Every `client.transaction` is one
//! call inside the caller's write job; row locks (`FOR NO KEY UPDATE`, `FOR KEY SHARE`,
//! `SKIP LOCKED`) and `pg_advisory_xact_lock` are no-ops under the one SQLite writer.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{
    WorkingHours, WorkingHoursDateOverride, WorkingHoursInterval, WorkingHoursMember,
};
use super::sql::Sql;
use super::{Json, Result, id_list, new_id, now_ms};

/// `WORKING_HOURS_MAX_DATE_OVERRIDES`.
pub const MAX_DATE_OVERRIDES: i64 = 400;

/// `WorkingHoursIntervalInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalInput {
    pub weekday: i64,
    pub start_minute: i64,
    pub end_minute: i64,
}

/// `WorkingHoursDateOverrideInput` (`ranges` = the JSON array of `{ startMinute, endMinute }`).
#[derive(Debug, Clone, PartialEq)]
pub struct DateOverrideInput {
    pub date: String,
    pub name: Option<String>,
    pub ranges: serde_json::Value,
}

/// `WorkingHoursWithRelations`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingHoursWithRelations {
    pub working_hours: WorkingHours,
    pub intervals: Vec<WorkingHoursInterval>,
    pub members: Vec<WorkingHoursMember>,
}

/// `WorkingHoursForAgent`: id + `(weekday, startMinute, endMinute)` intervals.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingHoursForAgent {
    pub id: i64,
    pub intervals: Vec<IntervalInput>,
}

/// `ReplaceMembersResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceMembersResult {
    Ok,
    NotFound,
    InvalidUsers(Vec<i64>),
}

/// `AddDateOverrideResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum AddDateOverrideResult {
    Ok(WorkingHoursDateOverride),
    NotFound,
    Duplicate,
    LimitReached,
}

/// `UpdateDateOverrideResult` (`None` = `notFound`).
pub type UpdateDateOverrideResult = Option<WorkingHoursDateOverride>;

fn lock_parent(conn: &Connection, workspace_id: i64, working_hours_id: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE \"WorkingHours\" SET \"updatedAt\" = ?3 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![working_hours_id, workspace_id, now_ms()],
    )?;
    Ok(n > 0)
}

fn touch_schedules(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE \"WorkingHours\" SET \"updatedAt\" = ?3 WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, id_list(&ids), now_ms()],
    )?;
    Ok(())
}

fn lock_date_override_scope(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
) -> Result<bool> {
    match working_hours_id {
        Some(id) => lock_parent(conn, workspace_id, id),
        None => Ok(true),
    }
}

fn scope_where(workspace_id: i64, working_hours_id: Option<i64>) -> Sql {
    let mut s = Sql::bind("\"workspaceId\" = ", workspace_id);
    match working_hours_id {
        Some(id) => {
            s.push(" AND \"workingHoursId\" = ");
            s.push_bind(id);
        }
        None => {
            s.push(" AND \"workingHoursId\" IS NULL");
        }
    }
    s
}

fn with_relations(
    conn: &Connection,
    rows: Vec<WorkingHours>,
) -> Result<Vec<WorkingHoursWithRelations>> {
    rows.into_iter()
        .map(|wh| {
            let intervals = super::find_many(conn, "WHERE \"workingHoursId\" = ?1", &[&wh.id])?;
            let members = super::find_many(conn, "WHERE \"workingHoursId\" = ?1", &[&wh.id])?;
            Ok(WorkingHoursWithRelations {
                working_hours: wh,
                intervals,
                members,
            })
        })
        .collect()
}

/// `listByWorkspace`: oldest first, with intervals and members.
pub fn list_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<WorkingHoursWithRelations>> {
    todo!("P3.3")
}

/// `findById`.
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<WorkingHoursWithRelations>> {
    todo!("P3.3")
}

/// `findForUser`: the schedule of a member.
pub fn find_for_user(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Option<WorkingHoursForAgent>> {
    todo!("P3.3")
}

/// `create`.
pub fn create(conn: &Connection, workspace_id: i64, name: &str) -> Result<WorkingHours> {
    todo!("P3.3")
}

/// `rename`.
pub fn rename(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    name: &str,
) -> Result<Option<WorkingHours>> {
    todo!("P3.3")
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `replaceIntervals`: `false` when the schedule is not in the workspace.
pub fn replace_intervals(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: i64,
    intervals: &[IntervalInput],
) -> Result<bool> {
    todo!("P3.3")
}

/// `replaceMembers`: a user moves from another schedule (that schedule is bumped).
pub fn replace_members(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: i64,
    user_ids: &[i64],
) -> Result<ReplaceMembersResult> {
    todo!("P3.3")
}

/// `listDateOverrides`: one scope (`None` = the workspace holidays), by date.
pub fn list_date_overrides(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
) -> Result<Vec<WorkingHoursDateOverride>> {
    todo!("P3.3")
}

/// `listDateOverridesInRange`: every scope, `from <= date <= to`.
pub fn list_date_overrides_in_range(
    conn: &Connection,
    workspace_id: i64,
    from: &str,
    to: &str,
) -> Result<Vec<WorkingHoursDateOverride>> {
    todo!("P3.3")
}

fn insert_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    o: &DateOverrideInput,
    ignore_conflict: bool,
) -> Result<Option<WorkingHoursDateOverride>> {
    let now = now_ms();
    let sql = format!(
        "INSERT INTO \"WorkingHoursDateOverride\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"workingHoursId\", \
         \"date\", \"name\", \"ranges\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7){}",
        if ignore_conflict {
            " ON CONFLICT DO NOTHING"
        } else {
            ""
        }
    );
    Ok(super::returning::<WorkingHoursDateOverride>(
        conn,
        &sql,
        &[
            &new_id(),
            &now,
            &workspace_id,
            &working_hours_id,
            &o.date,
            &o.name,
            &Json(o.ranges.clone()),
        ],
    )?
    .into_iter()
    .next())
}

/// `replaceDateOverrides`: `false` (= `notFound`) for an unknown schedule.
pub fn replace_date_overrides(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    overrides: &[DateOverrideInput],
) -> Result<bool> {
    todo!("P3.3")
}

/// `addDateOverride`: at most [`MAX_DATE_OVERRIDES`] rows per scope; one row per date.
pub fn add_date_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    o: &DateOverrideInput,
) -> Result<AddDateOverrideResult> {
    todo!("P3.3")
}

/// `updateDateOverride` (`name`: `None` = unchanged, `Some(None)` = clear).
pub fn update_date_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    id: i64,
    name: Option<Option<&str>>,
    ranges: Option<&serde_json::Value>,
) -> Result<UpdateDateOverrideResult> {
    todo!("P3.3")
}

/// `deleteDateOverride`.
pub fn delete_date_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    id: i64,
) -> Result<bool> {
    todo!("P3.3")
}

/// `deleteMembersForUsers`: removes the users from every schedule and bumps those schedules.
pub fn delete_members_for_users(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
) -> Result<()> {
    todo!("P3.3")
}
