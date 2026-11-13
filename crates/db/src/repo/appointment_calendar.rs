//! Port of `repositories/appointment-calendar/repository.ts`. Ids are new snowflake ids
//! ([`new_id`]) where the TS used `createId()`.

use std::collections::HashSet;

use rusqlite::Connection;

use rusqlite::types::Value;

use super::models::{
    AppointmentCalendar, AppointmentCalendarAvailability, AppointmentCalendarReminder,
};
use super::patch::{Patch, set_clause};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortDir, SortItem, get_pagination_with_defaults, like_contains, order_by_sql,
    parse_order_by,
};
use super::{Error, Model, Result, id_list, new_id, now_ms};

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
            &[
                &new_id(),
                &calendar_id,
                &r.flow_id,
                &r.timing_value,
                &r.timing_unit,
            ],
        )?;
        out.extend(rows);
    }
    Ok(out)
}

/// `calendarWhere`: the workspace's live calendars (name `ILIKE %search%`).
fn calendar_where(workspace_id: i64, search: Option<&str>) -> Sql {
    let mut q = Sql::bind("\"workspaceId\" = ", workspace_id);
    q.push(" AND \"deletedAt\" IS NULL");
    if let Some(search) = search.filter(|s| !s.is_empty()) {
        q.push(" AND ilike(\"name\", ");
        q.push_bind(like_contains(search));
        q.push(")");
    }
    q
}

fn select_where(filter: Sql) -> Sql {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"AppointmentCalendar\" WHERE ",
        AppointmentCalendar::columns_sql(None)
    ));
    q.append(filter);
    q
}

/// `listNamesWithPrefix`: names of the workspace's live calendars that start with `prefix`
/// (an exact prefix compare: Postgres `LIKE` is case-sensitive, SQLite `LIKE` is not).
pub fn list_names_with_prefix(
    conn: &Connection,
    workspace_id: i64,
    prefix: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"name\" FROM \"AppointmentCalendar\" WHERE \"workspaceId\" = ?1 AND \"deletedAt\" IS NULL \
         AND substr(\"name\", 1, length(?2)) = ?2 ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, prefix], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

/// `AppointmentCalendarListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarListInput {
    pub workspace_id: i64,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarPage {
    pub data: Vec<AppointmentCalendar>,
    pub page_count: i64,
    pub total: i64,
}

/// `list`: one page of live calendars (sort by the given columns, else `name` asc).
pub fn list(conn: &Connection, input: &CalendarListInput) -> Result<CalendarPage> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut order = parse_order_by(AppointmentCalendar::COLUMNS, input.sort.as_deref(), None);
    if order.is_empty() {
        order = vec![("name".to_owned(), SortDir::Asc)];
    }
    let filter = calendar_where(input.workspace_id, input.search.as_deref());
    let mut q = select_where(filter.clone());
    q.push(" ");
    q.push(&order_by_sql(&order, None));
    q.push(" LIMIT ");
    q.push_bind(pagination.limit);
    q.push(" OFFSET ");
    q.push_bind(pagination.offset);
    let data = q.models(conn)?;
    let total: i64 = conn.query_row(
        &format!(
            "SELECT count(*) FROM \"AppointmentCalendar\" WHERE {}",
            filter.text
        ),
        rusqlite::params_from_iter(filter.params.iter()),
        |r| r.get(0),
    )?;
    Ok(CalendarPage {
        data,
        page_count: (total + pagination.limit - 1) / pagination.limit,
        total,
    })
}

/// `findBy`: the workspace's calendar (soft-deleted ones only with `include_deleted`).
pub fn find_by(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    include_deleted: bool,
) -> Result<Option<AppointmentCalendar>> {
    let mut filter = Sql::bind("\"id\" = ", id);
    filter.push(" AND \"workspaceId\" = ");
    filter.push_bind(workspace_id);
    if !include_deleted {
        filter.push(" AND \"deletedAt\" IS NULL");
    }
    select_where(filter).model(conn)
}

/// `findByPublicLinkSlug`: the workspace's live calendar with that slug.
pub fn find_by_public_link_slug(
    conn: &Connection,
    workspace_id: i64,
    public_link_slug: &str,
) -> Result<Option<AppointmentCalendar>> {
    let mut filter = Sql::bind("\"workspaceId\" = ", workspace_id);
    filter.push(" AND \"publicLinkSlug\" = ");
    filter.push_bind(public_link_slug.to_owned());
    filter.push(" AND \"deletedAt\" IS NULL");
    select_where(filter).model(conn)
}

/// One `listForFlow` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarFlowOption {
    pub id: i64,
    pub name: String,
    pub active: bool,
}

