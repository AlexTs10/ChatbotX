//! `appointment-calendar/service.ts` (`appointmentCalendarService`) and its
//! pure slot helpers. No template locks / delete guards (P3.6-D1 rule).

use std::collections::{BTreeMap, HashMap, HashSet};

use chatbotx_core::flow_config::steps::choose_channel::choose_channel_step_default_fn;
use chatbotx_core::flow_config::steps::open_website::open_website_step_default_fn;
use chatbotx_core::flow_config::steps::send_text::send_text_step_default_fn;
use chatbotx_core::utils::datetime::resolve_filter_timezone;
use chatbotx_core::utils::id::create_id;
use chatbotx_core::utils::tz::{DateArg, format_in_time_zone, from_zoned_time};
use chatbotx_db::Db;
use chatbotx_db::repo::appointment as appointments;
use chatbotx_db::repo::appointment_calendar::{
    self as repo, AvailabilityInput, CalendarForEdit, CreateCalendarInput, ReminderInput,
};
use chatbotx_db::repo::client::is_unique_violation_error;
use chatbotx_db::repo::models::AppointmentCalendar;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::appointment::{
    AppointmentScheduleWindowConfig, parse_appointment_schedule_window_config,
};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::{Value, json};

use super::external::ExternalCalendars;
use crate::copy_name::{
    CopyNameOptions, CopyNameStyle, copy_name_prefix, insert_with_free_copy_name,
};
use crate::errors::{BusinessError, Result};
use crate::flow::crud::{PublishedDefault, create_published_default};
use crate::tx;

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 1440 * MINUTE_MS;
const LISTING_BUSY_EVENTS_TIMEOUT_MS: u64 = 2500;
const BOOKING_BUSY_EVENTS_TIMEOUT_MS: u64 = 5000;
/// `CALENDAR_COPY_NAME`.
pub const CALENDAR_COPY_NAME: CopyNameOptions = CopyNameOptions {
    style: CopyNameStyle::Dash,
    max_length: 255,
};
/// `defaultAppointmentExternalEventTitleTemplate`.
pub const DEFAULT_EXTERNAL_EVENT_TITLE_TEMPLATE: &str = "Appointment: {{booking_calendar}}";
/// `defaultAppointmentExternalEventAttendeesTemplate`.
pub const DEFAULT_EXTERNAL_EVENT_ATTENDEES_TEMPLATE: &str = "{{email}}";
const DEFAULT_REMINDER_TIMINGS: [(i64, &str); 3] = [(10, "minutes"), (1, "hours"), (1, "days")];

/// A bookable slot (`AvailableSlot`), Unix ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub start_at: i64,
    pub end_at: i64,
}

/// `ExternalBusyInterval` / `MergedInterval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub start: i64,
    pub end: i64,
}

/// `mergeIntervals`: sort and merge overlapping / touching intervals.
pub fn merge_intervals(intervals: &[Interval]) -> Vec<Interval> {
    let mut sorted = intervals.to_vec();
    sorted.sort_by_key(|i| i.start);
    let mut merged: Vec<Interval> = Vec::new();
    for current in sorted {
        match merged.last_mut() {
            Some(last) if current.start <= last.end => last.end = last.end.max(current.end),
            _ => merged.push(current),
        }
    }
    merged
}

/// `sliceIntervalIntoSlots`: `(startMinute, endMinute)` stepping by duration + buffer.
pub fn slice_interval_into_slots(
    interval: Interval,
    duration_minutes: i64,
    buffer_after_minutes: i64,
) -> Vec<(i64, i64)> {
    let step = duration_minutes + buffer_after_minutes;
    let mut out = Vec::new();
    let mut start = interval.start;
    while start + duration_minutes <= interval.end {
        out.push((start, start + duration_minutes));
        if step <= 0 {
            break;
        }
        start += step;
    }
    out
}

/// `overlaps`.
pub fn overlaps(start: i64, end: i64, intervals: &[Interval]) -> bool {
    intervals.iter().any(|i| end > i.start && start < i.end)
}

