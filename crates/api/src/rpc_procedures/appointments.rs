//! The appointment procedures (`appointmentCalendarsAPI.*`,
//! `appointmentsAPI.*`) and actions (`features/appointment-calendars`,
//! `appointment-management`, `external-calendars` actions). Most run the
//! ported public `/v1` routes ([`public`]): the same service as the builder.

use chatbotx_business::BusinessError;
use chatbotx_business::appointment::{booking, calendar};
use chatbotx_business::tx;
use serde_json::{Value, json};

use super::contacts::require_contact_access;
use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, schema_errors};
use crate::handlers::appointments::{deps, externals};
use crate::handlers::{date, id, opt_id};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "createAppointmentCalendarAction",
    "deleteAppointmentCalendarsAction",
    "duplicateAppointmentCalendarAction",
    "renameAppointmentCalendarAction",
    "updateAppointmentCalendarActiveAction",
    "updateAppointmentCalendarAction",
    "bookContactAppointmentAction",
    "cancelAppointmentAction",
    "deleteAppointmentAction",
    "createInboxBookingLinkAction",
    "disconnectGoogleCalendarAction",
];

/// `DEFAULT_SLOT_WINDOW_DAYS` (the availability range cap).
const SLOT_WINDOW_DAYS: i64 = 90;

/// `z.coerce.date()` of a procedure input.
fn date_of(input: &Value, key: &str) -> Result<i64, OrpcError> {
    match input.get(key) {
        Some(Value::String(text)) => chatbotx_core::js::parse_date(text).ok_or_else(invalid),
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "appointmentCalendarsAPI.listAppointmentCalendarsForFlowAPI" => {
            list_for_flow(ctx, input).await
        }
        "appointmentCalendarsAPI.getContactAvailabilityAPI" => availability(ctx, input).await,
        "appointmentsAPI.listContactAppointmentsAPI" => contact_appointments(ctx, input).await,
        _ => return None,
    })
}

async fn list_for_flow(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let keyword = In(input).opt_str("keyword")?;
    let ws = ctx.workspace_id;
    let rows = tx::read(&ctx.api.db, move |conn| {
        Ok(chatbotx_db::repo::appointment_calendar::list_for_flow(
            conn,
            ws,
            keyword.as_deref(),
        )?)
    })
    .await
    .map_err(fail)?;
    Ok(rows
        .iter()
        .map(|c| json!({ "id": id(c.id), "name": c.name, "active": c.active }))
        .collect())
}

async fn availability(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let calendar_id = In(input).id("calendarId")?;
    let contact_id = In(input).id("contactId")?;
    let (start, end) = (date_of(input, "startDate")?, date_of(input, "endDate")?);
    let span = end - start;
    if span < 0 || span > SLOT_WINDOW_DAYS * 86_400_000 {
        return Err(invalid());
    }
    require_contact_access(ctx, contact_id).await?;
    let external = externals(ctx.api);
    let (_, slots, timezone) = booking::check_availability(
        deps(ctx.api, &external),
        ctx.workspace_id,
        calendar_id,
        Some(contact_id),
        start,
        end,
    )
    .await
    .map_err(fail)?;
    let slots: Vec<Value> = slots
        .iter()
        .map(|s| json!({ "startAt": date(s.start_at), "endAt": date(s.end_at) }))
        .collect();
    Ok(json!({ "timezone": timezone, "slots": slots }))
}

