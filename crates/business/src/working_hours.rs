//! `working-hours/service.ts` (+ `workingHoursRepository`, the partial
//! schemas of `database/partials/working-hours.ts`): schedules, their weekly
//! intervals, members and date overrides, and the workspace holidays
//! (date overrides without a schedule).
//!
//! ponytail: no cache (`withCache` / tags) and no advisory locks; every
//! write is one SQLite transaction. The read side for the assignment steps
//! is `crates/worker/src/steps/working_hours.rs`.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{WorkingHours, WorkingHoursDateOverride};
use chatbotx_db::repo::{find_many, find_one, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::errors::BusinessError;
use crate::{Result, tx};

pub const MINUTES_PER_DAY: i64 = 1440;
pub const MAX_INTERVALS_PER_WEEKDAY: usize = 10;
pub const MAX_INTERVALS_TOTAL: usize = 70;
pub const MAX_DATE_OVERRIDES: usize = 400;
pub const MAX_NAME_LENGTH: usize = 100;

/// A weekly interval (`weekday` 0 = Sunday).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub weekday: i64,
    pub start_minute: i64,
    pub end_minute: i64,
}

/// An open range of a date (`startMinute` .. `endMinute`, end-exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start_minute: i64,
    pub end_minute: i64,
}

/// `WorkingHoursDateOverrideInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateOverride {
    pub date: String,
    pub name: Option<String>,
    pub ranges: Vec<Range>,
}

/// A schedule with its intervals (by weekday, start) and member user ids.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub row: WorkingHours,
    pub intervals: Vec<Interval>,
    pub member_user_ids: Vec<i64>,
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Working hours not found")
}

fn holiday_not_found() -> BusinessError {
    BusinessError::not_found("Holiday not found")
}

// ---- validation (the partial schemas' refinements) -------------------------

fn range_issue(start: i64, end: i64) -> Option<&'static str> {
    if !(0..MINUTES_PER_DAY).contains(&start) || !(1..=MINUTES_PER_DAY).contains(&end) {
        return Some("Minutes out of bounds");
    }
    (start >= end).then_some("endMinute must be greater than startMinute")
}

fn overlap(ranges: &mut [(i64, i64)]) -> bool {
    ranges.sort_unstable();
    ranges.windows(2).any(|w| w[1].0 < w[0].1)
}

/// `workingHoursIntervalsSchema`: the first issue message.
pub fn validate_intervals(intervals: &[Interval]) -> std::result::Result<(), String> {
    if intervals.len() > MAX_INTERVALS_TOTAL {
        return Err(format!(
            "Too big: expected array to have <={MAX_INTERVALS_TOTAL} items"
        ));
    }
    let mut by_day: std::collections::BTreeMap<i64, Vec<(i64, i64)>> = Default::default();
    for i in intervals {
        if !(0..=6).contains(&i.weekday) {
            return Err("weekday must be between 0 and 6".into());
        }
        if let Some(issue) = range_issue(i.start_minute, i.end_minute) {
            return Err(issue.into());
        }
        by_day
            .entry(i.weekday)
            .or_default()
            .push((i.start_minute, i.end_minute));
    }
    for (weekday, day) in &mut by_day {
        if day.len() > MAX_INTERVALS_PER_WEEKDAY {
            return Err(format!(
                "At most {MAX_INTERVALS_PER_WEEKDAY} intervals per weekday (weekday {weekday})"
            ));
        }
        if overlap(day) {
            return Err(format!("Intervals overlap on weekday {weekday}"));
        }
    }
    Ok(())
}

/// `workingHoursDateRangesSchema`.
pub fn validate_ranges(ranges: &[Range]) -> std::result::Result<(), String> {
    if ranges.len() > MAX_INTERVALS_PER_WEEKDAY {
        return Err(format!(
            "Too big: expected array to have <={MAX_INTERVALS_PER_WEEKDAY} items"
        ));
    }
    if let Some(issue) = ranges
        .iter()
        .find_map(|r| range_issue(r.start_minute, r.end_minute))
    {
        return Err(issue.into());
    }
    let mut pairs: Vec<(i64, i64)> = ranges
        .iter()
        .map(|r| (r.start_minute, r.end_minute))
        .collect();
    if overlap(&mut pairs) {
        return Err("Intervals overlap on this date".into());
    }
    Ok(())
}

