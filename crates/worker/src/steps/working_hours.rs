//! `working-hours/on-shift.ts` `listOnShiftUserIds` (+ `isWithinWorkingHours`
//! and `buildDateOverridesForSchedule` of `duration.ts` / `date-overrides.ts`)
//! for the automated assignment steps. The working-hours domain itself is
//! Q.D9; this is the read side only.

use std::collections::{HashMap, HashSet};

use chatbotx_core::utils::datetime::resolve_filter_timezone;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use chrono::{Datelike, TimeZone, Timelike};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interval {
    weekday: u32,
    start: u32,
    end: u32,
}

/// `isWithinWorkingHours`: `start <= minute < end` on the local date; a date
/// override replaces that date's weekly intervals.
fn is_within(
    at_ms: i64,
    intervals: &[Interval],
    timezone: &str,
    overrides: &HashMap<String, Vec<(u32, u32)>>,
) -> bool {
    let Ok(tz) = timezone.parse::<chrono_tz::Tz>() else {
        return false;
    };
    let Some(local) = tz.timestamp_millis_opt(at_ms).single() else {
        return false;
    };
    let key = local.format("%Y-%m-%d").to_string();
    let weekday = local.weekday().num_days_from_sunday();
    let minute = local.hour() * 60 + local.minute();
    match overrides.get(&key) {
        Some(ranges) => ranges.iter().any(|(s, e)| *s <= minute && minute < *e),
        None => intervals
            .iter()
            .any(|i| i.weekday == weekday && i.start <= minute && minute < i.end),
    }
}

fn ranges_of(json: &str) -> Vec<(u32, u32)> {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some((
                u32::try_from(r.get("startMinute")?.as_u64()?).ok()?,
                u32::try_from(r.get("endMinute")?.as_u64()?).ok()?,
            ))
        })
        .collect()
}

fn on_shift_in(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
    at_ms: i64,
) -> Result<HashSet<i64>, DbError> {
    // (userId, workingHoursId) of the candidates that have a schedule.
    let mut scheduled: HashMap<i64, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        r#"SELECT "userId", "workingHoursId" FROM "WorkingHoursMember" WHERE "workspaceId" = ?1"#,
    )?;
    let rows = stmt.query_map(params![workspace_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    for row in rows {
        let (user, schedule): (i64, i64) = row?;
        if user_ids.contains(&user) {
            scheduled.insert(user, schedule);
        }
    }
    let mut on_shift: HashSet<i64> = user_ids
        .iter()
        .copied()
        .filter(|u| !scheduled.contains_key(u))
        .collect();
    if scheduled.is_empty() {
        return Ok(on_shift);
    }
    let timezone: Option<String> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            params![workspace_id],
            |r| r.get(0),
        )
        .ok();
    let timezone = resolve_filter_timezone(timezone.as_deref());
    // The UTC date ± 1 day covers every zone's local date.
    let day = |offset: i64| {
        chrono::DateTime::from_timestamp_millis(at_ms + offset * 86_400_000)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    };
    let mut workspace_overrides = HashMap::new();
    let mut schedule_overrides: HashMap<i64, HashMap<String, Vec<(u32, u32)>>> = HashMap::new();
    let mut stmt = conn.prepare(
        r#"SELECT "workingHoursId", "date", "ranges" FROM "WorkingHoursDateOverride"
           WHERE "workspaceId" = ?1 AND "date" BETWEEN ?2 AND ?3"#,
    )?;
    let rows = stmt.query_map(params![workspace_id, day(-1), day(1)], |r| {
        Ok((
            r.get::<_, Option<i64>>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (schedule, date, ranges) = row?;
        match schedule {
            None => {
                workspace_overrides.insert(date, ranges_of(&ranges));
            }
            Some(id) => {
                schedule_overrides
                    .entry(id)
                    .or_default()
                    .insert(date, ranges_of(&ranges));
            }
        }
    }
    for schedule in scheduled.values().copied().collect::<HashSet<_>>() {
        let mut stmt = conn.prepare(
            r#"SELECT "weekday", "startMinute", "endMinute" FROM "WorkingHoursInterval" WHERE "workingHoursId" = ?1"#,
        )?;
        let intervals = stmt
            .query_map(params![schedule], |r| {
                Ok(Interval {
                    weekday: r.get(0)?,
                    start: r.get(1)?,
                    end: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut overrides = workspace_overrides.clone();
        overrides.extend(schedule_overrides.remove(&schedule).unwrap_or_default());
        if is_within(at_ms, &intervals, &timezone, &overrides) {
            on_shift.extend(
                scheduled
                    .iter()
                    .filter(|(_, s)| **s == schedule)
                    .map(|(u, _)| *u),
            );
        }
    }
    Ok(on_shift)
}

/// `listOnShiftUserIds`: users without a schedule are always on shift.
pub(crate) async fn list_on_shift_user_ids(
    db: &Db,
    workspace_id: i64,
    user_ids: Vec<i64>,
    at_ms: i64,
) -> Result<HashSet<i64>, DbError> {
    db.read(move |conn| on_shift_in(conn, workspace_id, &user_ids, at_ms))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_override_replaces_the_weekly_intervals() {
        // 2026-11-16 is a Monday (weekday 1); 10:00 UTC.
        let at = chrono::Utc
            .with_ymd_and_hms(2026, 11, 16, 10, 0, 0)
            .unwrap()
            .timestamp_millis();
        let weekly = [Interval {
            weekday: 1,
            start: 9 * 60,
            end: 17 * 60,
        }];
        assert!(is_within(at, &weekly, "UTC", &HashMap::new()));
        assert!(!is_within(at, &weekly, "Asia/Tokyo", &HashMap::new()));
        let holiday = HashMap::from([("2026-11-16".to_string(), Vec::new())]);
        assert!(!is_within(at, &weekly, "UTC", &holiday));
    }
}