/// `listForFlow`: `(id, name, active)` of the live calendars (name `ILIKE %keyword%`), by name.
pub fn list_for_flow(
    conn: &Connection,
    workspace_id: i64,
    keyword: Option<&str>,
) -> Result<Vec<CalendarFlowOption>> {
    let filter = calendar_where(workspace_id, keyword);
    let sql = format!(
        "SELECT \"id\", \"name\", \"active\" FROM \"AppointmentCalendar\" WHERE {} ORDER BY \"name\" ASC",
        filter.text
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(filter.params.iter()), |r| {
            Ok(CalendarFlowOption {
                id: r.get(0)?,
                name: r.get(1)?,
                active: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `getForEdit` result: the calendar with its availability (weekday, start) and reminders
/// (timing value).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarForEdit {
    pub calendar: AppointmentCalendar,
    pub availability: Vec<AppointmentCalendarAvailability>,
    pub reminders: Vec<AppointmentCalendarReminder>,
}

/// `getForEdit`.
pub fn get_for_edit(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<CalendarForEdit>> {
    let Some(calendar) = find_by(conn, workspace_id, id, false)? else {
        return Ok(None);
    };
    let availability = super::find_many(
        conn,
        "WHERE \"calendarId\" = ?1 ORDER BY \"weekday\" ASC, \"startMinute\" ASC",
        &[&calendar.id],
    )?;
    let reminders = super::find_many(
        conn,
        "WHERE \"calendarId\" = ?1 ORDER BY \"timingValue\" ASC",
        &[&calendar.id],
    )?;
    Ok(Some(CalendarForEdit {
        calendar,
        availability,
        reminders,
    }))
}

/// `CreateAppointmentCalendarInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCalendarInput {
    pub workspace_id: i64,
    pub name: String,
    pub timezone: String,
    pub location_type: String,
    pub public_link_slug: String,
    /// `None` = the column default.
    pub active: Option<bool>,
}

/// `create`.
pub fn create(conn: &Connection, input: &CreateCalendarInput) -> Result<AppointmentCalendar> {
    let mut cols = vec![
        "\"id\"",
        "\"workspaceId\"",
        "\"name\"",
        "\"timezone\"",
        "\"locationType\"",
        "\"publicLinkSlug\"",
    ];
    let mut params: Vec<Value> = vec![
        Value::Integer(new_id()),
        Value::Integer(input.workspace_id),
        Value::Text(input.name.clone()),
        Value::Text(input.timezone.clone()),
        Value::Text(input.location_type.clone()),
        Value::Text(input.public_link_slug.clone()),
    ];
    if let Some(active) = input.active {
        cols.push("\"active\"");
        params.push(Value::Integer(i64::from(active)));
    }
    let marks = vec!["?"; cols.len()].join(", ");
    let q = Sql::new(
        format!(
            "INSERT INTO \"AppointmentCalendar\" ({}) VALUES ({marks}) RETURNING {}",
            cols.join(", "),
            AppointmentCalendar::columns_sql(None)
        ),
        params,
    );
    q.model(conn)?
        .ok_or_else(|| Error::msg("appointment calendar insert returned no row"))
}

/// The columns `update` may set (`UpdateAppointmentCalendarInput`).
pub const UPDATE_COLUMNS: &[&str] = &[
    "name",
    "description",
    "active",
    "timezone",
    "durationMinutes",
    "bufferAfterMinutes",
    "locationType",
    "locationDetail",
    "scheduleWindowType",
    "scheduleWindowConfig",
    "maxAppointmentsPerUser",
    "dailyLimitEnabled",
    "maxPerDay",
    "allowGroupMeeting",
    "maxPerSlot",
    "confirmationMessage",
    "confirmationFlowId",
    "cancellationFlowId",
    "externalConnectionId",
    "externalEventTitleTemplate",
    "externalEventDescriptionTemplate",
    "externalEventAttendeesTemplate",
];

/// `update`: sets the patch columns (only [`UPDATE_COLUMNS`]; `updatedAt` = now) on the
/// workspace's live calendar; the updated row, or `None`.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<AppointmentCalendar>> {
    if let Some((col, _)) = values.iter().find(|(c, _)| !UPDATE_COLUMNS.contains(c)) {
        return Err(Error::msg(format!(
            "appointment calendar update cannot set {col}"
        )));
    }
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"AppointmentCalendar\" SET ");
    q.append(set_clause(AppointmentCalendar::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"deletedAt\" IS NULL RETURNING ");
    q.push(&AppointmentCalendar::columns_sql(None));
    q.model(conn)
}

/// `softDeleteMany`: sets `deletedAt` on the workspace's live calendars of `ids`; their ids.
pub fn soft_delete_many(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "UPDATE \"AppointmentCalendar\" SET \"deletedAt\" = ?3, \"updatedAt\" = ?3 WHERE \"workspaceId\" = ?1 \
         AND \"id\" IN (SELECT value FROM json_each(?2)) AND \"deletedAt\" IS NULL RETURNING \"id\"",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, id_list(ids), now_ms()],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// One wanted slot of [`replace_availability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvailabilityInput {
    pub weekday: i64,
    pub start_minute: i64,
    pub end_minute: i64,
}

/// `replaceAvailability`: deletes the calendar's slots and inserts the given ones.
pub fn replace_availability(
    conn: &Connection,
    calendar_id: i64,
    availability: &[AvailabilityInput],
) -> Result<Vec<AppointmentCalendarAvailability>> {
    conn.execute(
        "DELETE FROM \"AppointmentCalendarAvailability\" WHERE \"calendarId\" = ?1",
        [calendar_id],
    )?;
    let mut out = Vec::with_capacity(availability.len());
    for a in availability {
        let rows = super::returning::<AppointmentCalendarAvailability>(
            conn,
            "INSERT INTO \"AppointmentCalendarAvailability\" (\"id\", \"calendarId\", \"weekday\", \"startMinute\", \"endMinute\") VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                &new_id(),
                &calendar_id,
                &a.weekday,
                &a.start_minute,
                &a.end_minute,
            ],
        )?;
        out.extend(rows);
    }
    Ok(out)
}