/// `workingHoursLocalDateSchema`.
pub fn validate_local_date(date: &str) -> std::result::Result<(), String> {
    let shaped = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !shaped {
        return Err("Expected YYYY-MM-DD".into());
    }
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| "Not a real calendar date".into())
}

/// `workingHoursDateOverrideInputSchema`.
pub fn validate_override(o: &DateOverride) -> std::result::Result<(), String> {
    validate_local_date(&o.date)?;
    if o.name
        .as_deref()
        .is_some_and(|n| n.chars().count() > MAX_NAME_LENGTH)
    {
        return Err(format!(
            "Too big: expected string to have <={MAX_NAME_LENGTH} characters"
        ));
    }
    validate_ranges(&o.ranges)
}

/// `workingHoursDateOverridesSchema`.
pub fn validate_overrides(overrides: &[DateOverride]) -> std::result::Result<(), String> {
    if overrides.len() > MAX_DATE_OVERRIDES {
        return Err(format!(
            "Too big: expected array to have <={MAX_DATE_OVERRIDES} items"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for o in overrides {
        validate_override(o)?;
        if !seen.insert(o.date.as_str()) {
            return Err(format!("Duplicate date {}", o.date));
        }
    }
    Ok(())
}

fn checked<T>(field: &str, result: std::result::Result<T, String>) -> Result<T> {
    result.map_err(|message| BusinessError::validation(field, message))
}

pub fn ranges_json(ranges: &[Range]) -> Value {
    Value::Array(
        ranges
            .iter()
            .map(|r| json!({ "startMinute": r.start_minute, "endMinute": r.end_minute }))
            .collect(),
    )
}

// ---- reads -----------------------------------------------------------------

fn load(conn: &Connection, rows: Vec<WorkingHours>) -> Result<Vec<Schedule>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = id_list(&rows.iter().map(|r| r.id).collect::<Vec<_>>());
    let mut intervals: std::collections::HashMap<i64, Vec<Interval>> = Default::default();
    let mut stmt = conn.prepare(&format!(
        r#"SELECT "workingHoursId", "weekday", "startMinute", "endMinute" FROM "WorkingHoursInterval"
           WHERE "workingHoursId" IN (SELECT value FROM json_each('{ids}')) ORDER BY "weekday", "startMinute", "id""#
    ))?;
    let mut cursor = stmt.query([])?;
    while let Some(r) = cursor.next()? {
        intervals.entry(r.get(0)?).or_default().push(Interval {
            weekday: r.get(1)?,
            start_minute: r.get(2)?,
            end_minute: r.get(3)?,
        });
    }
    let mut members: std::collections::HashMap<i64, Vec<i64>> = Default::default();
    let mut stmt = conn.prepare(&format!(
        r#"SELECT "workingHoursId", "userId" FROM "WorkingHoursMember" WHERE "workingHoursId" IN (SELECT value FROM json_each('{ids}'))"#
    ))?;
    let mut cursor = stmt.query([])?;
    while let Some(r) = cursor.next()? {
        members.entry(r.get(0)?).or_default().push(r.get(1)?);
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut member_user_ids = members.remove(&row.id).unwrap_or_default();
            // `.sort()` on the id strings.
            member_user_ids.sort_by_key(|id| id.to_string());
            Schedule {
                intervals: intervals.remove(&row.id).unwrap_or_default(),
                member_user_ids,
                row,
            }
        })
        .collect())
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Schedule> {
    let row: Option<WorkingHours> = find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?;
    load(conn, row.into_iter().collect())?
        .pop()
        .ok_or_else(not_found)
}

/// `listByWorkspace` (by `createdAt`, `id`).
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<Schedule>> {
    tx::read(db, move |conn| {
        let rows = find_many(
            conn,
            "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\", \"id\"",
            &[&workspace_id],
        )?;
        load(conn, rows)
    })
    .await
}

/// `findByIdOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Schedule> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id)).await
}

