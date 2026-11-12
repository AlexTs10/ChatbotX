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
    let rows = super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" ASC, \"id\" ASC",
        &[&workspace_id],
    )?;
    with_relations(conn, rows)
}

/// `findById`.
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<WorkingHoursWithRelations>> {
    let row: Option<WorkingHours> = super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?;
    Ok(with_relations(conn, row.into_iter().collect())?
        .into_iter()
        .next())
}

/// `findForUser`: the schedule of a member.
pub fn find_for_user(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Option<WorkingHoursForAgent>> {
    let member: Option<WorkingHoursMember> = super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"userId\" = ?2",
        &[&workspace_id, &user_id],
    )?;
    let Some(member) = member else {
        return Ok(None);
    };
    let wh: Option<WorkingHours> = super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&member.working_hours_id, &workspace_id],
    )?;
    let Some(wh) = wh else {
        return Ok(None);
    };
    let intervals: Vec<WorkingHoursInterval> =
        super::find_many(conn, "WHERE \"workingHoursId\" = ?1", &[&wh.id])?;
    Ok(Some(WorkingHoursForAgent {
        id: wh.id,
        intervals: intervals
            .into_iter()
            .map(|i| IntervalInput {
                weekday: i.weekday,
                start_minute: i.start_minute,
                end_minute: i.end_minute,
            })
            .collect(),
    }))
}

/// `create`.
pub fn create(conn: &Connection, workspace_id: i64, name: &str) -> Result<WorkingHours> {
    let now = now_ms();
    super::returning::<WorkingHours>(
        conn,
        "INSERT INTO \"WorkingHours\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\") VALUES (?1, ?2, ?2, ?3, ?4)",
        &[&new_id(), &now, &workspace_id, &name],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to create working hours"))
}

/// `rename`.
pub fn rename(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    name: &str,
) -> Result<Option<WorkingHours>> {
    Ok(super::returning::<WorkingHours>(
        conn,
        "UPDATE \"WorkingHours\" SET \"name\" = ?3, \"updatedAt\" = ?4 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id, &name, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM \"WorkingHours\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id],
    )?;
    Ok(n > 0)
}

/// `replaceIntervals`: `false` when the schedule is not in the workspace.
pub fn replace_intervals(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: i64,
    intervals: &[IntervalInput],
) -> Result<bool> {
    if !lock_parent(conn, workspace_id, working_hours_id)? {
        return Ok(false);
    }
    conn.execute(
        "DELETE FROM \"WorkingHoursInterval\" WHERE \"workingHoursId\" = ?1",
        [working_hours_id],
    )?;
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"WorkingHoursInterval\" (\"id\", \"createdAt\", \"updatedAt\", \"workingHoursId\", \"weekday\", \
         \"startMinute\", \"endMinute\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for i in intervals {
        stmt.execute(rusqlite::params![
            new_id(),
            now,
            working_hours_id,
            i.weekday,
            i.start_minute,
            i.end_minute
        ])?;
    }
    Ok(true)
}

/// `replaceMembers`: a user moves from another schedule (that schedule is bumped).
pub fn replace_members(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: i64,
    user_ids: &[i64],
) -> Result<ReplaceMembersResult> {
    if !lock_parent(conn, workspace_id, working_hours_id)? {
        return Ok(ReplaceMembersResult::NotFound);
    }
    let mut users = user_ids.to_vec();
    users.sort_unstable();
    users.dedup();
    if !users.is_empty() {
        let mut stmt = conn.prepare_cached(
            "SELECT \"userId\" FROM \"WorkspaceMember\" WHERE \"workspaceId\" = ?1 AND \"userId\" IN (SELECT value FROM json_each(?2))",
        )?;
        let found = stmt
            .query_map(rusqlite::params![workspace_id, id_list(&users)], |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let missing: Vec<i64> = users
            .iter()
            .copied()
            .filter(|u| !found.contains(u))
            .collect();
        if !missing.is_empty() {
            return Ok(ReplaceMembersResult::InvalidUsers(missing));
        }
        let mut stmt = conn.prepare_cached(
            "SELECT \"workingHoursId\" FROM \"WorkingHoursMember\" WHERE \"workspaceId\" = ?1 \
             AND \"userId\" IN (SELECT value FROM json_each(?2)) AND \"workingHoursId\" <> ?3",
        )?;
        let moved_from = stmt
            .query_map(
                rusqlite::params![workspace_id, id_list(&users), working_hours_id],
                |r| r.get::<_, i64>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        touch_schedules(conn, workspace_id, &moved_from)?;
    }
    if users.is_empty() {
        conn.execute(
            "DELETE FROM \"WorkingHoursMember\" WHERE \"workingHoursId\" = ?1 AND \"workspaceId\" = ?2",
            rusqlite::params![working_hours_id, workspace_id],
        )?;
        return Ok(ReplaceMembersResult::Ok);
    }
    conn.execute(
        "DELETE FROM \"WorkingHoursMember\" WHERE \"workingHoursId\" = ?1 AND \"workspaceId\" = ?2 \
         AND \"userId\" NOT IN (SELECT value FROM json_each(?3))",
        rusqlite::params![working_hours_id, workspace_id, id_list(&users)],
    )?;
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"WorkingHoursMember\" (\"id\", \"createdAt\", \"updatedAt\", \"workingHoursId\", \"workspaceId\", \"userId\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5) ON CONFLICT (\"workspaceId\", \"userId\") DO UPDATE SET \
         \"workingHoursId\" = excluded.\"workingHoursId\"",
    )?;
    for u in users {
        stmt.execute(rusqlite::params![
            new_id(),
            now,
            working_hours_id,
            workspace_id,
            u
        ])?;
    }
    Ok(ReplaceMembersResult::Ok)
}

/// `listDateOverrides`: one scope (`None` = the workspace holidays), by date.
pub fn list_date_overrides(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
) -> Result<Vec<WorkingHoursDateOverride>> {
    let mut q = Sql::raw(format!(
        "{} WHERE ",
        <WorkingHoursDateOverride as super::Model>::select_sql()
    ));
    q.append(scope_where(workspace_id, working_hours_id));
    q.push(" ORDER BY \"date\"");
    q.models(conn)
}

/// `listDateOverridesInRange`: every scope, `from <= date <= to`.
pub fn list_date_overrides_in_range(
    conn: &Connection,
    workspace_id: i64,
    from: &str,
    to: &str,
) -> Result<Vec<WorkingHoursDateOverride>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"date\" >= ?2 AND \"date\" <= ?3 ORDER BY \"date\"",
        &[&workspace_id, &from, &to],
    )
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
    if !lock_date_override_scope(conn, workspace_id, working_hours_id)? {
        return Ok(false);
    }
    let mut d = Sql::raw("DELETE FROM \"WorkingHoursDateOverride\" WHERE ");
    d.append(scope_where(workspace_id, working_hours_id));
    d.execute(conn)?;
    for o in overrides {
        insert_override(conn, workspace_id, working_hours_id, o, false)?;
    }
    Ok(true)
}

