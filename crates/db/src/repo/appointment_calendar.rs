//! Port of `repositories/appointment-calendar/repository.ts` (`replaceReminders`; the
//! other functions are listed as not ported yet in `conversion/specs/P3.2.md`).

use std::collections::HashSet;

use rusqlite::Connection;

use super::models::AppointmentCalendarReminder;
use super::{Result, id_list, new_id};

/// One wanted reminder of [`replace_reminders`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderInput {
    pub flow_id: i64,
    pub timing_value: i64,
    /// `minutes`, `hours` or `days`.
    pub timing_unit: String,
}

fn key(flow_id: i64, value: i64, unit: &str) -> String {
    format!("{flow_id}:{value}:{unit}")
}

/// `replaceReminders`: keeps unchanged reminders (so their dispatch history is not
/// cascaded), deletes the ones not wanted any more and inserts the new ones. Returns the
/// kept reminders then the inserted ones.
pub fn replace_reminders(
    conn: &Connection,
    calendar_id: i64,
    reminders: &[ReminderInput],
) -> Result<Vec<AppointmentCalendarReminder>> {
    todo!("P3.2");
    let existing: Vec<AppointmentCalendarReminder> =
        super::find_many(conn, "WHERE \"calendarId\" = ?1", &[&calendar_id])?;
    let next: HashSet<String> = reminders
        .iter()
        .map(|r| key(r.flow_id, r.timing_value, &r.timing_unit))
        .collect();
    let existing_keys: HashSet<String> = existing
        .iter()
        .map(|r| key(r.flow_id, r.timing_value, &r.timing_unit))
        .collect();
    let delete_ids: Vec<i64> = existing
        .iter()
        .filter(|r| !next.contains(&key(r.flow_id, r.timing_value, &r.timing_unit)))
        .map(|r| r.id)
        .collect();
    if !delete_ids.is_empty() {
        conn.execute(
            "DELETE FROM \"AppointmentCalendarReminder\" WHERE \"id\" IN (SELECT value FROM json_each(?1))",
            [id_list(&delete_ids)],
        )?;
    }
    let mut out: Vec<AppointmentCalendarReminder> = existing
        .into_iter()
        .filter(|r| next.contains(&key(r.flow_id, r.timing_value, &r.timing_unit)))
        .collect();
    for r in reminders
        .iter()
        .filter(|r| !existing_keys.contains(&key(r.flow_id, r.timing_value, &r.timing_unit)))
    {
        let rows = super::returning::<AppointmentCalendarReminder>(
            conn,
            "INSERT INTO \"AppointmentCalendarReminder\" (\"id\", \"calendarId\", \"flowId\", \"timingValue\", \"timingUnit\") VALUES (?1, ?2, ?3, ?4, ?5)",
            &[&new_id(), &calendar_id, &r.flow_id, &r.timing_value, &r.timing_unit],
        )?;
        out.extend(rows);
    }
    Ok(out)
}