fn scope_sql(working_hours_id: Option<i64>) -> &'static str {
    match working_hours_id {
        Some(_) => "\"workspaceId\" = ?1 AND \"workingHoursId\" = ?2",
        None => "\"workspaceId\" = ?1 AND \"workingHoursId\" IS NULL AND ?2 IS NULL",
    }
}

fn overrides_in(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
) -> Result<Vec<WorkingHoursDateOverride>> {
    Ok(find_many(
        conn,
        &format!("WHERE {} ORDER BY \"date\"", scope_sql(working_hours_id)),
        &[&workspace_id, &working_hours_id],
    )?)
}

/// `listDateOverrides` (a schedule's, or the holidays with `None`), by date.
pub async fn list_date_overrides(
    db: &Db,
    workspace_id: i64,
    working_hours_id: Option<i64>,
) -> Result<Vec<WorkingHoursDateOverride>> {
    tx::read(db, move |conn| {
        overrides_in(conn, workspace_id, working_hours_id)
    })
    .await
}

// ---- writes ----------------------------------------------------------------

/// `lockParent`: bumps `updatedAt`; `false` = not in the workspace.
fn touch(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        r#"UPDATE "WorkingHours" SET "updatedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![id, workspace_id, now_ms()],
    )? > 0)
}

fn replace_intervals(conn: &Connection, id: i64, intervals: &[Interval]) -> Result<()> {
    conn.execute(
        r#"DELETE FROM "WorkingHoursInterval" WHERE "workingHoursId" = ?1"#,
        [id],
    )?;
    let now = now_ms();
    for i in intervals {
        conn.execute(
            r#"INSERT INTO "WorkingHoursInterval" ("id", "createdAt", "updatedAt", "workingHoursId", "weekday", "startMinute", "endMinute")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)"#,
            params![new_id(), now, id, i.weekday, i.start_minute, i.end_minute],
        )?;
    }
    Ok(())
}

