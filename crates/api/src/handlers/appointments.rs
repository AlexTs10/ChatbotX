//! The appointment public routes: `appointmentCalendars.*`
//! (`features/appointment-calendars/api/public.ts`),
//! `appointmentExternalCalendars.*` (`features/external-calendars/api/public.ts`),
//! `appointmentReminders.list` (`features/appointment-management/api/public.ts`)
//! and `appointments.*` (`features/appointments/api/public.ts`).

use chatbotx_business::appointment::booking::{self, BookInput, Deps};
use chatbotx_business::appointment::calendar::{self, UpdateCalendar};
use chatbotx_business::appointment::external::{self, ExternalCalendars};
use chatbotx_business::conversation_list as list;
use chatbotx_business::message::{self, OutgoingInput};
use chatbotx_business::message_actions as actions;
use chatbotx_business::tx;
use chatbotx_db::repo::appointment::{AppointmentListInput, DispatchListInput, list_dispatches};
use chatbotx_db::repo::appointment_calendar::{
    AvailabilityInput, CalendarListInput, ReminderInput,
};
use chatbotx_db::repo::models::{Appointment, AppointmentCalendar, AppointmentReminderDispatch};
use chatbotx_db::repo::partials::appointment::parse_appointment_schedule_window_config;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Principal, Reply, business_error, date,
    id, json, no_content, opt_date, opt_id,
};
use crate::auth::token_has_any_scope;
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "appointmentCalendars.list",
    "appointmentCalendars.create",
    "appointmentCalendars.get",
    "appointmentCalendars.update",
    "appointmentCalendars.delete",
    "appointmentCalendars.setActive",
    "appointmentCalendars.duplicate",
    "appointmentCalendars.getAvailability",
    "appointmentExternalCalendars.list",
    "appointmentExternalCalendars.disconnect",
    "appointmentReminders.list",
    "appointments.list",
    "appointments.book",
    "appointments.get",
    "appointments.delete",
    "appointments.createBookingLink",
    "appointments.cancel",
];