/// `addDateOverride`: at most [`MAX_DATE_OVERRIDES`] rows per scope; one row per date.
pub fn add_date_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    o: &DateOverrideInput,
) -> Result<AddDateOverrideResult> {
    if !lock_date_override_scope(conn, workspace_id, working_hours_id)? {
        return Ok(AddDateOverrideResult::NotFound);
    }
    let mut c = Sql::raw("SELECT count(*) FROM \"WorkingHoursDateOverride\" WHERE ");
    c.append(scope_where(workspace_id, working_hours_id));
    let count: i64 = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    if count >= MAX_DATE_OVERRIDES {
        return Ok(AddDateOverrideResult::LimitReached);
    }
    Ok(
        match insert_override(conn, workspace_id, working_hours_id, o, true)? {
            Some(row) => AddDateOverrideResult::Ok(row),
            None => AddDateOverrideResult::Duplicate,
        },
    )
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
    if !lock_date_override_scope(conn, workspace_id, working_hours_id)? {
        return Ok(None);
    }
    let mut q = Sql::raw("UPDATE \"WorkingHoursDateOverride\" SET ");
    if let Some(name) = name {
        q.push("\"name\" = ");
        q.push_bind(name.map_or(Value::Null, |n| Value::Text(n.to_owned())));
        q.push(", ");
    }
    if let Some(r) = ranges {
        q.push("\"ranges\" = ");
        q.push_bind(Value::Text(r.to_string()));
        q.push(", ");
    }
    q.push("\"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE ");
    q.append(scope_where(workspace_id, working_hours_id));
    q.push(" AND \"id\" = ");
    q.push_bind(id);
    q.push(&format!(
        " RETURNING {}",
        <WorkingHoursDateOverride as super::Model>::columns_sql(None)
    ));
    q.model(conn)
}

/// `deleteDateOverride`.
pub fn delete_date_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    id: i64,
) -> Result<bool> {
    if !lock_date_override_scope(conn, workspace_id, working_hours_id)? {
        return Ok(false);
    }
    let mut q = Sql::raw("DELETE FROM \"WorkingHoursDateOverride\" WHERE ");
    q.append(scope_where(workspace_id, working_hours_id));
    q.push(" AND \"id\" = ");
    q.push_bind(id);
    Ok(q.execute(conn)? > 0)
}

/// `deleteMembersForUsers`: removes the users from every schedule and bumps those schedules.
pub fn delete_members_for_users(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
) -> Result<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"WorkingHoursMember\" WHERE \"workspaceId\" = ?1 AND \"userId\" IN (SELECT value FROM json_each(?2)) \
         RETURNING \"workingHoursId\"",
    )?;
    let removed = stmt
        .query_map(rusqlite::params![workspace_id, id_list(user_ids)], |r| {
            r.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    touch_schedules(conn, workspace_id, &removed)
}