/// `replaceMembers`: a user on another schedule is moved here (that
/// schedule's `updatedAt` is bumped); a non-member user id fails the write.
fn replace_members(conn: &Connection, workspace_id: i64, id: i64, user_ids: &[i64]) -> Result<()> {
    if !touch(conn, workspace_id, id)? {
        return Err(not_found());
    }
    let mut user_ids = user_ids.to_vec();
    user_ids.sort_unstable();
    user_ids.dedup();
    let now = now_ms();
    if !user_ids.is_empty() {
        let list = id_list(&user_ids);
        let members: i64 = conn.query_row(
            &format!(
                r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" IN (SELECT value FROM json_each('{list}'))"#
            ),
            [workspace_id],
            |r| r.get(0),
        )?;
        if members != user_ids.len() as i64 {
            return Err(BusinessError::validation(
                "userIds",
                "One or more userIds are not members of this workspace",
            ));
        }
        conn.execute(
            &format!(
                r#"UPDATE "WorkingHours" SET "updatedAt" = ?3 WHERE "workspaceId" = ?1 AND "id" <> ?2 AND "id" IN
                   (SELECT "workingHoursId" FROM "WorkingHoursMember" WHERE "workspaceId" = ?1 AND "userId" IN (SELECT value FROM json_each('{list}')))"#
            ),
            params![workspace_id, id, now],
        )?;
        conn.execute(
            &format!(
                r#"DELETE FROM "WorkingHoursMember" WHERE "workingHoursId" = ?1 AND "workspaceId" = ?2 AND "userId" NOT IN (SELECT value FROM json_each('{list}'))"#
            ),
            params![id, workspace_id],
        )?;
    } else {
        conn.execute(
            r#"DELETE FROM "WorkingHoursMember" WHERE "workingHoursId" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )?;
    }
    for user_id in &user_ids {
        conn.execute(
            r#"INSERT INTO "WorkingHoursMember" ("id", "createdAt", "updatedAt", "workingHoursId", "workspaceId", "userId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)
               ON CONFLICT ("workspaceId", "userId") DO UPDATE SET "workingHoursId" = excluded."workingHoursId""#,
            params![new_id(), now, id, workspace_id, user_id],
        )?;
    }
    Ok(())
}

fn insert_override(
    conn: &Connection,
    workspace_id: i64,
    working_hours_id: Option<i64>,
    o: &DateOverride,
) -> Result<Option<WorkingHoursDateOverride>> {
    let id = new_id();
    let now = now_ms();
    let inserted = conn.execute(
        r#"INSERT INTO "WorkingHoursDateOverride" ("id", "createdAt", "updatedAt", "workspaceId", "workingHoursId", "date", "name", "ranges")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT DO NOTHING"#,
        params![id, now, workspace_id, working_hours_id, o.date, o.name, ranges_json(&o.ranges).to_string()],
    )?;
    if inserted == 0 {
        return Ok(None);
    }
    Ok(find_one(conn, "\"id\" = ?1", &[&id])?)
}

/// `create`.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    intervals: Vec<Interval>,
    user_ids: Vec<i64>,
) -> Result<Schedule> {
    checked("intervals", validate_intervals(&intervals))?;
    tx::write(db, move |conn| {
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "WorkingHours" ("id", "createdAt", "updatedAt", "workspaceId", "name") VALUES (?1, ?2, ?2, ?3, ?4)"#,
            params![id, now, workspace_id, name],
        )?;
        replace_intervals(conn, id, &intervals)?;
        if !user_ids.is_empty() {
            replace_members(conn, workspace_id, id, &user_ids)?;
        }
        find_in(conn, workspace_id, id)
    })
    .await
}

/// `update`: `None` fields stay unchanged.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    intervals: Option<Vec<Interval>>,
    date_overrides: Option<Vec<DateOverride>>,
) -> Result<Schedule> {
    if let Some(intervals) = &intervals {
        checked("intervals", validate_intervals(intervals))?;
    }
    if let Some(overrides) = &date_overrides {
        checked("dateOverrides", validate_overrides(overrides))?;
    }
    tx::write(db, move |conn| {
        let current = find_in(conn, workspace_id, id)?;
        if let Some(intervals) = &intervals {
            touch(conn, workspace_id, id)?;
            replace_intervals(conn, id, intervals)?;
        }
        if let Some(overrides) = &date_overrides {
            touch(conn, workspace_id, id)?;
            conn.execute(
                r#"DELETE FROM "WorkingHoursDateOverride" WHERE "workspaceId" = ?1 AND "workingHoursId" = ?2"#,
                params![workspace_id, id],
            )?;
            for o in overrides {
                insert_override(conn, workspace_id, Some(id), o)?;
            }
        }
        if let Some(name) = name.filter(|n| *n != current.row.name) {
            conn.execute(
                r#"UPDATE "WorkingHours" SET "name" = ?3, "updatedAt" = ?4 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id, name, now_ms()],
            )?;
        }
        find_in(conn, workspace_id, id)
    })
    .await
}

/// `delete`.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        let deleted = conn.execute(
            r#"DELETE FROM "WorkingHours" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )?;
        if deleted == 0 {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}

/// `replaceMembers` (duplicates dropped).
pub async fn set_members(
    db: &Db,
    workspace_id: i64,
    id: i64,
    user_ids: Vec<i64>,
) -> Result<Schedule> {
    tx::write(db, move |conn| {
        replace_members(conn, workspace_id, id, &user_ids)?;
        find_in(conn, workspace_id, id)
    })
    .await
}

