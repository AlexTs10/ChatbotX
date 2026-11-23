//! `packages/business/src/response-sla/service.ts`: the workspace policy.
//! ponytail: no read cache (one process, SQLite reads are cheap), so no
//! warm-up and one full-year working-time walk instead of the cached 35-day
//! walk plus the fallback (same result). The window snapshots
//! ([`track_window`], [`compute_snapshot`]) run inside the caller's write
//! transaction (one writer, so no row lock) and never fail it.

use std::collections::{BTreeMap, HashMap, HashSet};

use chatbotx_core::js;
use chatbotx_core::utils::tz::Zone;
use chatbotx_db::Db;
use chatbotx_db::repo::models::ResponseSlaPolicy;
use chatbotx_db::repo::response_sla::{
    self as repo, ResponseSlaPolicyInput, SlaSnapshot, WindowState,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

pub const MIN_TARGET_SECONDS: i64 = 60;
pub const MAX_TARGET_SECONDS: i64 = 30 * 24 * 60 * 60;

/// `toResponseSlaResource`.
pub fn resource(row: &ResponseSlaPolicy) -> Value {
    json!({
        "enabled": row.enabled,
        "firstResponseTargetSeconds": row.first_response_target_seconds,
        "nextResponseTargetSeconds": row.next_response_target_seconds,
        "breachingThresholdPercent": row.breaching_threshold_percent,
        "createdAt": chatbotx_db::repo::iso(row.created_at),
        "updatedAt": chatbotx_db::repo::iso(row.updated_at),
    })
}

/// `parsePolicy`.
fn parse_policy(data: &ResponseSlaPolicyInput) -> Result<()> {
    let in_range = |v: i64| (MIN_TARGET_SECONDS..=MAX_TARGET_SECONDS).contains(&v);
    if !in_range(data.first_response_target_seconds) {
        return Err(BusinessError::validation(
            "firstResponseTargetSeconds",
            format!(
                "Must be a whole number of seconds between {MIN_TARGET_SECONDS} and {MAX_TARGET_SECONDS}"
            ),
        ));
    }
    if data
        .next_response_target_seconds
        .is_some_and(|v| !in_range(v))
    {
        return Err(BusinessError::validation(
            "nextResponseTargetSeconds",
            format!(
                "Must be null or a whole number of seconds between {MIN_TARGET_SECONDS} and {MAX_TARGET_SECONDS}"
            ),
        ));
    }
    if !(1..=99).contains(&data.breaching_threshold_percent) {
        return Err(BusinessError::validation(
            "breachingThresholdPercent",
            "Must be a whole number between 1 and 99",
        ));
    }
    Ok(())
}

/// `findPolicy`.
pub async fn find_policy(db: &Db, workspace_id: i64) -> Result<Option<ResponseSlaPolicy>> {
    tx::read(db, move |c| Ok(repo::find_policy(c, workspace_id)?)).await
}

/// `upsertPolicy`: a disabled policy clears the open windows' deadlines.
/// Returns the row and the audit detail.
pub async fn upsert_policy(
    db: &Db,
    workspace_id: i64,
    data: ResponseSlaPolicyInput,
) -> Result<(ResponseSlaPolicy, &'static str)> {
    parse_policy(&data)?;
    let row = tx::write(db, move |c| {
        let saved = repo::upsert_policy(c, workspace_id, data)?;
        if !saved.enabled {
            repo::clear_workspace_snapshots(c, workspace_id)?;
        }
        Ok(saved)
    })
    .await?;
    let detail = if row.enabled {
        "updated the response SLA policy"
    } else {
        "disabled the response SLA policy"
    };
    Ok((row, detail))
}

/// `deletePolicy`: `true` when a policy was removed.
pub async fn delete_policy(db: &Db, workspace_id: i64) -> Result<bool> {
    tx::write(db, move |c| {
        let removed = repo::delete_policy(c, workspace_id)?;
        repo::clear_workspace_snapshots(c, workspace_id)?;
        Ok(removed)
    })
    .await
}

// ─── Working-time arithmetic (`working-hours/duration.ts`) ─────────────────

const MS_PER_MINUTE: i64 = 60_000;
const MS_PER_DAY: i64 = 86_400_000;
const MINUTES_PER_DAY: i64 = 1440;
/// `MAX_WORKING_WALK_DAYS`.
pub const MAX_WORKING_WALK_DAYS: i64 = 366;

/// `(startMinute, endMinute)` of one open range on a local date.
pub type Range = (i64, i64);

/// One local date, as days since 1970-01-01.
fn local_day(zone: &Zone, instant_ms: i64) -> i64 {
    let f = zone.wall_clock(instant_ms);
    js::days_from_civil(f.year, i64::from(f.month) - 1, i64::from(f.day))
}

fn date_key(day: i64) -> String {
    let (y, m, d) = js::civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `offsetAt`: the zone offset at the whole second of an instant.
fn offset_at(zone: &Zone, instant_ms: i64) -> i64 {
    zone.offset_ms(instant_ms.div_euclid(1000) * 1000)
}

/// `wallMinuteToInstant`: a nonexistent time (spring gap) is the transition
/// instant; an ambiguous one (fall-back overlap) the first occurrence.
fn wall_minute_to_instant(day: i64, minute: i64, zone: &Zone) -> i64 {
    let wall = day * MS_PER_DAY + minute * MS_PER_MINUTE;
    let before = offset_at(zone, wall - MS_PER_DAY);
    let after = offset_at(zone, wall + MS_PER_DAY);
    if before == after {
        return wall - before;
    }
    let (cand_before, cand_after) = (wall - before, wall - after);
    let before_valid = offset_at(zone, cand_before) == before;
    let after_valid = offset_at(zone, cand_after) == after;
    match (before_valid, after_valid) {
        (true, true) => cand_before.min(cand_after),
        (true, false) => cand_before,
        (false, true) => cand_after,
        (false, false) => {
            let mut low = (wall - MS_PER_DAY).div_euclid(MS_PER_MINUTE);
            let mut high = -(-(wall + MS_PER_DAY)).div_euclid(MS_PER_MINUTE);
            while high - low > 1 {
                let mid = (low + high).div_euclid(2);
                if offset_at(zone, mid * MS_PER_MINUTE) == before {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            high * MS_PER_MINUTE
        }
    }
}

/// `addWorkingSeconds`: the instant `seconds` of working time after
/// `from_ms`, walking the local dates in `zone`; a date override (by
/// `YYYY-MM-DD`) replaces that date's weekly intervals. `None` when not
/// reached within `max_days` dates.
pub fn add_working_seconds(
    from_ms: i64,
    seconds: i64,
    intervals: &[(i64, i64, i64)],
    zone: &Zone,
    overrides: &HashMap<String, Vec<Range>>,
    max_days: i64,
) -> Option<i64> {
    if seconds < 0 {
        return None;
    }
    if seconds == 0 {
        return Some(from_ms);
    }
    if intervals.is_empty() && overrides.is_empty() {
        return None;
    }
    let mut weekly: BTreeMap<i64, Vec<Range>> = BTreeMap::new();
    for &(weekday, start, end) in intervals {
        weekly.entry(weekday).or_default().push((start, end));
    }
    for day in weekly.values_mut() {
        day.sort_unstable();
    }
    let mut remaining = seconds * 1000;
    let mut day = local_day(zone, from_ms);
    for _ in 0..=max_days {
        let ranges = match overrides.get(&date_key(day)) {
            Some(o) => {
                let mut o = o.clone();
                o.sort_unstable();
                o
            }
            // 1970-01-01 (day 0) was a Thursday (4).
            None => weekly
                .get(&(day + 4).rem_euclid(7))
                .cloned()
                .unwrap_or_default(),
        };
        for (start, end) in ranges {
            let start_ms = wall_minute_to_instant(day, start, zone);
            let end_ms = if end >= MINUTES_PER_DAY {
                wall_minute_to_instant(day + 1, 0, zone)
            } else {
                wall_minute_to_instant(day, end, zone)
            };
            let usable = start_ms.max(from_ms);
            let available = end_ms - usable;
            if available <= 0 {
                continue;
            }
            if available >= remaining {
                return Some(usable + remaining);
            }
            remaining -= available;
        }
        day += 1;
    }
    None
}

// ─── Snapshot and window tracking (`response-sla/service.ts`) ──────────────

/// `computeSnapshot`: the deadline of a window that started at `start_ms`,
/// in the assignee's working time (calendar time without a schedule).
/// `None` when a working-time deadline is not reached within a year.
pub fn compute_snapshot(
    conn: &Connection,
    workspace_id: i64,
    assigned_user_id: Option<i64>,
    start_ms: i64,
    target_seconds: i64,
    breaching_threshold_percent: i64,
) -> chatbotx_db::repo::Result<Option<SlaSnapshot>> {
    let breaching_seconds =
        (((target_seconds * (100 - breaching_threshold_percent)) as f64 / 100.0).round() as i64)
            .max(1);
    let to_ms = start_ms + MAX_WORKING_WALK_DAYS * MS_PER_DAY;
    let Some(schedule) = load_schedule(conn, workspace_id, assigned_user_id, start_ms, to_ms)?
    else {
        return Ok(Some(SlaSnapshot {
            response_due_at: start_ms + target_seconds * 1000,
            response_sla_breaching_at: start_ms + breaching_seconds * 1000,
            response_sla_target_seconds: target_seconds,
        }));
    };
    let walk = |seconds| {
        add_working_seconds(
            start_ms,
            seconds,
            &schedule.intervals,
            &schedule.zone,
            &schedule.overrides,
            MAX_WORKING_WALK_DAYS,
        )
    };
    match (walk(target_seconds), walk(breaching_seconds)) {
        (Some(due), Some(breaching)) => Ok(Some(SlaSnapshot {
            response_due_at: due,
            response_sla_breaching_at: breaching,
            response_sla_target_seconds: target_seconds,
        })),
        _ => {
            tracing::warn!(
                workspace_id,
                target_seconds,
                "Response SLA deadline not reached within the walk limit; no SLA set"
            );
            Ok(None)
        }
    }
}

/// An agent's working schedule as walk inputs.
pub struct LoadedSchedule {
    pub working_hours_id: i64,
    pub intervals: Vec<(i64, i64, i64)>,
    pub zone: Zone,
    pub overrides: HashMap<String, Vec<Range>>,
}

/// `workingHoursService.findForAgent` + `resolveWorkingHoursTimezone` +
/// `findDateOverridesFor(from, to)`: `None` without a user or a schedule.
pub fn load_schedule(
    conn: &Connection,
    workspace_id: i64,
    user_id: Option<i64>,
    from_ms: i64,
    to_ms: i64,
) -> chatbotx_db::repo::Result<Option<LoadedSchedule>> {
    let schedule = match user_id {
        Some(user) => chatbotx_db::repo::working_hours::find_for_user(conn, workspace_id, user)?,
        None => None,
    };
    let Some(schedule) = schedule else {
        return Ok(None);
    };
    let timezone: Option<String> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let resolved = chatbotx_core::utils::datetime::resolve_filter_timezone(timezone.as_deref());
    let zone = Zone::parse(&resolved).unwrap_or_else(Zone::utc);
    // `findDateOverridesFor` (UTC date keys a day either side of the walk).
    let from_key = date_key((from_ms - MS_PER_DAY).div_euclid(MS_PER_DAY));
    let to_key = date_key((to_ms + MS_PER_DAY).div_euclid(MS_PER_DAY));
    let rows = chatbotx_db::repo::working_hours::list_date_overrides_in_range(
        conn,
        workspace_id,
        &from_key,
        &to_key,
    )?;
    let ranges_of = |v: &serde_json::Value| -> Vec<Range> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| Some((r["startMinute"].as_i64()?, r["endMinute"].as_i64()?)))
            .collect()
    };
    // `buildDateOverridesForSchedule`: holidays, then the schedule's own rows.
    let mut overrides = HashMap::new();
    for own in [false, true] {
        for row in &rows {
            let applies = if own {
                row.working_hours_id == Some(schedule.id)
            } else {
                row.working_hours_id.is_none()
            };
            if applies {
                overrides.insert(row.date.clone(), ranges_of(&row.ranges));
            }
        }
    }
    let intervals: Vec<(i64, i64, i64)> = schedule
        .intervals
        .iter()
        .map(|i| (i.weekday, i.start_minute, i.end_minute))
        .collect();
    Ok(Some(LoadedSchedule {
        working_hours_id: schedule.id,
        intervals,
        zone,
        overrides,
    }))
}

/// `workingSecondsBetween`: whole working seconds in `[from_ms, to_ms)`; a date
/// override replaces that local date's weekly intervals.
pub fn working_seconds_between(
    from_ms: i64,
    to_ms: i64,
    intervals: &[(i64, i64, i64)],
    zone: &Zone,
    overrides: &HashMap<String, Vec<Range>>,
) -> i64 {
    if to_ms <= from_ms || (intervals.is_empty() && overrides.is_empty()) {
        return 0;
    }
    let mut weekly: BTreeMap<i64, Vec<Range>> = BTreeMap::new();
    for &(weekday, start, end) in intervals {
        weekly.entry(weekday).or_default().push((start, end));
    }
    let (mut day, last) = (local_day(zone, from_ms), local_day(zone, to_ms));
    let mut total = 0;
    while day <= last {
        let ranges = match overrides.get(&date_key(day)) {
            Some(o) => o.clone(),
            // 1970-01-01 (day 0) was a Thursday (4).
            None => weekly
                .get(&(day + 4).rem_euclid(7))
                .cloned()
                .unwrap_or_default(),
        };
        for (start, end) in ranges {
            let start_ms = wall_minute_to_instant(day, start, zone);
            let end_ms = if end >= MINUTES_PER_DAY {
                wall_minute_to_instant(day + 1, 0, zone)
            } else {
                wall_minute_to_instant(day, end, zone)
            };
            let overlap = end_ms.min(to_ms) - start_ms.max(from_ms);
            if overlap > 0 {
                total += overlap;
            }
        }
        day += 1;
    }
    total / 1000
}

/// `trackWindow`: runs `run` (a statement that can open the DM waiting
/// window or change the assignee of `ids`) and keeps the SLA snapshot in
/// step: a window it opened gets a deadline, an open window whose assignee
/// changed gets it recomputed. `may_reassign` = the caller passes assignee
/// hints (always the full path). SLA bookkeeping never fails the caller: it
/// runs in a savepoint and its errors are logged.
pub fn track_window<T, E>(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    may_reassign: bool,
    run: impl FnOnce(&Connection) -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let policy = match repo::find_policy(conn, workspace_id) {
        Ok(policy) => policy,
        Err(err) => {
            tracing::warn!(workspace_id, err = %err, "Response SLA policy read failed");
            return run(conn);
        }
    };
    let Some(policy) = policy.filter(|p| p.enabled && !ids.is_empty()) else {
        return run(conn);
    };
    if !may_reassign && repo::are_windows_already_open(conn, workspace_id, ids).unwrap_or(false) {
        return run(conn);
    }
    let Ok(before) = repo::read_window_states(conn, workspace_id, ids, true) else {
        return run(conn);
    };
    let result = run(conn)?;
    let synced = (|| -> chatbotx_db::repo::Result<()> {
        let after = repo::read_window_states(conn, workspace_id, ids, false)?;
        let changes = detect_changes(&before, &after);
        if changes.is_empty() {
            return Ok(());
        }
        conn.execute_batch("SAVEPOINT response_sla")?;
        match sync_snapshots(conn, workspace_id, &policy, &changes) {
            Ok(()) => conn.execute_batch("RELEASE response_sla")?,
            Err(err) => {
                conn.execute_batch("ROLLBACK TO response_sla; RELEASE response_sla")?;
                tracing::warn!(
                    workspace_id,
                    err = %err,
                    "Response SLA snapshot update failed; window left without a deadline"
                );
                // A wrong deadline is worse than none.
                drop_stale_snapshots(conn, workspace_id, &before, &after)?;
            }
        }
        Ok(())
    })();
    if let Err(err) = synced {
        tracing::warn!(workspace_id, err = %err, "Response SLA bookkeeping failed");
    }
    Ok(result)
}

/// `SlaWindowChange`: `(state, since, opened, reassigned)`.
type Change<'a> = (&'a WindowState, i64, bool, bool);

/// `detectChanges`.
fn detect_changes<'a>(before: &[WindowState], after: &'a [WindowState]) -> Vec<Change<'a>> {
    let previous: HashMap<i64, &WindowState> = before.iter().map(|s| (s.id, s)).collect();
    after
        .iter()
        .filter_map(|current| {
            let since = current.awaiting_human_reply_since?;
            if current.source_id.is_some() {
                return None;
            }
            let prior = previous.get(&current.id);
            let opened = prior.is_some_and(|p| p.awaiting_human_reply_since.is_none());
            let reassigned = !opened
                && current.response_due_at.is_some()
                && prior.map(|p| p.assigned_user_id) != Some(current.assigned_user_id);
            (opened || reassigned).then_some((current, since, opened, reassigned))
        })
        .collect()
}