/// `AvailabilityFingerprint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub external_connection_id: Option<i64>,
    pub timezone: String,
    pub updated_at: i64,
}

/// `buildAvailabilityFingerprint`.
pub fn fingerprint(calendar: &AppointmentCalendar) -> Fingerprint {
    Fingerprint {
        external_connection_id: calendar.external_connection_id,
        timezone: calendar.timezone.clone(),
        updated_at: calendar.updated_at,
    }
}

fn zoned(text: &str, timezone: &str) -> i64 {
    from_zoned_time(text, timezone).unwrap_or(0)
}

fn day_label(ms: i64, timezone: &str) -> String {
    format_in_time_zone(DateArg::Ms(ms), timezone, "yyyy-MM-dd").unwrap_or_default()
}

/// `resolveWindowBounds`: the requested window clamped to the calendar's
/// schedule window and minimum notice; `None` when empty.
pub fn resolve_window_bounds(
    config: &AppointmentScheduleWindowConfig,
    timezone: &str,
    requested_start: i64,
    requested_end: i64,
    now: i64,
) -> Option<(i64, i64)> {
    use AppointmentScheduleWindowConfig as W;
    let tz = resolve_filter_timezone(Some(timezone));
    let min_advance = match config {
        W::RollingDays {
            min_advance_days, ..
        }
        | W::DateRange {
            min_advance_days, ..
        }
        | W::SpecificDay {
            min_advance_days, ..
        }
        | W::AnyFutureDate { min_advance_days } => *min_advance_days,
    };
    let clamp_start = requested_start.max(now + min_advance * DAY_MS);
    let (start, end) = match config {
        W::RollingDays { rolling_days, .. } => {
            (clamp_start, requested_end.min(now + rolling_days * DAY_MS))
        }
        W::DateRange {
            start_date,
            end_date,
            ..
        } => (
            clamp_start.max(zoned(&format!("{start_date}T00:00:00.000"), &tz)),
            requested_end.min(zoned(&format!("{end_date}T23:59:59.999"), &tz)),
        ),
        W::SpecificDay { date, .. } => (
            clamp_start.max(zoned(&format!("{date}T00:00:00.000"), &tz)),
            requested_end.min(zoned(&format!("{date}T23:59:59.999"), &tz)),
        ),
        W::AnyFutureDate { .. } => (clamp_start, requested_end),
    };
    (start <= end).then_some((start, end))
}

/// `listZonedCalendarDays`: `(date, weekday 0 = Sunday)` from `start`'s to `end`'s local day.
pub fn list_zoned_calendar_days(start: i64, end: i64, timezone: &str) -> Vec<(String, i64)> {
    use chrono::{Datelike, NaiveDate};
    let tz = resolve_filter_timezone(Some(timezone));
    let parse = |label: String| NaiveDate::parse_from_str(&label, "%Y-%m-%d").ok();
    let (Some(mut cursor), Some(last)) = (parse(day_label(start, &tz)), parse(day_label(end, &tz)))
    else {
        return Vec::new();
    };
    let mut days = Vec::new();
    while cursor <= last {
        days.push((
            cursor.format("%Y-%m-%d").to_string(),
            i64::from(cursor.weekday().num_days_from_sunday()),
        ));
        let Some(next) = cursor.succ_opt() else { break };
        cursor = next;
    }
    days
}

fn hhmmss(total_minutes: i64) -> String {
    let m = total_minutes.clamp(0, 1439);
    format!("{:02}:{:02}:00", m / 60, m % 60)
}

/// The calendar's window config (`appointmentScheduleWindowConfigSchema.parse`).
pub fn window_config(calendar: &AppointmentCalendar) -> Result<AppointmentScheduleWindowConfig> {
    let mut value = match &calendar.schedule_window_config.0 {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({}),
    };
    value["scheduleWindowType"] = json!(calendar.schedule_window_type);
    parse_appointment_schedule_window_config(&value).map_err(BusinessError::Repo)
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Appointment calendar not found")
}