/// `addHoliday`.
pub async fn add_holiday(
    db: &Db,
    workspace_id: i64,
    o: DateOverride,
) -> Result<WorkingHoursDateOverride> {
    checked("date", validate_override(&o))?;
    tx::write(db, move |conn| {
        let count: i64 = conn.query_row(
            r#"SELECT COUNT(*) FROM "WorkingHoursDateOverride" WHERE "workspaceId" = ?1 AND "workingHoursId" IS NULL"#,
            [workspace_id],
            |r| r.get(0),
        )?;
        if count >= MAX_DATE_OVERRIDES as i64 {
            return Err(BusinessError::validation(
                "date",
                format!("At most {MAX_DATE_OVERRIDES} holidays per workspace"),
            ));
        }
        insert_override(conn, workspace_id, None, &o)?.ok_or_else(|| {
            BusinessError::validation("date", format!("A holiday already exists on {}", o.date))
        })
    })
    .await
}

/// `updateHoliday`: `name` `Some(None)` clears it; `None` = unchanged.
pub async fn update_holiday(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<Option<String>>,
    ranges: Option<Vec<Range>>,
) -> Result<WorkingHoursDateOverride> {
    if let Some(ranges) = &ranges {
        checked("ranges", validate_ranges(ranges))?;
    }
    tx::write(db, move |conn| {
        let updated = conn.execute(
            r#"UPDATE "WorkingHoursDateOverride" SET
                 "name" = CASE WHEN ?3 THEN ?4 ELSE "name" END,
                 "ranges" = COALESCE(?5, "ranges"),
                 "updatedAt" = ?6
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "workingHoursId" IS NULL"#,
            params![
                id,
                workspace_id,
                name.is_some(),
                name.clone().flatten(),
                ranges.as_deref().map(|r| ranges_json(r).to_string()),
                now_ms()
            ],
        )?;
        if updated == 0 {
            return Err(holiday_not_found());
        }
        find_one(conn, "\"id\" = ?1", &[&id])?.ok_or_else(holiday_not_found)
    })
    .await
}

/// `deleteHoliday`.
pub async fn delete_holiday(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        let deleted = conn.execute(
            r#"DELETE FROM "WorkingHoursDateOverride" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "workingHoursId" IS NULL"#,
            params![id, workspace_id],
        )?;
        if deleted == 0 {
            return Err(holiday_not_found());
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(weekday: i64, start: i64, end: i64) -> Interval {
        Interval {
            weekday,
            start_minute: start,
            end_minute: end,
        }
    }

    #[test]
    fn intervals_reject_overlap_bounds_and_the_per_day_cap() {
        assert!(validate_intervals(&[iv(1, 540, 720), iv(1, 780, 1020)]).is_ok());
        assert_eq!(
            validate_intervals(&[iv(1, 540, 720), iv(1, 700, 800)]),
            Err("Intervals overlap on weekday 1".into())
        );
        assert!(validate_intervals(&[iv(1, 600, 600)]).is_err());
        assert!(validate_intervals(&[iv(7, 0, 60)]).is_err());
        assert!(validate_intervals(&[iv(0, 0, 1440)]).is_ok());
        let eleven: Vec<_> = (0..11).map(|n| iv(2, n * 60, n * 60 + 30)).collect();
        assert!(
            validate_intervals(&eleven)
                .unwrap_err()
                .contains("At most 10")
        );
    }

    #[test]
    fn local_dates_must_be_real() {
        assert!(validate_local_date("2026-02-28").is_ok());
        assert_eq!(
            validate_local_date("2026-02-30"),
            Err("Not a real calendar date".into())
        );
        assert_eq!(
            validate_local_date("2026-2-3"),
            Err("Expected YYYY-MM-DD".into())
        );
    }

    #[test]
    fn overrides_reject_a_duplicate_date() {
        let o = DateOverride {
            date: "2026-12-25".into(),
            name: None,
            ranges: vec![],
        };
        assert!(
            validate_overrides(&[o.clone(), o])
                .unwrap_err()
                .contains("Duplicate")
        );
    }
}