async fn contact_appointments(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let contact_id = In(input).id("contactId")?;
    let scope = ctx.require_contact_scope()?;
    let external = externals(ctx.api);
    let rows = booking::list_contact_appointments(
        deps(ctx.api, &external),
        ctx.workspace_id,
        contact_id,
        None,
        scope.restrict_to_assigned_user_id,
        &ctx.api.app_url,
    )
    .await
    .map_err(fail)?;
    Ok(rows
        .iter()
        .map(|(row, url)| {
            let a = &row.appointment;
            json!({
                "id": id(a.id),
                "workspaceId": id(a.workspace_id),
                "calendarId": id(a.calendar_id),
                "contactId": id(a.contact_id),
                "conversationId": opt_id(a.conversation_id),
                "calendarName": row.calendar_name,
                "startAt": date(a.start_at),
                "endAt": date(a.end_at),
                "inviteeTimezone": a.invitee_timezone,
                "status": a.status,
                "scheduleUrl": url,
            })
        })
        .collect())
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The id bound at `index`.
fn bound(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let done =
        |result: chatbotx_business::Result<()>| result.map(|()| Value::Null).map_err(action_fail);
    let route = |route: &'static str, id: i64, body: Option<&Value>| async move {
        public(ctx, route, &[("id", id.to_string())], None, body).await
    };
    match name {
        "createAppointmentCalendarAction" => {
            let name = In(arg(args, 1)).str("name", true, 1, 255)?;
            calendar::create(db, ws, &name)
                .await
                .map(|created| json!({ "id": id(created) }))
                .map_err(action_fail)
        }
        "deleteAppointmentCalendarsAction" => {
            let ids = In(arg(args, 1)).ids("ids")?;
            calendar::delete_many(db, ws, ids)
                .await
                .map(|_| Value::Null)
                .map_err(action_fail)
        }
        "duplicateAppointmentCalendarAction" => calendar::duplicate(db, ws, bound(args, 1)?)
            .await
            .map(|created| json!({ "id": id(created) }))
            .map_err(action_fail),
        "renameAppointmentCalendarAction" => {
            let calendar_id = bound(args, 1)?;
            let name = In(arg(args, 2)).str("name", true, 1, 255)?;
            done(calendar::rename(db, ws, calendar_id, &name).await)
        }
        "updateAppointmentCalendarActiveAction" => {
            let calendar_id = bound(args, 1)?;
            let active = In(arg(args, 2)).opt_bool("active")?.ok_or_else(invalid)?;
            done(calendar::set_active(db, ws, calendar_id, active).await)
        }
        "updateAppointmentCalendarAction" => route(
            "appointmentCalendars.update",
            bound(args, 1)?,
            Some(arg(args, 2)),
        )
        .await
        .map(|_| Value::Null)
        .map_err(schema_errors),
        "cancelAppointmentAction" => {
            let appointment_id = In(arg(args, 1)).id("appointmentId")?;
            route("appointments.cancel", appointment_id, Some(&json!({})))
                .await
                .map(|_| Value::Null)
        }
        "deleteAppointmentAction" => {
            let appointment_id = In(arg(args, 1)).id("appointmentId")?;
            route("appointments.delete", appointment_id, None)
                .await
                .map(|_| Value::Null)
        }
        "disconnectGoogleCalendarAction" => {
            let integration_id = bound(args, 1)?;
            let params = [("integrationId", integration_id.to_string())];
            public(
                ctx,
                "appointmentExternalCalendars.disconnect",
                &params,
                None,
                None,
            )
            .await
            .map(|_| Value::Null)
        }
        "createInboxBookingLinkAction" => {
            let conversation_id = bound(args, 1)?;
            let input = In(arg(args, 2));
            let mut body = json!({
                "calendarId": input.id("calendarId")?.to_string(),
                "send": input.opt_bool("send")?.ok_or_else(invalid)?,
            });
            if let Some(inbox_id) = input.opt_id("inboxId")? {
                body["inboxId"] = json!(inbox_id.to_string());
            }
            if let Some(bypass) = input.opt_bool("bypassThreadControlLock")? {
                body["bypassThreadControlLock"] = json!(bypass);
            }
            let params = [("conversationId", conversation_id.to_string())];
            let link = public(
                ctx,
                "appointments.createBookingLink",
                &params,
                None,
                Some(&body),
            )
            .await?;
            Ok(json!({
                "url": link["url"],
                "expiresAt": link["expiresAt"],
                "messageId": link["messageId"],
            }))
        }
        "bookContactAppointmentAction" => book(ctx, arg(args, 1)).await,
        _ => Err(fail(BusinessError::not_found("Action not found"))),
    }
}

/// `bookContactAppointmentAction`: a stale slot, an already held start or a
/// changed availability is a result flag, not an error.
async fn book(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id;
    let calendar_id = In(input).id("calendarId")?;
    let contact_id = In(input).id("contactId")?;
    let conversation_id = In(input).opt_id("conversationId")?;
    let start_at = match In(input).get("startAt") {
        Some(Value::String(text)) if text.contains('T') => {
            chatbotx_core::js::parse_date(text).ok_or_else(invalid)?
        }
        _ => return Err(invalid()),
    };
    let invitee_timezone = In(input).str("inviteeTimezone", false, 1, 255)?;
    if !chatbotx_core::utils::tz::is_intl_time_zone(&invitee_timezone) {
        return Err(invalid());
    }
    require_contact_access(ctx, contact_id).await?;
    let mut contact_inbox_id = None;
    if let Some(conversation_id) = conversation_id {
        let conversation =
            chatbotx_business::conversation_list::find_or_fail(&ctx.api.db, ws, conversation_id)
                .await
                .map_err(action_fail)?;
        let contact_inbox =
            chatbotx_business::message_actions::resolve_contact_inbox_for_conversation(
                &ctx.api.db,
                &conversation,
                None,
            )
            .await
            .map_err(action_fail)?;
        contact_inbox_id = Some(contact_inbox.id);
    }
    let external = externals(ctx.api);
    let result = booking::book(
        deps(ctx.api, &external),
        booking::BookInput {
            workspace_id: ws,
            calendar_id,
            contact_id,
            conversation_id,
            contact_inbox_id,
            start_at,
            invitee_timezone: Some(invitee_timezone),
            metadata: None,
        },
    )
    .await;
    let mut out = json!({
        "appointment": null,
        "completed": false,
        "staleSlot": false,
        "availabilityChanged": false,
        "alreadyScheduled": false,
    });
    match result {
        Ok(appointment) => {
            out["appointment"] = json!({ "id": id(appointment.id) });
            out["completed"] = json!(true);
        }
        Err(BusinessError::Exception { code, .. }) if code == "slotUnavailable" => {
            out["staleSlot"] = json!(true);
        }
        Err(BusinessError::Exception { code, .. }) if code == "appointmentAlreadyScheduled" => {
            out["alreadyScheduled"] = json!(true);
        }
        Err(BusinessError::Exception { code, .. }) if code == "appointmentAvailabilityChanged" => {
            out["availabilityChanged"] = json!(true);
        }
        Err(error) => return Err(action_fail(error)),
    }
    Ok(out)
}