/// `syncSnapshots`.
fn sync_snapshots(
    conn: &Connection,
    workspace_id: i64,
    policy: &ResponseSlaPolicy,
    changes: &[Change<'_>],
) -> chatbotx_db::repo::Result<()> {
    for &(current, since, opened, reassigned) in changes {
        let target = if opened {
            if current.admin_replied_at.is_none() {
                Some(policy.first_response_target_seconds)
            } else {
                policy.next_response_target_seconds
            }
        } else {
            current.response_sla_target_seconds
        };
        let Some(target) = target else { continue };
        // `resolveMeasuredFrom` with the assignee as the measured admin.
        let start = match (current.assigned_user_id, current.assigned_user_at) {
            (Some(_), Some(at)) if at > since => at,
            _ => since,
        };
        let snapshot = compute_snapshot(
            conn,
            workspace_id,
            current.assigned_user_id,
            start,
            target,
            policy.breaching_threshold_percent,
        )?;
        if let Some(snapshot) = snapshot {
            repo::set_snapshot_if_window_unchanged(
                conn,
                workspace_id,
                current.id,
                since,
                snapshot,
            )?;
        } else if reassigned {
            repo::clear_snapshot_if_window_unchanged(conn, workspace_id, current.id, since)?;
        }
    }
    Ok(())
}

/// `dropStaleSnapshots`: clears the deadline of windows open before and after.
fn drop_stale_snapshots(
    conn: &Connection,
    workspace_id: i64,
    before: &[WindowState],
    after: &[WindowState],
) -> chatbotx_db::repo::Result<()> {
    let open_before: HashSet<i64> = before
        .iter()
        .filter(|s| s.awaiting_human_reply_since.is_some())
        .map(|s| s.id)
        .collect();
    for state in after {
        if let (Some(since), Some(_)) = (state.awaiting_human_reply_since, state.response_due_at)
            && open_before.contains(&state.id)
        {
            repo::clear_snapshot_if_window_unchanged(conn, workspace_id, state.id, since)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON_9: i64 = 1_704_099_600_000; // 2024-01-01T09:00Z, a Monday.

    #[test]
    fn adds_working_seconds_across_closed_days_and_overrides() {
        let utc = Zone::utc();
        let weekdays: Vec<_> = (1..=5).map(|d| (d, 9 * 60, 17 * 60)).collect();
        let none = HashMap::new();
        // One hour inside the shift.
        assert_eq!(
            add_working_seconds(MON_9, 3600, &weekdays, &utc, &none, 366),
            Some(MON_9 + 3_600_000)
        );
        // Friday 16:00 + 2h -> Monday 10:00.
        let fri_16 = MON_9 + 4 * MS_PER_DAY + 7 * 3_600_000;
        assert_eq!(
            add_working_seconds(fri_16, 7200, &weekdays, &utc, &none, 366),
            Some(MON_9 + 7 * MS_PER_DAY + 3_600_000)
        );
        // A holiday on Monday pushes the start to Tuesday.
        let holiday = HashMap::from([("2024-01-01".to_owned(), vec![])]);
        assert_eq!(
            add_working_seconds(MON_9, 60, &weekdays, &utc, &holiday, 366),
            Some(MON_9 + MS_PER_DAY + 60_000)
        );
        // No schedule at all never reaches the target.
        assert_eq!(add_working_seconds(MON_9, 60, &[], &utc, &none, 366), None);
        assert_eq!(
            add_working_seconds(MON_9, 0, &[], &utc, &none, 366),
            Some(MON_9)
        );
    }

    fn z(iso: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(iso)
            .expect("iso")
            .timestamp_millis()
    }

    /// `working-hours-duration.test.ts` `workingSecondsBetween` (UTC).
    #[test]
    fn working_seconds_between_cases_of_the_duration_test() {
        let utc = Zone::utc();
        let none = HashMap::new();
        let whole_day: Vec<_> = (0..7).map(|d| (d, 0, 1440)).collect();
        // Floors fractional seconds.
        assert_eq!(
            working_seconds_between(
                z("2026-06-01T09:00:00.100Z"),
                z("2026-06-01T09:01:00.900Z"),
                &whole_day,
                &utc,
                &none
            ),
            60
        );
        let t = z("2026-06-01T10:00:00Z");
        assert_eq!(working_seconds_between(t, t, &whole_day, &utc, &none), 0);
        assert_eq!(
            working_seconds_between(z("2026-06-01T11:00:00Z"), t, &whole_day, &utc, &none),
            0
        );
        assert_eq!(
            working_seconds_between(
                z("2026-06-01T00:00:00Z"),
                z("2026-06-08T00:00:00Z"),
                &[],
                &utc,
                &none
            ),
            0
        );
        // Sat 12:00 -> Mon 10:30 on Mon-Fri 09-17 = 1.5h.
        let weekdays: Vec<_> = (1..=5).map(|d| (d, 540, 1020)).collect();
        assert_eq!(
            working_seconds_between(
                z("2026-06-06T12:00:00Z"),
                z("2026-06-08T10:30:00Z"),
                &weekdays,
                &utc,
                &none
            ),
            5400
        );
        // A holiday Monday counts nothing.
        let holiday = HashMap::from([("2026-06-08".to_owned(), vec![])]);
        assert_eq!(
            working_seconds_between(
                z("2026-06-06T12:00:00Z"),
                z("2026-06-08T10:30:00Z"),
                &weekdays,
                &utc,
                &holiday
            ),
            0
        );
    }

    /// `working-hours-duration.test.ts` `addWorkingSeconds` (Monday 09-17, UTC).
    #[test]
    fn add_working_seconds_cases_of_the_duration_test() {
        let utc = Zone::utc();
        let monday = [(1, 540, 1020)];
        let none = HashMap::new();
        let add = |from: &str, seconds: i64, overrides: &HashMap<String, Vec<Range>>, max: i64| {
            add_working_seconds(z(from), seconds, &monday, &utc, overrides, max)
        };
        assert_eq!(
            add("2026-06-01T03:00:00Z", 0, &none, 366),
            Some(z("2026-06-01T03:00:00Z"))
        );
        assert_eq!(add("2026-06-01T03:00:00Z", -1, &none, 366), None);
        assert_eq!(
            add("2026-06-01T03:00:00Z", 3600, &none, 366),
            Some(z("2026-06-01T10:00:00Z"))
        );
        assert_eq!(
            add("2026-06-01T10:30:00Z", 1800, &none, 366),
            Some(z("2026-06-01T11:00:00Z"))
        );
        assert_eq!(
            add("2026-06-01T16:00:00Z", 3600, &none, 366),
            Some(z("2026-06-01T17:00:00Z"))
        );
        let closed = HashMap::from([("2026-06-01".to_owned(), vec![])]);
        assert_eq!(
            add("2026-06-01T08:00:00Z", 3600, &closed, 366),
            Some(z("2026-06-08T10:00:00Z"))
        );
        let opened = HashMap::from([("2026-06-02".to_owned(), vec![(600, 660)])]);
        assert_eq!(
            add("2026-06-02T00:00:00Z", 1800, &opened, 366),
            Some(z("2026-06-02T10:30:00Z"))
        );
        assert_eq!(add("2026-06-01T08:00:00Z", 60, &closed, 3), None);
        // More working time than one year of Mondays can hold.
        assert_eq!(add("2026-06-01T08:00:00Z", 60 * 8 * 3600, &none, 366), None);
        // New York spring-forward Sunday: 09:00 local = 13:00Z.
        let ny = Zone::parse("America/New_York").expect("zone");
        assert_eq!(
            add_working_seconds(
                z("2026-03-08T00:00:00Z"),
                1800,
                &[(0, 540, 600)],
                &ny,
                &none,
                366
            ),
            Some(z("2026-03-08T13:30:00Z"))
        );
    }

    #[test]
    fn a_spring_gap_wall_time_is_the_transition_instant() {
        let berlin = Zone::parse("Europe/Berlin").expect("zone");
        // 2024-03-31 02:30 does not exist in Berlin; 03:00 CEST = 01:00Z.
        let day = js::days_from_civil(2024, 2, 31);
        assert_eq!(
            wall_minute_to_instant(day, 150, &berlin),
            js::date_utc(2024, 2, 31, 1, 0, 0, 0)
        );
    }
}