const IDEMPOTENCY: [(&str, u16); 3] = [
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnBookingAppointment`.
const BOOKING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("invalidAppointmentConversation", 400),
    ("slotUnavailable", 409),
    ("appointmentAvailabilityChanged", 409),
    ("appointmentAlreadyScheduled", 409),
    ("appointmentNotCancellable", 409),
    ("appointmentDeleteBlocked", 409),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
/// `possibleErrorsOnCreatingBookingLink`.
const BOOKING_LINK: ErrorMap = &[
    ("notFound", 404),
    ("appointmentCalendarInactive", 409),
    ("businessError", 400),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
/// `possibleErrorsOnCreatingAppointmentCalendar`.
const CREATING_CALENDAR: ErrorMap = &[
    ("businessError", 400),
    ("nameAlreadyExists", 409),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
/// `possibleErrorsOnMutatingAppointmentCalendar`.
const MUTATING_CALENDAR: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("nameAlreadyExists", 409),
    ("duplicateReminder", 409),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
/// `possibleErrorsOnDisconnectingExternalCalendar`.
const DISCONNECTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("connectionInUse", 409),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
const DURATIONS: &[i64] = &[5, 10, 15, 20, 25, 30, 45, 60];
const BUFFERS: &[i64] = &[5, 10, 15, 30, 45, 60];
const NONE_SELECTION: &str = "__none__";

/// The external calendar calls of the state (`ENCRYPTION_KEY`).
pub(crate) fn externals(state: &ApiState) -> ExternalCalendars {
    ExternalCalendars {
        encryption: state.encryption.clone(),
        ..Default::default()
    }
}

pub(crate) fn deps<'a>(state: &'a ApiState, external: &'a ExternalCalendars) -> Deps<'a> {
    Deps {
        db: &state.db,
        jobs: state.jobs.as_deref(),
        external,
    }
}

fn public_url(app_url: &str, slug: &str) -> String {
    url::Url::parse(app_url)
        .and_then(|base| base.join(&format!("/booking/{slug}")))
        .map_or_else(|_| format!("{app_url}/booking/{slug}"), |u| u.to_string())
}

/// `appointmentCalendarPublicResource` (+ `publicUrl`).
pub(crate) fn calendar_json(c: &AppointmentCalendar, app_url: &str) -> Map<String, Value> {
    let mut config = match &c.schedule_window_config.0 {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    // The stored jsonb as is: a new calendar's `{}` stays `{}`.
    let config = if config.is_empty() {
        Value::Object(config)
    } else {
        config.insert("scheduleWindowType".into(), json!(c.schedule_window_type));
        parse_appointment_schedule_window_config(&Value::Object(config.clone()))
            .ok()
            .and_then(|parsed| serde_json::to_value(parsed).ok())
            .unwrap_or(Value::Object(config))
    };
    let Value::Object(out) = json!({
        "id": id(c.id),
        "name": c.name,
        "description": c.description,
        "active": c.active,
        "timezone": c.timezone,
        "durationMinutes": c.duration_minutes,
        "bufferAfterMinutes": c.buffer_after_minutes,
        "locationType": c.location_type,
        "locationDetail": c.location_detail,
        "scheduleWindowType": c.schedule_window_type,
        "scheduleWindowConfig": config,
        "maxAppointmentsPerUser": c.max_appointments_per_user,
        "dailyLimitEnabled": c.daily_limit_enabled,
        "maxPerDay": c.max_per_day,
        "allowGroupMeeting": c.allow_group_meeting,
        "maxPerSlot": c.max_per_slot,
        "confirmationMessage": c.confirmation_message,
        "confirmationFlowId": opt_id(c.confirmation_flow_id),
        "cancellationFlowId": opt_id(c.cancellation_flow_id),
        "externalConnectionId": opt_id(c.external_connection_id),
        "publicLinkSlug": c.public_link_slug,
        "createdAt": date(c.created_at),
        "updatedAt": date(c.updated_at),
        "publicUrl": public_url(app_url, &c.public_link_slug),
    }) else {
        unreachable!("an object literal")
    };
    out
}

/// `appointmentPublicResource`.
pub(crate) fn appointment_json(a: &Appointment) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": id(a.id),
        "calendarId": id(a.calendar_id),
        "contactId": id(a.contact_id),
        "conversationId": opt_id(a.conversation_id),
        "startAt": date(a.start_at),
        "endAt": date(a.end_at),
        "inviteeTimezone": a.invitee_timezone,
        "status": a.status,
        "locationType": a.location_type,
        "locationDetail": a.location_detail,
        "externalEventId": a.external_event_id,
        "cancelledAt": opt_date(a.cancelled_at),
        "createdAt": date(a.created_at),
        "updatedAt": date(a.updated_at),
    }) else {
        unreachable!("an object literal")
    };
    out
}

fn dispatch_json(d: &AppointmentReminderDispatch) -> Value {
    json!({
        "id": id(d.id),
        "appointmentId": id(d.appointment_id),
        "reminderConfigId": id(d.reminder_config_id),
        "contactInboxId": opt_id(d.contact_inbox_id),
        "sendAt": date(d.send_at),
        "status": d.status,
        "jobId": d.job_id,
        "sentAt": opt_date(d.sent_at),
        "cancelledAt": opt_date(d.cancelled_at),
        "failedReason": d.failed_reason,
        "createdAt": date(d.created_at),
        "updatedAt": date(d.updated_at),
    })
}

/// `publicListRequest`: `(page, perPage)` (defaults 1 / 50).
fn paging(input: &mut Input) -> (i64, i64) {
    let page = input.opt_coerce_int(
        "page",
        Num {
            int: true,
            min: Some(1.0),
            max: None,
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    (page.unwrap_or(1), per_page.unwrap_or(50))
}

/// `z.coerce.date()` (`new Date(value)` must be valid).
fn coerced_date(input: &mut Input, name: &str) -> Option<i64> {
    let parsed = match input.get(name) {
        Some(Value::String(text)) => chatbotx_core::js::parse_date(text),
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64),
        _ => None,
    };
    if parsed.is_none() {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

/// A `z.preprocess("" | null → null, z.coerce.number().int().min(1).nullable())` key.
fn nullable_positive(input: &mut Input, name: &str) -> Option<i64> {
    match input.get(name).cloned() {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(value) => input
            .coerce_number_at(
                &key(name),
                Some(&value),
                Num {
                    int: true,
                    min: Some(1.0),
                    max: None,
                },
                true,
            )
            .map(|n| n as i64),
    }
}

/// `optionalFlowIdField`: `"__none__"` / null / absent = `None`.
fn optional_ref(input: &mut Input, name: &str) -> Option<i64> {
    match input.get(name).cloned() {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text == NONE_SELECTION => None,
        Some(value) => input.id_at(&key(name), Some(&value), true),
    }
}

/// `optionalExternalEventTemplate(max)`: blank = `None`.
fn template(input: &mut Input, name: &str, max: usize) -> Option<String> {
    match input.get(name).cloned() {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.trim().is_empty() => None,
        Some(value) => input.string_at(&key(name), Some(&value), Str::max(max), true),
    }
}

/// `updateAppointmentCalendarRequest` (with its `superRefine`).
pub(crate) fn update_request(input: &mut Input, workspace_id: i64, id: i64) -> UpdateCalendar {
    let name = input.string("name", Str::trimmed(1, 255));
    let description = input
        .nullish_string(
            "description",
            Str {
                trim: true,
                ..Str::max(2000)
            },
        )
        .flatten();
    let active = input.bool("active");
    let timezone = input.string(
        "timezone",
        Str {
            trim: true,
            min: Some(1),
            ..Str::default()
        },
    );
    let duration = {
        let value = input.get("durationMinutes").cloned();
        input
            .coerce_number_at(
                &key("durationMinutes"),
                value.as_ref(),
                Num {
                    int: true,
                    ..Num::default()
                },
                true,
            )
            .map(|n| n as i64)
    };
    if let Some(d) = duration
        && !DURATIONS.contains(&d)
    {
        input.issue(&key("durationMinutes"), "custom", "Invalid duration");
    }
    let buffer = match input.get("bufferAfterMinutes").cloned() {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() || text == NONE_SELECTION => None,
        Some(value) => {
            let n = input
                .coerce_number_at(
                    &key("bufferAfterMinutes"),
                    Some(&value),
                    Num {
                        int: true,
                        ..Num::default()
                    },
                    true,
                )
                .map(|n| n as i64);
            if let Some(b) = n
                && !BUFFERS.contains(&b)
            {
                input.issue(&key("bufferAfterMinutes"), "custom", "Invalid buffer");
            }
            n
        }
    };
    let location_type = {
        let value = input.get("locationType").cloned();
        input.enum_at(
            &key("locationType"),
            value.as_ref(),
            &["inPerson", "phoneCall", "onlineMeeting"],
            true,
        )
    };
    let location_detail = input
        .nullish_string(
            "locationDetail",
            Str {
                trim: true,
                ..Str::max(500)
            },
        )
        .flatten();
    let (window_type, window_config) = match input.get("scheduleWindowConfig").cloned() {
        Some(value) => match parse_appointment_schedule_window_config(&value) {
            Ok(parsed) => {
                let mut config = serde_json::to_value(&parsed).unwrap_or(Value::Null);
                let kind = config["scheduleWindowType"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if let Some(obj) = config.as_object_mut() {
                    obj.remove("scheduleWindowType");
                }
                (kind, config)
            }
            Err(error) => {
                // `field: message` names the failing key of the config.
                let message = error.message();
                let (path, text) = match message.split_once(": ") {
                    Some((field, text))
                        if !field.is_empty() && field.bytes().all(|b| b.is_ascii_alphabetic()) =>
                    {
                        (child(&key("scheduleWindowConfig"), field), text.to_string())
                    }
                    _ => (key("scheduleWindowConfig"), message.clone()),
                };
                input.issue(&path, "custom", &text);
                (String::new(), Value::Null)
            }
        },
        None => {
            input.issue(
                &key("scheduleWindowConfig"),
                "invalid_union",
                "Invalid input",
            );
            (String::new(), Value::Null)
        }
    };
    let max_per_user = nullable_positive(input, "maxAppointmentsPerUser");
    let daily_limit_enabled = input.bool("dailyLimitEnabled");
    let max_per_day = nullable_positive(input, "maxPerDay");
    let allow_group_meeting = input.bool("allowGroupMeeting");
    let max_per_slot = nullable_positive(input, "maxPerSlot");
    let confirmation_message = input
        .nullish_string(
            "confirmationMessage",
            Str {
                trim: true,
                ..Str::max(2000)
            },
        )
        .flatten();
    let confirmation_flow_id = optional_ref(input, "confirmationFlowId");
    let cancellation_flow_id = optional_ref(input, "cancellationFlowId");
    let external_connection_id = optional_ref(input, "externalConnectionId");
    let title = template(input, "externalEventTitleTemplate", 1024);
    let description_template = template(input, "externalEventDescriptionTemplate", 8192);
    let attendees = template(input, "externalEventAttendeesTemplate", 8192);
    let availability = availability_at(input);
    let reminders = reminders_at(input);
    if daily_limit_enabled == Some(true) && max_per_day.is_none() {
        input.issue(
            &key("maxPerDay"),
            "custom",
            "Max per day is required when daily limit is enabled",
        );
    }
    if allow_group_meeting == Some(true) && max_per_slot.is_none() {
        input.issue(
            &key("maxPerSlot"),
            "custom",
            "Max per slot is required when group meeting is allowed",
        );
    }
    UpdateCalendar {
        workspace_id,
        id,
        name: name.unwrap_or_default(),
        description,
        active: active.unwrap_or_default(),
        timezone: timezone.unwrap_or_default(),
        duration_minutes: duration.unwrap_or(30),
        buffer_after_minutes: buffer,
        location_type: location_type.unwrap_or_default(),
        location_detail,
        schedule_window_type: window_type,
        schedule_window_config: window_config,
        max_appointments_per_user: max_per_user,
        daily_limit_enabled: daily_limit_enabled.unwrap_or_default(),
        max_per_day,
        allow_group_meeting: allow_group_meeting.unwrap_or_default(),
        max_per_slot,
        confirmation_message,
        confirmation_flow_id,
        cancellation_flow_id,
        external_connection_id,
        external_event_title_template: title,
        external_event_description_template: description_template,
        external_event_attendees_template: attendees,
        availability,
        reminders,
    }
}

fn array_at(input: &mut Input, name: &str, max: usize) -> Vec<Value> {
    match input.get(name).cloned() {
        Some(Value::Array(items)) => {
            if items.len() > max {
                input.issue(
                    &key(name),
                    "too_big",
                    &format!("Too big: expected array to have <={max} items"),
                );
            }
            items
        }
        other => {
            input.issue(
                &key(name),
                "invalid_type",
                &format!(
                    "Invalid input: expected array, received {}",
                    super::input::type_name(other.as_ref())
                ),
            );
            Vec::new()
        }
    }
}

fn availability_at(input: &mut Input) -> Vec<AvailabilityInput> {
    let items = array_at(input, "availability", 70);
    let mut out = Vec::new();
    let mut per_day: std::collections::BTreeMap<i64, (usize, usize)> = Default::default();
    for (index, item) in items.iter().enumerate() {
        let at = vec![Seg::Key("availability".into()), Seg::Index(index)];
        let field = |name: &str| item.get(name);
        let weekday = input.number_at(
            &child(&at, "weekday"),
            field("weekday"),
            Num::int(0.0, 6.0),
            true,
        );
        let start = input.number_at(
            &child(&at, "startMinute"),
            field("startMinute"),
            Num::int(0.0, 1425.0),
            true,
        );
        if let Some(s) = start
            && s % 15.0 != 0.0
        {
            input.issue(
                &child(&at, "startMinute"),
                "not_multiple_of",
                "Too small: expected number to be a multiple of 15",
            );
        }
        let end = input.number_at(
            &child(&at, "endMinute"),
            field("endMinute"),
            Num::int(15.0, 1439.0),
            true,
        );
        if let Some(e) = end
            && e != 1439.0
            && e % 15.0 != 0.0
        {
            input.issue(
                &child(&at, "endMinute"),
                "custom",
                "End time must be a 15-minute step or 23:59",
            );
        }
        if let (Some(weekday), Some(start), Some(end)) = (weekday, start, end) {
            if end <= start {
                input.issue(
                    &child(&at, "endMinute"),
                    "custom",
                    "End time must be after start time",
                );
            }
            let entry = per_day.entry(weekday as i64).or_insert((index, 0));
            entry.1 += 1;
            out.push(AvailabilityInput {
                weekday: weekday as i64,
                start_minute: start as i64,
                end_minute: end as i64,
            });
        }
    }
    for (first, total) in per_day.values() {
        if *total > 10 {
            input.issue(
                &[Seg::Key("availability".into()), Seg::Index(*first)],
                "custom",
                "A maximum of 10 intervals per day is allowed",
            );
        }
    }
    out
}

fn reminders_at(input: &mut Input) -> Vec<ReminderInput> {
    let items = array_at(input, "reminders", 50);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let at = vec![Seg::Key("reminders".into()), Seg::Index(index)];
        let flow_id = input.id_at(&child(&at, "flowId"), item.get("flowId"), true);
        let value = input
            .coerce_number_at(
                &child(&at, "timingValue"),
                item.get("timingValue"),
                Num {
                    int: true,
                    min: Some(1.0),
                    max: None,
                },
                true,
            )
            .map(|n| n as i64);
        let unit = input.enum_at(
            &child(&at, "timingUnit"),
            item.get("timingUnit"),
            &["minutes", "hours", "days"],
            true,
        );
        if let (Some(flow_id), Some(value), Some(unit)) = (flow_id, value, unit) {
            if !seen.insert(format!("{flow_id}:{value}:{unit}")) {
                input.issue(
                    &at,
                    "custom",
                    "Duplicate reminder: same flow and timing already exists",
                );
            }
            out.push(ReminderInput {
                flow_id,
                timing_value: value,
                timing_unit: unit,
            });
        }
    }
    out
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let external = externals(state);
    let deps = deps(state, &external);
    match route_id {
        "appointmentCalendars.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = paging(&mut input);
            let search = input.opt_string("search", Str::default());
            input.finish()?;
            let found = tx::read(db, move |conn| {
                Ok(chatbotx_db::repo::appointment_calendar::list(
                    conn,
                    &CalendarListInput {
                        workspace_id: ws,
                        search,
                        page: Some(page),
                        per_page: Some(per_page),
                        sort: None,
                    },
                )?)
            })
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = found
                .data
                .iter()
                .map(|c| Value::Object(calendar_json(c, &state.app_url)))
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": found.page_count }),
            ))
        }
        "appointmentCalendars.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let calendar_id = input.id("id");
            input.finish()?;
            let found = calendar::get_for_edit(db, ws, calendar_id.unwrap_or(-1))
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(
                200,
                Value::Object(calendar_for_edit_json(&found, &state.app_url)),
            ))
        }
        "appointmentCalendars.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            input.finish()?;
            let created = calendar::create(db, ws, &name.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, CREATING_CALENDAR))?;
            Ok(json(201, json!({ "id": id(created) })))
        }
        "appointmentCalendars.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let calendar_id = input.id("id");
            let request = update_request(&mut input, ws, calendar_id.unwrap_or(-1));
            input.finish()?;
            calendar::update(db, state.jobs.as_deref(), request)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING_CALENDAR))
        }
        "appointmentCalendars.setActive" => {
            let mut input = Input::from_body(call.params, call.body);
            let calendar_id = input.id("id");
            let active = input.bool("active");
            input.finish()?;
            calendar::set_active(
                db,
                ws,
                calendar_id.unwrap_or(-1),
                active.unwrap_or_default(),
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, MUTATING))
        }
        "appointmentCalendars.duplicate" => {
            let mut input = Input::from_body(call.params, call.body);
            let calendar_id = input.id("id");
            input.finish()?;
            calendar::duplicate(db, ws, calendar_id.unwrap_or(-1))
                .await
                .map(|created| json(201, json!({ "id": id(created) })))
                .map_err(|e| business_error(e, MUTATING_CALENDAR))
        }
        "appointmentCalendars.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let calendar_id = input.id("id");
            input.finish()?;
            calendar::delete_many(db, ws, vec![calendar_id.unwrap_or(-1)])
                .await
                .map(|_| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "appointmentCalendars.getAvailability" => {
            let mut input = Input::from_query(call.params, call.query);
            let calendar_id = input.id("id");
            let start = coerced_date(&mut input, "startDate");
            let end = coerced_date(&mut input, "endDate");
            let contact_id = match input.get("contactId").cloned() {
                None => None,
                Some(value) => input.id_at(&key("contactId"), Some(&value), true),
            };
            input.finish()?;
            let (text, slots, _) = booking::check_availability(
                deps,
                ws,
                calendar_id.unwrap_or(-1),
                contact_id,
                start.unwrap_or_default(),
                end.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            let slots: Vec<Value> = slots
                .iter()
                .map(|s| json!({ "startAt": date(s.start_at), "endAt": date(s.end_at) }))
                .collect();
            Ok(json(200, json!({ "text": text, "slots": slots })))
        }
        "appointmentExternalCalendars.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let items = tx::read(db, move |conn| {
                external::list_with_connected_count(conn, ws)
            })
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let start = usize::try_from((page - 1) * per_page).unwrap_or(usize::MAX);
            let data: Vec<Value> = items
                .iter()
                .skip(start)
                .take(per_page as usize)
                .map(external_json)
                .collect();
            let page_count = ((items.len() as i64 + per_page - 1) / per_page).max(1);
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "appointmentExternalCalendars.disconnect" => {
            let mut input = Input::from_query(call.params, call.query);
            let integration_id = input.id("integrationId");
            input.finish()?;
            external::disconnect(db, &external, ws, integration_id.unwrap_or(-1))
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, DISCONNECTING))
        }
        "appointmentReminders.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = paging(&mut input);
            let status = {
                let value = input.get("status").cloned();
                input.enum_at(
                    &key("status"),
                    value.as_ref(),
                    &["pending", "sent", "cancelled", "failed"],
                    false,
                )
            };
            input.finish()?;
            let (rows, page_count, _) = tx::read(db, move |conn| {
                Ok(list_dispatches(
                    conn,
                    &DispatchListInput {
                        workspace_id: Some(ws),
                        status,
                        page: Some(page),
                        per_page: Some(per_page),
                        sort: None,
                    },
                )?)
            })
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows.iter().map(dispatch_json).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "appointments.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = paging(&mut input);
            let calendar_id = match input.get("calendarId").cloned() {
                None => None,
                Some(value) => input.id_at(&key("calendarId"), Some(&value), true),
            };
            let tab = {
                let value = input.get("tab").cloned();
                input.enum_at(&key("tab"), value.as_ref(), &["next", "past"], false)
            };
            let search = input.opt_string("search", Str::default());
            input.finish()?;
            let (items, page_count) = booking::list(
                deps,
                AppointmentListInput {
                    workspace_id: ws,
                    calendar_id,
                    search,
                    past: tab.as_deref() == Some("past"),
                    page: Some(page),
                    per_page: Some(per_page),
                    now_ms: chatbotx_db::repo::now_ms(),
                },
                &state.app_url,
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = items.iter().map(list_item_json).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "appointments.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let appointment_id = input.id("id");
            input.finish()?;
            booking::find_or_fail(db, ws, appointment_id.unwrap_or(-1))
                .await
                .map(|a| json(200, Value::Object(appointment_json(&a))))
                .map_err(|e| business_error(e, FINDING))
        }
        "appointments.book" => book(state, deps, call).await,
        "appointments.createBookingLink" => create_booking_link(state, deps, call, principal).await,
        "appointments.cancel" => {
            let mut input = Input::from_body(call.params, call.body);
            let appointment_id = input.id("id");
            input.finish()?;
            booking::cancel_by_id(deps, ws, appointment_id.unwrap_or(-1))
                .await
                .map(|a| json(200, Value::Object(appointment_json(&a))))
                .map_err(|e| business_error(e, BOOKING))
        }
        "appointments.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let appointment_id = input.id("id");
            input.finish()?;
            booking::delete_by_id(db, ws, appointment_id.unwrap_or(-1))
                .await
                .map(|_| no_content())
                .map_err(|e| business_error(e, BOOKING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `appointmentCalendarForEditPublicResource`.
pub(crate) fn calendar_for_edit_json(
    found: &chatbotx_db::repo::appointment_calendar::CalendarForEdit,
    app_url: &str,
) -> Map<String, Value> {
    let mut out = calendar_json(&found.calendar, app_url);
    out.insert(
        "availability".into(),
        found
            .availability
            .iter()
            .map(|a| {
                json!({
                    "id": id(a.id),
                    "weekday": a.weekday,
                    "startMinute": a.start_minute,
                    "endMinute": a.end_minute,
                    "createdAt": date(a.created_at),
                    "updatedAt": date(a.updated_at),
                })
            })
            .collect(),
    );
    out.insert(
        "reminders".into(),
        found
            .reminders
            .iter()
            .map(|r| {
                json!({
                    "id": id(r.id),
                    "flowId": id(r.flow_id),
                    "timingValue": r.timing_value,
                    "timingUnit": r.timing_unit,
                    "createdAt": date(r.created_at),
                    "updatedAt": date(r.updated_at),
                })
            })
            .collect(),
    );
    out
}

pub(crate) fn external_json(item: &external::ExternalCalendarItem) -> Value {
    json!({
        "id": id(item.id),
        "providerType": "googleCalendar",
        "label": item.label,
        "providerCalendarId": item.provider_calendar_id,
        "email": item.email,
        "connectedCount": item.connected_count,
        "createdAt": date(item.created_at),
        "updatedAt": date(item.updated_at),
    })
}

/// `appointmentListItemPublicResource`.
pub(crate) fn list_item_json(item: &booking::ListItem) -> Value {
    let mut out = appointment_json(&item.row.appointment);
    out.insert("calendarName".into(), json!(item.row.calendar_name));
    out.insert("scheduleUrl".into(), json!(item.schedule_url));
    out.insert("cancellable".into(), json!(item.cancellable));
    out.insert("deletable".into(), json!(item.deletable));
    Value::Object(out)
}

async fn book(state: &ApiState, deps: Deps<'_>, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let calendar_id = input.id("calendarId");
    let contact_id = input.id("contactId");
    let conversation_id = input.nullish_id("conversationId").flatten();
    let inbox_id = match input.get("inboxId").cloned() {
        None => None,
        Some(value) => input.id_at(&key("inboxId"), Some(&value), true),
    };
    let start_at = coerced_date(&mut input, "startAt");
    let invitee_timezone = input.opt_string("inviteeTimezone", Str::default());
    if inbox_id.is_some() && conversation_id.is_none() {
        input.issue(&key("inboxId"), "custom", "inboxId requires conversationId");
    }
    input.finish()?;
    let mut contact_inbox_id = None;
    if let Some(conversation_id) = conversation_id {
        let conversation = list::find_or_fail(&state.db, ws, conversation_id)
            .await
            .map_err(|e| business_error(e, BOOKING))?;
        if let Some(inbox_id) = inbox_id {
            let contact_inbox = actions::resolve_contact_inbox_for_conversation(
                &state.db,
                &conversation,
                Some(inbox_id),
            )
            .await
            .map_err(|e| business_error(e, BOOKING))?;
            contact_inbox_id = Some(contact_inbox.id);
        }
    }
    booking::book(
        deps,
        BookInput {
            workspace_id: ws,
            calendar_id: calendar_id.unwrap_or(-1),
            contact_id: contact_id.unwrap_or(-1),
            conversation_id,
            contact_inbox_id,
            start_at: start_at.unwrap_or_default(),
            invitee_timezone,
            metadata: None,
        },
    )
    .await
    .map(|a| json(201, Value::Object(appointment_json(&a))))
    .map_err(|e| business_error(e, BOOKING))
}

async fn create_booking_link(
    state: &ApiState,
    deps: Deps<'_>,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let conversation_id = input.id("conversationId");
    let calendar_id = input.id("calendarId");
    let inbox_id = match input.get("inboxId").cloned() {
        None => None,
        Some(value) => input.id_at(&key("inboxId"), Some(&value), true),
    };
    let send = input.opt_bool("send").unwrap_or(false);
    let _bypass = input.opt_bool("bypassThreadControlLock");
    input.finish()?;
    if send && !token_has_any_scope(principal.scopes.as_deref(), &["inbox"]) {
        return Err(OrpcError::new(
            "FORBIDDEN",
            None,
            Some("Token is not authorized for the 'inbox' scope"),
        ));
    }
    let conversation = list::find_or_fail(&state.db, ws, conversation_id.unwrap_or(-1))
        .await
        .map_err(|e| business_error(e, BOOKING_LINK))?;
    let contact_inbox =
        actions::resolve_contact_inbox_for_conversation(&state.db, &conversation, inbox_id)
            .await
            .map_err(|e| business_error(e, BOOKING_LINK))?;
    let link = booking::create_inbox_booking_link(
        deps,
        ws,
        &conversation,
        &contact_inbox,
        calendar_id.unwrap_or(-1),
        &state.app_url,
    )
    .await
    .map_err(|e| business_error(e, BOOKING_LINK))?;
    let mut message_id = Value::Null;
    if send {
        let outgoing = OutgoingInput {
            text: Some(link.text.clone()),
            inbox_id: Some(contact_inbox.inbox_id),
            content_attributes: Some(json!({
                "type": "template",
                "payload": { "templateType": "button", "buttons": [link.button] },
            })),
            quick_replies: Some(json!([link.button])),
            ..Default::default()
        };
        let sent =
            message::create_outgoing(&state.db, &conversation, &contact_inbox, outgoing, None)
                .await
                .map_err(|e| business_error(e, BOOKING_LINK))?;
        message_id = sent
            .message
            .as_ref()
            .map_or(Value::Null, |m| id(m.message.id));
        super::contact_messages::deliver(state, &conversation, sent).await;
    }
    Ok(json(
        201,
        json!({
            "url": link.url,
            "expiresAt": date(link.expires_at),
            "calendar": { "id": id(link.calendar.0), "name": link.calendar.1 },
            "messageId": message_id,
        }),
    ))
}
