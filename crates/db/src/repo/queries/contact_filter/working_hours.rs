//! Port of `working-hours-predicates.ts`: `isWithinWorkingHours`.
//!
//! The TS read `now() AT TIME ZONE <zone>` in SQL; here the local wall clock of `now` in
//! the zone is computed once in Rust and bound as the local day (`YYYY-MM-DD`), the
//! minute of the day and the weekday (0 = Sunday, `EXTRACT(DOW)`).

use chrono::{DateTime, Datelike, Timelike};
use chrono_tz::Tz;
use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::functions::wall_ms;
use crate::repo::sql::Sql;
use crate::repo::utils::is_bigint_id;

/// `ASSIGNED_AGENT_WORKING_HOURS_VALUE`.
pub const ASSIGNED_AGENT_WORKING_HOURS_VALUE: &str = "assignedAgent";

struct LocalNow {
    day: String,
    minute: i64,
    weekday: i64,
}

fn local_now(now: i64, zone: Tz) -> LocalNow {
    let wall = wall_ms(now, zone).unwrap_or(now);
    let t = DateTime::from_timestamp_millis(wall)
        .unwrap_or_default()
        .naive_utc();
    LocalNow {
        day: format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day()),
        minute: i64::from(t.hour() * 60 + t.minute()),
        weekday: i64::from(t.weekday().num_days_from_sunday()),
    }
}

const OVERRIDE: &str = "\"WorkingHoursDateOverride\"";

fn override_on(out: &mut Sql, workspace_id: i64, local: &LocalNow, scope: &str) {
    out.push(&format!("{OVERRIDE}.\"workspaceId\" = "));
    out.push_bind(workspace_id);
    out.push(&format!(" AND {OVERRIDE}.\"date\" = "));
    out.push_bind(local.day.clone());
    out.push(&format!(" AND {scope}"));
}

fn override_exists(out: &mut Sql, workspace_id: i64, local: &LocalNow, scope: &str) {
    out.push(&format!("EXISTS (SELECT 1 FROM {OVERRIDE} WHERE "));
    override_on(out, workspace_id, local, scope);
    out.push(")");
}

fn override_open(out: &mut Sql, workspace_id: i64, local: &LocalNow, scope: &str) {
    out.push(&format!(
        "EXISTS (SELECT 1 FROM {OVERRIDE}, json_each({OVERRIDE}.\"ranges\") AS date_range WHERE "
    ));
    override_on(out, workspace_id, local, scope);
    out.push(" AND CAST(json_extract(date_range.value, '$.startMinute') AS INTEGER) <= ");
    out.push_bind(local.minute);
    out.push(" AND ");
    out.push_bind(local.minute);
    out.push(" < CAST(json_extract(date_range.value, '$.endMinute') AS INTEGER))");
}

/// `scheduleOpenNowSql`: the schedule `<working_hours_id>` is open now.
fn schedule_open_now(working_hours_id: &str, workspace_id: i64, local: &LocalNow) -> Sql {
    let own = format!("{OVERRIDE}.\"workingHoursId\" = {working_hours_id}");
    let holiday = format!("{OVERRIDE}.\"workingHoursId\" IS NULL");
    let mut out = Sql::raw("(CASE WHEN ");
    override_exists(&mut out, workspace_id, local, &own);
    out.push(" THEN ");
    override_open(&mut out, workspace_id, local, &own);
    out.push(" WHEN ");
    override_exists(&mut out, workspace_id, local, &holiday);
    out.push(" THEN ");
    override_open(&mut out, workspace_id, local, &holiday);
    out.push(&format!(
        " ELSE EXISTS (SELECT 1 FROM \"WorkingHoursInterval\" WHERE \"WorkingHoursInterval\".\"workingHoursId\" = {working_hours_id} AND \"WorkingHoursInterval\".\"weekday\" = "
    ));
    out.push_bind(local.weekday);
    out.push(" AND \"WorkingHoursInterval\".\"startMinute\" <= ");
    out.push_bind(local.minute);
    out.push(" AND ");
    out.push_bind(local.minute);
    out.push(" < \"WorkingHoursInterval\".\"endMinute\") END)");
    out
}

/// `buildIsWithinWorkingHoursWhere`.
pub(crate) fn working_hours_where(
    operator: &str,
    value: Option<&Json>,
    workspace_id: Option<i64>,
    zone: Tz,
    now: i64,
) -> Option<Sql> {
    let negate = operator == "notIn";
    let workspace_id = workspace_id?;
    if operator != "in" && !negate {
        return None;
    }
    let selections: Vec<&str> = match value {
        Some(Json::Array(a)) => a.iter().filter_map(Json::as_str).collect(),
        Some(Json::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    };
    let schedule_ids: Vec<Value> = selections
        .iter()
        .filter(|s| is_bigint_id(s))
        .filter_map(|s| s.parse::<i64>().ok())
        .map(Value::Integer)
        .collect();
    let assigned_agent = selections.contains(&ASSIGNED_AGENT_WORKING_HOURS_VALUE);
    if schedule_ids.is_empty() && !assigned_agent {
        return None;
    }
    let local = local_now(now, zone);
    let mut branches = Vec::new();
    if !schedule_ids.is_empty() {
        let mut s = Sql::bind(
            "EXISTS (SELECT 1 FROM \"WorkingHours\" WHERE \"WorkingHours\".\"workspaceId\" = ",
            workspace_id,
        );
        s.push(" AND ");
        s.append(super::sqlx::in_list(
            "\"WorkingHours\".\"id\"",
            schedule_ids,
            false,
        ));
        s.push(" AND ");
        s.append(schedule_open_now(
            "\"WorkingHours\".\"id\"",
            workspace_id,
            &local,
        ));
        branches.push(s.closed(")"));
    }
    if assigned_agent {
        let member = "\"WorkingHoursMember\"";
        let mut s = Sql::bind(
            "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"Contact\".\"id\" AND \"Conversation\".\"workspaceId\" = ",
            workspace_id,
        );
        s.push(&format!(
            " AND \"Conversation\".\"sourceId\" IS NULL AND \"Conversation\".\"assignedUserId\" IS NOT NULL AND (NOT EXISTS (SELECT 1 FROM {member} WHERE {member}.\"workspaceId\" = "
        ));
        s.push_bind(workspace_id);
        s.push(&format!(
            " AND {member}.\"userId\" = \"Conversation\".\"assignedUserId\") OR EXISTS (SELECT 1 FROM {member} WHERE {member}.\"workspaceId\" = "
        ));
        s.push_bind(workspace_id);
        s.push(&format!(
            " AND {member}.\"userId\" = \"Conversation\".\"assignedUserId\" AND "
        ));
        s.append(schedule_open_now(
            &format!("{member}.\"workingHoursId\""),
            workspace_id,
            &local,
        ));
        branches.push(s.closed(")))"));
    }
    let any = super::sqlx::join_raw(branches, " OR ")?;
    let mut out = Sql::raw(if negate { "NOT (" } else { "(" });
    out.append(any);
    Some(out.closed(")"))
}