fn name_exists() -> BusinessError {
    BusinessError::exception("Calendar name already exists", "nameAlreadyExists", 409)
}

fn map_unique(error: chatbotx_db::repo::Error) -> BusinessError {
    if is_unique_violation_error(&error, None) {
        name_exists()
    } else {
        BusinessError::Repo(error)
    }
}

/// `findByOrFail`.
pub fn find_or_fail(conn: &Connection, workspace_id: i64, id: i64) -> Result<AppointmentCalendar> {
    repo::find_by(conn, workspace_id, id, false)?.ok_or_else(not_found)
}

/// `getForEdit`.
pub async fn get_for_edit(db: &Db, workspace_id: i64, id: i64) -> Result<CalendarForEdit> {
    tx::read(db, move |conn| {
        repo::get_for_edit(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

fn default_booking_flow_node(text: &str, url: &str) -> (i64, Value) {
    let id = create_id();
    let mut website = open_website_step_default_fn();
    website["url"] = json!(url);
    let mut send = send_text_step_default_fn(&json!({ "text": text }));
    send["buttons"] = json!([{
        "id": create_id(),
        "label": "More Information",
        "buttonType": "openWebsite",
        "beforeStep": website,
        "steps": [],
    }]);
    let node = json!({
        "id": id,
        "position": { "x": 100, "y": 300 },
        "measured": { "width": 288, "height": 100 },
        "type": "sendMessage",
        "data": {
            "name": "Start",
            "isStartNode": true,
            "details": {
                "beforeStep": choose_channel_step_default_fn(&json!({})),
                "steps": [send],
                "quickReplies": [],
            },
        },
    });
    (id.parse().unwrap_or_default(), node)
}

/// `create`: the calendar, its default confirmation and reminder flows and
/// the three default reminders. Returns the calendar id.
pub async fn create(db: &Db, workspace_id: i64, name: &str) -> Result<i64> {
    let name = name.trim().to_string();
    tx::write(db, move |conn| {
        let row = repo::create(
            conn,
            &CreateCalendarInput {
                workspace_id,
                name,
                timezone: "UTC".into(),
                location_type: "onlineMeeting".into(),
                public_link_slug: create_id(),
                active: Some(true),
            },
        )
        .map_err(map_unique)?;
        let mut flow = |label: &str, text: &str| {
            let (start_node_id, node) = default_booking_flow_node(text, "{{booking_link}}");
            create_published_default(
                conn,
                PublishedDefault {
                    workspace_id,
                    name: format!("{label} - {}", row.name),
                    folder_id: None,
                    start_node_id,
                    nodes: vec![node],
                    edges: vec![],
                },
            )
            .map(|ids| ids.flow_id)
        };
        let confirmation = flow(
            "Booking confirmation",
            "Appointment Confirmation - {{booking_calendar}}\n\nDate: {{booking_date}}",
        )?;
        let reminder = flow(
            "Reminder",
            "Reminder - {{booking_calendar}}\n\nDate: {{booking_date}}",
        )?;
        repo::update(
            conn,
            workspace_id,
            row.id,
            &vec![("confirmationFlowId", Sql::Integer(confirmation))],
        )?;
        let reminders: Vec<ReminderInput> = DEFAULT_REMINDER_TIMINGS
            .iter()
            .map(|(value, unit)| ReminderInput {
                flow_id: reminder,
                timing_value: *value,
                timing_unit: (*unit).into(),
            })
            .collect();
        repo::replace_reminders(conn, row.id, &reminders)?;
        Ok(row.id)
    })
    .await
}

/// `setActive`.
pub async fn set_active(db: &Db, workspace_id: i64, id: i64, active: bool) -> Result<()> {
    tx::write(db, move |conn| {
        repo::update(
            conn,
            workspace_id,
            id,
            &vec![("active", Sql::Integer(i64::from(active)))],
        )?
        .map(|_| ())
        .ok_or_else(not_found)
    })
    .await
}

/// `rename`.
pub async fn rename(db: &Db, workspace_id: i64, id: i64, name: &str) -> Result<()> {
    let name = name.trim().to_string();
    tx::write(db, move |conn| {
        repo::update(conn, workspace_id, id, &vec![("name", Sql::Text(name))])
            .map_err(map_unique)?
            .map(|_| ())
            .ok_or_else(not_found)
    })
    .await
}

fn opt_text(value: &Option<String>) -> Sql {
    value.clone().map_or(Sql::Null, Sql::Text)
}

fn opt_int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

/// `duplicate`: an inactive copy (`<name> - Copy`) with the same schedule; its id.
pub async fn duplicate(db: &Db, workspace_id: i64, id: i64) -> Result<i64> {
    tx::write(db, move |conn| {
        let source = repo::get_for_edit(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let s = &source.calendar;
        let taken: HashSet<String> = repo::list_names_with_prefix(
            conn,
            workspace_id,
            &copy_name_prefix(&s.name, CALENDAR_COPY_NAME),
        )?
        .into_iter()
        .collect();
        // A failed INSERT rolls back only its own statement in SQLite.
        let created = insert_with_free_copy_name(
            &s.name,
            &taken,
            CALENDAR_COPY_NAME,
            None,
            |error: &chatbotx_db::repo::Error, constraint| {
                is_unique_violation_error(error, constraint)
            },
            |name| {
                repo::create(
                    conn,
                    &CreateCalendarInput {
                        workspace_id,
                        name: name.to_string(),
                        timezone: s.timezone.clone(),
                        location_type: s.location_type.clone(),
                        public_link_slug: create_id(),
                        active: None,
                    },
                )
            },
        )?
        .ok_or_else(name_exists)?;
        repo::update(
            conn,
            workspace_id,
            created.id,
            &vec![
                ("description", opt_text(&s.description)),
                ("durationMinutes", Sql::Integer(s.duration_minutes)),
                ("bufferAfterMinutes", opt_int(s.buffer_after_minutes)),
                ("locationDetail", opt_text(&s.location_detail)),
                (
                    "scheduleWindowType",
                    Sql::Text(s.schedule_window_type.clone()),
                ),
                (
                    "scheduleWindowConfig",
                    Sql::Text(s.schedule_window_config.0.to_string()),
                ),
                (
                    "maxAppointmentsPerUser",
                    opt_int(s.max_appointments_per_user),
                ),
                (
                    "dailyLimitEnabled",
                    Sql::Integer(i64::from(s.daily_limit_enabled)),
                ),
                ("maxPerDay", opt_int(s.max_per_day)),
                (
                    "allowGroupMeeting",
                    Sql::Integer(i64::from(s.allow_group_meeting)),
                ),
                ("maxPerSlot", opt_int(s.max_per_slot)),
                ("confirmationMessage", opt_text(&s.confirmation_message)),
                ("confirmationFlowId", opt_int(s.confirmation_flow_id)),
                ("cancellationFlowId", opt_int(s.cancellation_flow_id)),
                ("externalConnectionId", opt_int(s.external_connection_id)),
                (
                    "externalEventTitleTemplate",
                    opt_text(&s.external_event_title_template),
                ),
                (
                    "externalEventDescriptionTemplate",
                    opt_text(&s.external_event_description_template),
                ),
                (
                    "externalEventAttendeesTemplate",
                    opt_text(&s.external_event_attendees_template),
                ),
                ("active", Sql::Integer(0)),
            ],
        )?;
        let availability: Vec<AvailabilityInput> = source
            .availability
            .iter()
            .map(|a| AvailabilityInput {
                weekday: a.weekday,
                start_minute: a.start_minute,
                end_minute: a.end_minute,
            })
            .collect();
        repo::replace_availability(conn, created.id, &availability)?;
        let reminders: Vec<ReminderInput> = source
            .reminders
            .iter()
            .map(|r| ReminderInput {
                flow_id: r.flow_id,
                timing_value: r.timing_value,
                timing_unit: r.timing_unit.clone(),
            })
            .collect();
        repo::replace_reminders(conn, created.id, &reminders)?;
        Ok(created.id)
    })
    .await
}

/// `deleteMany` (soft delete); the deleted ids.
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    tx::write(db, move |conn| {
        Ok(repo::soft_delete_many(conn, workspace_id, &ids)?)
    })
    .await
}

/// `UpdateAppointmentCalendarInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateCalendar {
    pub workspace_id: i64,
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub active: bool,
    pub timezone: String,
    pub duration_minutes: i64,
    pub buffer_after_minutes: Option<i64>,
    pub location_type: String,
    pub location_detail: Option<String>,
    pub schedule_window_type: String,
    /// The config object (without `scheduleWindowType`).
    pub schedule_window_config: Value,
    pub max_appointments_per_user: Option<i64>,
    pub daily_limit_enabled: bool,
    pub max_per_day: Option<i64>,
    pub allow_group_meeting: bool,
    pub max_per_slot: Option<i64>,
    pub confirmation_message: Option<String>,
    pub confirmation_flow_id: Option<i64>,
    pub cancellation_flow_id: Option<i64>,
    pub external_connection_id: Option<i64>,
    pub external_event_title_template: Option<String>,
    pub external_event_description_template: Option<String>,
    pub external_event_attendees_template: Option<String>,
    pub availability: Vec<AvailabilityInput>,
    pub reminders: Vec<ReminderInput>,
}

/// `update`: the calendar, its availability and reminders in one write; then
/// the future appointments' reminders are scheduled again (a failure there is
/// logged, as the original).
pub async fn update(
    db: &Db,
    jobs: Option<&dyn chatbotx_jobs::enqueue::Enqueuer>,
    input: UpdateCalendar,
) -> Result<()> {
    let (workspace_id, id) = (input.workspace_id, input.id);
    tx::write(db, move |conn| {
        repo::get_for_edit(conn, workspace_id, id)?.ok_or_else(not_found)?;
        if let Some(integration_id) = input.external_connection_id {
            super::external::google_connection(conn, workspace_id, integration_id)?;
        }
        let mut flows: Vec<i64> = [input.confirmation_flow_id, input.cancellation_flow_id]
            .into_iter()
            .flatten()
            .chain(input.reminders.iter().map(|r| r.flow_id))
            .collect();
        flows.sort_unstable();
        flows.dedup();
        for flow_id in flows {
            if !chatbotx_db::repo::flow::exists_in_workspace(conn, workspace_id, flow_id)? {
                return Err(BusinessError::not_found("Flow not found"));
            }
        }
        let connected = input.external_connection_id.is_some();
        let template = |value: &Option<String>, default: &str| match value {
            Some(text) => Sql::Text(text.clone()),
            None if connected => Sql::Text(default.into()),
            None => Sql::Null,
        };
        repo::update(
            conn,
            workspace_id,
            id,
            &vec![
                ("name", Sql::Text(input.name.trim().to_string())),
                ("description", opt_text(&input.description)),
                ("active", Sql::Integer(i64::from(input.active))),
                ("timezone", Sql::Text(input.timezone.clone())),
                ("durationMinutes", Sql::Integer(input.duration_minutes)),
                ("bufferAfterMinutes", opt_int(input.buffer_after_minutes)),
                ("locationType", Sql::Text(input.location_type.clone())),
                ("locationDetail", opt_text(&input.location_detail)),
                (
                    "scheduleWindowType",
                    Sql::Text(input.schedule_window_type.clone()),
                ),
                (
                    "scheduleWindowConfig",
                    Sql::Text(input.schedule_window_config.to_string()),
                ),
                (
                    "maxAppointmentsPerUser",
                    opt_int(input.max_appointments_per_user),
                ),
                (
                    "dailyLimitEnabled",
                    Sql::Integer(i64::from(input.daily_limit_enabled)),
                ),
                ("maxPerDay", opt_int(input.max_per_day)),
                (
                    "allowGroupMeeting",
                    Sql::Integer(i64::from(input.allow_group_meeting)),
                ),
                ("maxPerSlot", opt_int(input.max_per_slot)),
                ("confirmationMessage", opt_text(&input.confirmation_message)),
                ("confirmationFlowId", opt_int(input.confirmation_flow_id)),
                ("cancellationFlowId", opt_int(input.cancellation_flow_id)),
                (
                    "externalConnectionId",
                    opt_int(input.external_connection_id),
                ),
                (
                    "externalEventTitleTemplate",
                    template(
                        &input.external_event_title_template,
                        DEFAULT_EXTERNAL_EVENT_TITLE_TEMPLATE,
                    ),
                ),
                (
                    "externalEventDescriptionTemplate",
                    opt_text(&input.external_event_description_template),
                ),
                (
                    "externalEventAttendeesTemplate",
                    template(
                        &input.external_event_attendees_template,
                        DEFAULT_EXTERNAL_EVENT_ATTENDEES_TEMPLATE,
                    ),
                ),
            ],
        )
        .map_err(map_unique)?
        .ok_or_else(not_found)?;
        repo::replace_availability(conn, id, &input.availability)?;
        repo::replace_reminders(conn, id, &input.reminders).map_err(|error| {
            if is_unique_violation_error(&error, None) {
                BusinessError::exception(
                    "Duplicate reminder: same flow and timing already exists",
                    "duplicateReminder",
                    409,
                )
            } else {
                BusinessError::Repo(error)
            }
        })?;
        Ok(())
    })
    .await?;
    if let Err(error) =
        super::reminder::reschedule_future_for_calendar(db, jobs, workspace_id, id, now_ms()).await
    {
        tracing::warn!(
            workspace_id,
            calendar_id = id,
            err = %error.message(),
            "Failed to reschedule future appointment reminders after calendar update"
        );
    }
    Ok(())
}

/// `AvailabilityContext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailabilityContext {
    pub fingerprint: Fingerprint,
    pub busy: Vec<Interval>,
    pub listing_empty: bool,
}

/// `prepareAvailabilityContext`: the calendar's fingerprint and the external
/// busy intervals of the window. `throw_on_failure` = `failurePolicy: "throw"`.
pub async fn prepare_availability_context(
    db: &Db,
    external: &ExternalCalendars,
    workspace_id: i64,
    calendar_id: i64,
    start: i64,
    end: i64,
    throw_on_failure: bool,
) -> Result<AvailabilityContext> {
    let calendar = tx::read(db, move |conn| {
        find_or_fail(conn, workspace_id, calendar_id)
    })
    .await?;
    let mut context = AvailabilityContext {
        fingerprint: fingerprint(&calendar),
        busy: Vec::new(),
        listing_empty: true,
    };
    if !calendar.active {
        return Ok(context);
    }
    let tz = resolve_filter_timezone(Some(&calendar.timezone));
    let Some((from, to)) =
        resolve_window_bounds(&window_config(&calendar)?, &tz, start, end, now_ms())
    else {
        return Ok(context);
    };
    let Some(integration_id) = calendar.external_connection_id else {
        context.listing_empty = false;
        return Ok(context);
    };
    let lookup_end = if from < to {
        to
    } else {
        from + calendar.duration_minutes * MINUTE_MS
    };
    let timeout = if throw_on_failure {
        BOOKING_BUSY_EVENTS_TIMEOUT_MS
    } else {
        LISTING_BUSY_EVENTS_TIMEOUT_MS
    };
    match external
        .busy_intervals(
            db,
            workspace_id,
            integration_id,
            from,
            lookup_end,
            Some(&tz),
            timeout,
        )
        .await
    {
        Ok(busy) => {
            context.busy = busy;
            context.listing_empty = false;
        }
        Err(error) if throw_on_failure => return Err(error),
        Err(error) => tracing::warn!(
            workspace_id,
            calendar_id,
            integration_id,
            reason = "externalBusyEventsFailed",
            err = %error.message(),
            "Failed to fetch appointment external busy events"
        ),
    }
    Ok(context)
}

/// `resolveAvailableSlotsForListing`.
pub async fn resolve_available_slots_for_listing(
    db: &Db,
    external: &ExternalCalendars,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: Option<i64>,
    start: i64,
    end: i64,
) -> Result<Vec<Slot>> {
    let context =
        prepare_availability_context(db, external, workspace_id, calendar_id, start, end, false)
            .await?;
    if context.listing_empty {
        return Ok(Vec::new());
    }
    tx::read(db, move |conn| {
        generate_available_slots(
            conn,
            &SlotQuery {
                workspace_id,
                calendar_id,
                contact_id,
                start,
                end,
                busy: Some(context.busy),
                now: now_ms(),
            },
        )
    })
    .await
}

/// `hasExternalBusyConflictForSlot`: fails closed on a lookup error.
pub async fn has_external_busy_conflict(
    db: &Db,
    external: &ExternalCalendars,
    workspace_id: i64,
    external_connection_id: Option<i64>,
    start: i64,
    end: i64,
) -> bool {
    let Some(integration_id) = external_connection_id else {
        return false;
    };
    match external
        .busy_intervals(
            db,
            workspace_id,
            integration_id,
            start,
            end,
            None,
            BOOKING_BUSY_EVENTS_TIMEOUT_MS,
        )
        .await
    {
        Ok(busy) => overlaps(start, end, &busy),
        Err(error) => {
            tracing::warn!(
                workspace_id,
                integration_id,
                reason = "externalSlotRevalidationFailed",
                err = %error.message(),
                "External calendar re-check failed, failing closed"
            );
            true
        }
    }
}

/// `GenerateAvailableSlotsInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotQuery {
    pub workspace_id: i64,
    pub calendar_id: i64,
    pub contact_id: Option<i64>,
    pub start: i64,
    pub end: i64,
    pub busy: Option<Vec<Interval>>,
    pub now: i64,
}

/// `generateAvailableSlots`: the single slot generator (flow step, booking
/// pickers, booking check), sorted by start.
pub fn generate_available_slots(conn: &Connection, q: &SlotQuery) -> Result<Vec<Slot>> {
    let Some(calendar) = repo::find_by(conn, q.workspace_id, q.calendar_id, false)? else {
        return Ok(Vec::new());
    };
    if !calendar.active {
        return Ok(Vec::new());
    }
    let tz = resolve_filter_timezone(Some(&calendar.timezone));
    let Some((from, to)) =
        resolve_window_bounds(&window_config(&calendar)?, &tz, q.start, q.end, q.now)
    else {
        return Ok(Vec::new());
    };
    if let (Some(max), Some(contact_id)) = (calendar.max_appointments_per_user, q.contact_id) {
        let count = appointments::count_future_scheduled_for_contact(
            conn,
            q.workspace_id,
            q.calendar_id,
            contact_id,
            q.now,
        )?;
        if count >= max {
            return Ok(Vec::new());
        }
    }
    let availability = repo::get_for_edit(conn, q.workspace_id, q.calendar_id)?
        .map(|c| c.availability)
        .unwrap_or_default();
    let mut by_weekday: BTreeMap<i64, Vec<Interval>> = BTreeMap::new();
    for row in &availability {
        by_weekday.entry(row.weekday).or_default().push(Interval {
            start: row.start_minute,
            end: row.end_minute,
        });
    }
    let by_weekday: BTreeMap<i64, Vec<Interval>> = by_weekday
        .into_iter()
        .map(|(day, intervals)| (day, merge_intervals(&intervals)))
        .collect();
    let duration = calendar.duration_minutes;
    let buffer = calendar.buffer_after_minutes.unwrap_or(0);
    let capacity = if calendar.allow_group_meeting {
        calendar.max_per_slot.unwrap_or(i64::MAX)
    } else {
        1
    };
    let days = list_zoned_calendar_days(from, to, &tz);
    let daily_limit = calendar
        .max_per_day
        .filter(|_| calendar.daily_limit_enabled);
    let day_start = |date: &str| zoned(&format!("{date}T00:00:00.000"), &tz);
    let day_end = |date: &str| zoned(&format!("{date}T23:59:59.999"), &tz);
    let (count_from, count_to) = match (daily_limit, days.first(), days.last()) {
        (Some(_), Some(first), Some(last)) => (day_start(&first.0), day_end(&last.0)),
        _ => (from, to),
    };
    let mut booked: HashMap<i64, i64> = HashMap::new();
    for start_at in appointments::scheduled_start_ats(
        conn,
        q.workspace_id,
        q.calendar_id,
        count_from,
        count_to,
    )? {
        *booked.entry(start_at).or_default() += 1;
    }
    let mut slots = Vec::new();
    for (date, weekday) in &days {
        if let Some(max) = daily_limit {
            let (a, b) = (day_start(date), day_end(date));
            let day_count: i64 = booked
                .iter()
                .filter(|(at, _)| **at >= a && **at <= b)
                .map(|(_, n)| n)
                .sum();
            if day_count >= max {
                continue;
            }
        }
        for interval in by_weekday.get(weekday).into_iter().flatten() {
            for (start_minute, _) in slice_interval_into_slots(*interval, duration, buffer) {
                let start_at = zoned(&format!("{date}T{}", hhmmss(start_minute)), &tz);
                if start_at < from || start_at > to {
                    continue;
                }
                if booked.get(&start_at).copied().unwrap_or(0) >= capacity {
                    continue;
                }
                let end_at = start_at + duration * MINUTE_MS;
                if q.busy
                    .as_deref()
                    .is_some_and(|busy| overlaps(start_at, end_at, busy))
                {
                    continue;
                }
                slots.push(Slot { start_at, end_at });
            }
        }
    }
    slots.sort_by_key(|s| s.start_at);
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(start: i64, end: i64) -> Interval {
        Interval { start, end }
    }

    #[test]
    fn merges_overlapping_and_touching_intervals() {
        assert_eq!(
            merge_intervals(&[iv(600, 720), iv(540, 660), iv(720, 780), iv(900, 960)]),
            vec![iv(540, 780), iv(900, 960)]
        );
        assert!(merge_intervals(&[]).is_empty());
    }

    #[test]
    fn slices_with_buffer() {
        assert_eq!(
            slice_interval_into_slots(iv(540, 660), 30, 15),
            vec![(540, 570), (585, 615), (630, 660)]
        );
        assert!(slice_interval_into_slots(iv(540, 560), 30, 0).is_empty());
    }

    #[test]
    fn overlap_is_half_open() {
        assert!(overlaps(10, 20, &[iv(15, 30)]));
        assert!(!overlaps(10, 20, &[iv(20, 30)]));
        assert!(!overlaps(10, 20, &[iv(0, 10)]));
    }

    #[test]
    fn zoned_days_carry_sunday_zero_weekdays() {
        // 2026-11-15 is a Sunday.
        let start = from_zoned_time("2026-11-15T00:30:00.000", "Asia/Ho_Chi_Minh").unwrap();
        let end = from_zoned_time("2026-11-16T23:00:00.000", "Asia/Ho_Chi_Minh").unwrap();
        assert_eq!(
            list_zoned_calendar_days(start, end, "Asia/Ho_Chi_Minh"),
            vec![("2026-11-15".into(), 0), ("2026-11-16".into(), 1)]
        );
    }

    #[test]
    fn window_bounds_clamp_to_min_advance_and_rolling_days() {
        let now = 1_000 * DAY_MS;
        let config = AppointmentScheduleWindowConfig::RollingDays {
            rolling_days: 7,
            min_advance_days: 1,
        };
        assert_eq!(
            resolve_window_bounds(&config, "UTC", now, now + 30 * DAY_MS, now),
            Some((now + DAY_MS, now + 7 * DAY_MS))
        );
        assert_eq!(
            resolve_window_bounds(&config, "UTC", now + 8 * DAY_MS, now + 9 * DAY_MS, now),
            None
        );
    }
}
