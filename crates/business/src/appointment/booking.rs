//! `appointment/service.ts` (`appointmentService`).
//!
//! The Postgres advisory locks (slot, cap, day) are one SQLite write
//! transaction: every check and the insert run under the single writer.

use chatbotx_core::encryption::Encryption;
use chatbotx_core::encryption::tokens::{
    APPOINTMENT_TOKEN_TTL_MS, AppointmentClaims, AppointmentWebviewClaims,
};
use chatbotx_core::utils::tz::{DateArg, format_in_time_zone, is_intl_time_zone};
use chatbotx_db::Db;
use chatbotx_db::repo::appointment::{
    self as repo, AppointmentListInput, AppointmentListRow, ContactAppointmentRow,
    CreateAppointment,
};
use chatbotx_db::repo::models::{Appointment, AppointmentCalendar, Contact, Conversation};
use chatbotx_db::repo::{find_one, now_ms};
use chatbotx_jobs::enqueue::Enqueuer;
use chatbotx_jobs::options::JobOptions;
use chatbotx_jobs::queue::QueueName;
use serde_json::{Value, json};

use super::calendar::{self, Slot, SlotQuery};
use super::external::{self, ExternalCalendars};
use super::reminder;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// The services a booking call uses.
#[derive(Clone, Copy)]
pub struct Deps<'a> {
    pub db: &'a Db,
    pub jobs: Option<&'a dyn Enqueuer>,
    pub external: &'a ExternalCalendars,
}

impl Deps<'_> {
    fn encryption(&self) -> Result<&Encryption> {
        self.external.encryption.as_deref().ok_or_else(|| {
            BusinessError::Repo(chatbotx_db::repo::Error::msg("ENCRYPTION_KEY is not set"))
        })
    }
}

/// `SlotUnavailableException`.
pub fn slot_unavailable() -> BusinessError {
    BusinessError::exception("Appointment slot is unavailable", "slotUnavailable", 409)
}

fn availability_changed() -> BusinessError {
    BusinessError::exception(
        "Appointment calendar availability changed. Please try again.",
        "appointmentAvailabilityChanged",
        409,
    )
}

fn already_scheduled() -> BusinessError {
    BusinessError::exception(
        "Contact already has a scheduled appointment for this calendar",
        "appointmentAlreadyScheduled",
        409,
    )
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Appointment not found")
}

/// `buildAppointmentUrl`: `<appUrl><pathname>?token=<token>`.
pub fn appointment_url(app_url: &str, pathname: &str, token: &str) -> String {
    match url::Url::parse(app_url).and_then(|base| base.join(pathname)) {
        Ok(mut url) => {
            url.query_pairs_mut().clear().append_pair("token", token);
            url.to_string()
        }
        Err(_) => format!("{app_url}{pathname}?token={token}"),
    }
}

fn claims(appointment: &Appointment) -> AppointmentClaims {
    AppointmentClaims {
        appointment_id: appointment.id.to_string(),
        workspace_id: appointment.workspace_id.to_string(),
        contact_id: appointment.contact_id.to_string(),
        conversation_id: appointment.conversation_id.map(|id| id.to_string()),
        ..Default::default()
    }
}

fn token_error(error: impl std::fmt::Display) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.to_string()))
}

fn schedule_url(
    encryption: &Encryption,
    app_url: &str,
    claims: &AppointmentClaims,
) -> Result<String> {
    let token = encryption
        .sign_appointment_schedule_token(claims, APPOINTMENT_TOKEN_TTL_MS)
        .map_err(token_error)?;
    Ok(appointment_url(app_url, "/booking/schedule", &token))
}

/// One `list` item (`cancellable` / `deletable`, the signed schedule URL).
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub row: AppointmentListRow,
    pub contact_name: Option<String>,
    pub schedule_url: String,
    pub cancellable: bool,
    pub deletable: bool,
}

fn name_of(full: Option<&str>, first: Option<&str>, last: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [first, last]
        .into_iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .collect();
    let name = full.map_or_else(|| parts.join(" "), str::to_string);
    (!name.is_empty()).then_some(name)
}

/// `list`: `(items, pageCount)`.
pub async fn list(
    deps: Deps<'_>,
    mut input: AppointmentListInput,
    app_url: &str,
) -> Result<(Vec<ListItem>, i64)> {
    let now = now_ms();
    input.now_ms = now;
    let (rows, page_count, _) =
        tx::read(deps.db, move |conn| Ok(repo::list(conn, &input)?)).await?;
    let encryption = deps.encryption()?;
    let items = rows
        .into_iter()
        .map(|row| {
            let cancellable =
                row.appointment.status == "scheduled" && row.appointment.start_at > now;
            Ok(ListItem {
                contact_name: name_of(
                    row.contact_full_name.as_deref(),
                    row.contact_first_name.as_deref(),
                    row.contact_last_name.as_deref(),
                ),
                schedule_url: schedule_url(encryption, app_url, &claims(&row.appointment))?,
                cancellable,
                deletable: !cancellable,
                row,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((items, page_count))
}

/// `listContactAppointments`: the most relevant non-cancelled appointment per
/// calendar (upcoming first), each with a schedule URL. `restricted_to` is
/// the member's assigned-user id (the contact must be theirs).
pub async fn list_contact_appointments(
    deps: Deps<'_>,
    workspace_id: i64,
    contact_id: i64,
    limit: Option<i64>,
    restricted_to: Option<i64>,
    app_url: &str,
) -> Result<Vec<(ContactAppointmentRow, String)>> {
    let now = now_ms();
    let rows = tx::read(deps.db, move |conn| {
        if let Some(user_id) = restricted_to {
            let visible: Option<Contact> = find_one(
                conn,
                "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"assignedUserId\" = ?3",
                &[&contact_id, &workspace_id, &user_id],
            )?;
            if visible.is_none() {
                return Err(BusinessError::not_found("Contact not found"));
            }
        }
        Ok(repo::list_by_contact(
            conn,
            workspace_id,
            contact_id,
            limit.unwrap_or(20),
            now,
        )?)
    })
    .await?;
    let encryption = deps.encryption()?;
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| row.appointment.status != "cancelled")
        .filter(|row| seen.insert(row.appointment.calendar_id))
        .map(|row| {
            let url = schedule_url(encryption, app_url, &claims(&row.appointment))?;
            Ok((row, url))
        })
        .collect()
}

/// `findByOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Appointment> {
    tx::read(db, move |conn| {
        repo::find_by(conn, workspace_id, id, false)?.ok_or_else(not_found)
    })
    .await
}

/// `bookAppointment` input (`metadata` is passed on to the confirmation flow).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookInput {
    pub workspace_id: i64,
    pub calendar_id: i64,
    pub contact_id: i64,
    pub conversation_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
    pub start_at: i64,
    pub invitee_timezone: Option<String>,
    pub metadata: Option<Value>,
}

/// `bookAppointment`.
pub async fn book(deps: Deps<'_>, input: BookInput) -> Result<Appointment> {
    let (ws, contact_id) = (input.workspace_id, input.contact_id);
    let conversation_id = input.conversation_id;
    tx::read(deps.db, move |conn| {
        find_one::<Contact>(
            conn,
            "\"id\" = ?1 AND \"workspaceId\" = ?2",
            &[&contact_id, &ws],
        )?
        .ok_or_else(|| BusinessError::not_found("Contact not found"))?;
        if let Some(conversation_id) = conversation_id {
            let conversation: Conversation = find_one(
                conn,
                "\"id\" = ?1 AND \"workspaceId\" = ?2",
                &[&conversation_id, &ws],
            )?
            .ok_or_else(|| BusinessError::not_found("Conversation not found"))?;
            if conversation.contact_id != contact_id {
                return Err(BusinessError::exception(
                    "Appointment conversation does not belong to the contact",
                    "invalidAppointmentConversation",
                    400,
                ));
            }
        }
        Ok(())
    })
    .await?;
    let context = calendar::prepare_availability_context(
        deps.db,
        deps.external,
        ws,
        input.calendar_id,
        input.start_at,
        input.start_at,
        true,
    )
    .await?;
    // The external re-check is a network call: it runs before the write
    // (the original ran it under the advisory locks).
    let calendar_row = tx::read(deps.db, {
        let calendar_id = input.calendar_id;
        move |conn| calendar::find_or_fail(conn, ws, calendar_id)
    })
    .await?;
    let busy_conflict = calendar::has_external_busy_conflict(
        deps.db,
        deps.external,
        ws,
        calendar_row.external_connection_id,
        input.start_at,
        input.start_at + calendar_row.duration_minutes * 60_000,
    )
    .await;
    let write = input.clone();
    let (appointment, calendar) = tx::write(deps.db, move |conn| {
        let calendar = calendar::find_or_fail(conn, ws, write.calendar_id)?;
        if !calendar.active {
            return Err(slot_unavailable());
        }
        if calendar::fingerprint(&calendar) != context.fingerprint {
            return Err(availability_changed());
        }
        let now = now_ms();
        if let Some(max) = calendar.max_appointments_per_user {
            let held = repo::list_future_scheduled_for_contact(
                conn,
                ws,
                write.calendar_id,
                write.contact_id,
                now,
            )?;
            if held.len() as i64 >= max {
                return Err(already_scheduled());
            }
        }
        if repo::exists_scheduled_for_contact_at(
            conn,
            ws,
            write.calendar_id,
            write.contact_id,
            write.start_at,
        )? {
            return Err(already_scheduled());
        }
        if busy_conflict {
            return Err(slot_unavailable());
        }
        let slots = calendar::generate_available_slots(
            conn,
            &SlotQuery {
                workspace_id: ws,
                calendar_id: write.calendar_id,
                contact_id: Some(write.contact_id),
                start: write.start_at,
                end: write.start_at,
                busy: Some(context.busy.clone()),
                now,
            },
        )?;
        let Some(slot) = slots.iter().find(|s| s.start_at == write.start_at).copied() else {
            return Err(slot_unavailable());
        };
        if let Some(contact_inbox_id) = write.contact_inbox_id
            && chatbotx_db::repo::contact_inbox::find_by_id_for_contact(
                conn,
                ws,
                write.contact_id,
                contact_inbox_id,
            )?
            .is_none()
        {
            return Err(BusinessError::exception(
                "Appointment contact inbox does not belong to the contact",
                "invalidAppointmentContactInbox",
                400,
            ));
        }
        let google = calendar
            .external_connection_id
            .map(|id| external::google_connection(conn, ws, id))
            .transpose()?;
        let invitee_timezone = write
            .invitee_timezone
            .filter(|tz| is_intl_time_zone(tz))
            .unwrap_or_else(|| calendar.timezone.clone());
        let appointment = repo::create(
            conn,
            &CreateAppointment {
                workspace_id: ws,
                calendar_id: write.calendar_id,
                contact_id: write.contact_id,
                conversation_id: write.conversation_id,
                contact_inbox_id: write.contact_inbox_id,
                start_at: slot.start_at,
                end_at: slot.end_at,
                invitee_timezone,
                location_type: calendar.location_type.clone(),
                location_detail: calendar.location_detail.clone(),
                external_sync_status: google.as_ref().map(|_| "pending".to_string()),
                external_event_integration_id: calendar.external_connection_id,
                external_event_provider_calendar_id: google.map(|g| g.provider_calendar_id),
            },
        )?;
        Ok((appointment, calendar))
    })
    .await?;
    enqueue_external_sync(
        deps.jobs,
        &appointment,
        "create",
        appointment.external_event_integration_id,
    )
    .await;
    if let Err(error) = reminder::schedule_for_appointment(
        deps.db,
        deps.jobs,
        ws,
        appointment.id,
        appointment.calendar_id,
        input.contact_inbox_id,
    )
    .await
    {
        tracing::warn!(
            workspace_id = ws,
            appointment_id = appointment.id,
            err = %error.message(),
            "Failed to schedule appointment reminders"
        );
    }
    enqueue_calendar_flow(
        deps.jobs,
        input.conversation_id,
        input.contact_inbox_id,
        calendar.confirmation_flow_id,
        input.metadata.as_ref(),
        appointment.id,
    )
    .await;
    Ok(appointment)
}

/// `syncExternalCalendarEventJobId`.
pub fn sync_external_job_id(appointment_id: i64, operation: &str) -> String {
    format!("sync-external-event-{appointment_id}-{operation}")
}

async fn enqueue_external_sync(
    jobs: Option<&dyn Enqueuer>,
    appointment: &Appointment,
    operation: &str,
    external_connection_id: Option<i64>,
) {
    if external_connection_id.is_none() {
        return;
    }
    let Some(jobs) = jobs else {
        tracing::warn!(
            appointment_id = appointment.id,
            "Failed to enqueue external calendar sync"
        );
        return;
    };
    let data = json!({
        "type": "syncExternalCalendarEvent",
        "data": {
            "workspaceId": appointment.workspace_id.to_string(),
            "appointmentId": appointment.id.to_string(),
            "operation": operation,
        },
    });
    let opts = JobOptions {
        job_id: Some(sync_external_job_id(appointment.id, operation)),
        ..Default::default()
    };
    if let Err(error) = jobs
        .add(QueueName::Default, "syncExternalCalendarEvent", data, opts)
        .await
    {
        tracing::warn!(
            appointment_id = appointment.id,
            operation,
            err = %error,
            "Failed to enqueue external calendar sync"
        );
    }
}

async fn enqueue_calendar_flow(
    jobs: Option<&dyn Enqueuer>,
    conversation_id: Option<i64>,
    contact_inbox_id: Option<i64>,
    flow_id: Option<i64>,
    metadata: Option<&Value>,
    appointment_id: i64,
) {
    let (Some(flow_id), Some(conversation_id), Some(contact_inbox_id)) =
        (flow_id, conversation_id, contact_inbox_id)
    else {
        return;
    };
    let Some(jobs) = jobs else {
        tracing::warn!(flow_id, "Failed to enqueue appointment follow-up flow");
        return;
    };
    let mut data = json!({
        "conversationId": conversation_id.to_string(),
        "contactInboxId": contact_inbox_id.to_string(),
        "flowId": flow_id.to_string(),
        "appointmentId": appointment_id.to_string(),
        "origin": "channel",
    });
    if let Some(metadata) = metadata {
        data["metadata"] = metadata.clone();
    }
    if let Err(error) = jobs
        .add(
            QueueName::Integration,
            "sendFlow",
            json!({ "type": "sendFlow", "data": data }),
            JobOptions::default(),
        )
        .await
    {
        tracing::warn!(flow_id, err = %error, "Failed to enqueue appointment follow-up flow");
    }
}

/// `getCancellationExternalSyncStatus`.
fn cancellation_sync_status(
    appointment: &Appointment,
    calendar: &AppointmentCalendar,
) -> Option<String> {
    if appointment.external_event_integration_id.is_some()
        || calendar.external_connection_id.is_some()
    {
        Some("pending".into())
    } else {
        appointment.external_sync_status.clone()
    }
}

/// A cancelled appointment with what its side effects need.
struct Cancelled {
    appointment: Appointment,
    calendar: AppointmentCalendar,
}

fn cancel_in(
    conn: &chatbotx_db::rusqlite::Connection,
    row: &Appointment,
) -> Result<Option<Cancelled>> {
    let calendar = chatbotx_db::repo::appointment_calendar::find_by(
        conn,
        row.workspace_id,
        row.calendar_id,
        true,
    )?
    .ok_or_else(not_found)?;
    let status = cancellation_sync_status(row, &calendar);
    Ok(
        repo::cancel_scheduled(conn, row.workspace_id, row.id, now_ms(), status.as_deref())?.map(
            |appointment| Cancelled {
                appointment,
                calendar,
            },
        ),
    )
}

async fn cancellation_side_effects(
    deps: Deps<'_>,
    cancelled: &Cancelled,
    conversation_id: Option<i64>,
    contact_inbox_id: Option<i64>,
    metadata: Option<&Value>,
) -> Result<()> {
    let a = &cancelled.appointment;
    reminder::cancel_pending_for_appointment(deps.db, a.workspace_id, a.id).await?;
    let external = a
        .external_event_integration_id
        .or(cancelled.calendar.external_connection_id);
    enqueue_external_sync(deps.jobs, a, "cancel", external).await;
    enqueue_calendar_flow(
        deps.jobs,
        conversation_id,
        contact_inbox_id,
        cancelled.calendar.cancellation_flow_id,
        metadata,
        a.id,
    )
    .await;
    Ok(())
}

/// `cancelAppointment` (the flow step / postback): the contact's one future
/// appointment on the calendar.
pub async fn cancel_for_contact(
    deps: Deps<'_>,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: i64,
    conversation_id: Option<i64>,
    contact_inbox_id: Option<i64>,
    metadata: Option<Value>,
) -> Result<Appointment> {
    let cancelled = tx::write(deps.db, move |conn| {
        let rows = repo::list_future_scheduled_for_contact(
            conn,
            workspace_id,
            calendar_id,
            contact_id,
            now_ms(),
        )?;
        if rows.len() != 1 {
            return Err(BusinessError::exception(
                format!(
                    "Expected exactly one future appointment to cancel, found {}",
                    rows.len()
                ),
                "ambiguousAppointmentCancel",
                409,
            ));
        }
        cancel_in(conn, &rows[0])?.ok_or_else(not_found)
    })
    .await?;
    cancellation_side_effects(
        deps,
        &cancelled,
        conversation_id,
        contact_inbox_id,
        metadata.as_ref(),
    )
    .await?;
    Ok(cancelled.appointment)
}

/// `cancelAppointmentByToken`: `(cancellable, appointment)`.
pub async fn cancel_by_token(
    deps: Deps<'_>,
    workspace_id: i64,
    appointment_id: i64,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
) -> Result<(bool, Appointment)> {
    let (cancelled, row) = tx::write(deps.db, move |conn| {
        let row = repo::find_by(conn, workspace_id, appointment_id, false)?
            .filter(|a| a.contact_id == contact_id)
            .ok_or_else(not_found)?;
        if row.status != "scheduled" || row.start_at <= now_ms() {
            return Ok((None, row));
        }
        Ok((cancel_in(conn, &row)?, row))
    })
    .await?;
    match cancelled {
        Some(cancelled) => {
            let conversation_id = cancelled.appointment.conversation_id;
            cancellation_side_effects(deps, &cancelled, conversation_id, contact_inbox_id, None)
                .await?;
            Ok((true, cancelled.appointment))
        }
        None => Ok((false, row)),
    }
}

/// `cancelAppointmentById` (`appointmentNotCancellable` 409 for a past or
/// cancelled one).
pub async fn cancel_by_id(
    deps: Deps<'_>,
    workspace_id: i64,
    appointment_id: i64,
) -> Result<Appointment> {
    let cancelled = tx::write(deps.db, move |conn| {
        let row =
            repo::find_by(conn, workspace_id, appointment_id, false)?.ok_or_else(not_found)?;
        if row.status != "scheduled" || row.start_at <= now_ms() {
            return Err(BusinessError::exception(
                "Appointment cannot be cancelled",
                "appointmentNotCancellable",
                409,
            ));
        }
        cancel_in(conn, &row)?.ok_or_else(not_found)
    })
    .await?;
    let conversation_id = cancelled.appointment.conversation_id;
    cancellation_side_effects(deps, &cancelled, conversation_id, None, None).await?;
    Ok(cancelled.appointment)
}

/// `deleteAppointmentById` (soft delete; an upcoming scheduled one is refused).
pub async fn delete_by_id(db: &Db, workspace_id: i64, appointment_id: i64) -> Result<Appointment> {
    tx::write(db, move |conn| {
        let row =
            repo::find_by(conn, workspace_id, appointment_id, false)?.ok_or_else(not_found)?;
        if row.status == "scheduled" && row.start_at > now_ms() {
            return Err(BusinessError::exception(
                "Cancel upcoming appointments before deleting them",
                "appointmentDeleteBlocked",
                409,
            ));
        }
        repo::soft_delete(conn, workspace_id, appointment_id)?.ok_or_else(not_found)
    })
    .await
}

/// `checkAvailability`: `(text, slots, timezone)`; the text lists the first
/// 30 slot starts (`yyyy-MM-dd hh:mm:ss a` in the calendar zone).
pub async fn check_availability(
    deps: Deps<'_>,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: Option<i64>,
    start: i64,
    end: i64,
) -> Result<(String, Vec<Slot>, String)> {
    let calendar = tx::read(deps.db, move |conn| {
        calendar::find_or_fail(conn, workspace_id, calendar_id)
    })
    .await?;
    let slots = calendar::resolve_available_slots_for_listing(
        deps.db,
        deps.external,
        workspace_id,
        calendar_id,
        contact_id,
        start,
        end,
    )
    .await?;
    let text = slots
        .iter()
        .take(30)
        .map(|s| {
            format_in_time_zone(
                DateArg::Ms(s.start_at),
                &calendar.timezone,
                "yyyy-MM-dd hh:mm:ss a",
            )
            .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok((text, slots, calendar.timezone))
}

/// `completeWebviewBooking`: books the picked slot of a webview token;
/// `(appointment, scheduleUrl, cancelUrl)`.
pub async fn complete_webview_booking(
    deps: Deps<'_>,
    token: &AppointmentWebviewClaims,
    availability_range: Option<(i64, i64)>,
    selected_start_at: i64,
    invitee_timezone: &str,
    app_url: &str,
) -> Result<(Appointment, String, String)> {
    let id = |text: &str| text.parse::<i64>().map_err(|_| not_found());
    let workspace_id = id(&token.workspace_id)?;
    let calendar_id = id(&token.calendar_id)?;
    if let Some((range_start, range_end)) = availability_range {
        let calendar = tx::read(deps.db, move |conn| {
            calendar::find_or_fail(conn, workspace_id, calendar_id)
        })
        .await?;
        let selected_end = selected_start_at + calendar.duration_minutes * 60_000;
        if selected_start_at < range_start || selected_end > range_end {
            return Err(slot_unavailable());
        }
    }
    let appointment = book(
        deps,
        BookInput {
            workspace_id,
            calendar_id,
            contact_id: id(&token.contact_id)?,
            conversation_id: Some(id(&token.conversation_id)?),
            contact_inbox_id: Some(id(&token.contact_inbox_id)?),
            start_at: selected_start_at,
            invitee_timezone: Some(invitee_timezone.to_string()),
            metadata: None,
        },
    )
    .await?;
    let full = find_or_fail(deps.db, workspace_id, appointment.id).await?;
    let mut claims = claims(&full);
    claims.contact_inbox_id = Some(token.contact_inbox_id.clone());
    claims.flow_version_id = token.flow_version_id.clone();
    let encryption = deps.encryption()?;
    let schedule = schedule_url(encryption, app_url, &claims)?;
    let cancel = encryption
        .sign_appointment_cancel_token(&claims, APPOINTMENT_TOKEN_TTL_MS)
        .map_err(token_error)?;
    Ok((
        full,
        schedule,
        appointment_url(app_url, "/booking/cancel", &cancel),
    ))
}

/// `getScheduleDetailByToken`.
pub async fn schedule_detail(
    db: &Db,
    workspace_id: i64,
    appointment_id: i64,
    contact_id: i64,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let a = repo::find_by(conn, workspace_id, appointment_id, false)?
            .filter(|a| a.contact_id == contact_id)
            .ok_or_else(not_found)?;
        let calendar = chatbotx_db::repo::appointment_calendar::find_by(
            conn,
            workspace_id,
            a.calendar_id,
            true,
        )?
        .ok_or_else(not_found)?;
        let contact: Option<Contact> = find_one(conn, "\"id\" = ?1", &[&a.contact_id])?;
        let contact_name = contact.and_then(|c| {
            name_of(
                c.full_name.as_deref(),
                c.first_name.as_deref(),
                c.last_name.as_deref(),
            )
            .or(c.email.filter(|e| !e.is_empty()))
            .or(c.phone_number.filter(|p| !p.is_empty()))
        });
        Ok(json!({
            "id": a.id.to_string(),
            "workspaceId": a.workspace_id.to_string(),
            "contactId": a.contact_id.to_string(),
            "conversationId": a.conversation_id.map(|id| id.to_string()),
            "calendarName": calendar.name,
            "contactName": contact_name,
            "startAt": chatbotx_db::repo::iso(a.start_at),
            "endAt": chatbotx_db::repo::iso(a.end_at),
            "inviteeTimezone": a.invitee_timezone,
            "status": a.status,
            "locationType": a.location_type,
            "locationDetail": a.location_detail,
            "confirmationMessage": calendar.confirmation_message,
            "cancellable": a.status == "scheduled" && a.start_at > now_ms(),
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appointment_url_sets_the_token_query() {
        assert_eq!(
            appointment_url("https://app.example.com", "/booking/cancel", "a+b"),
            "https://app.example.com/booking/cancel?token=a%2Bb"
        );
    }

    #[test]
    fn contact_name_prefers_full_name_then_parts() {
        assert_eq!(
            name_of(Some("Ann B"), Some("x"), None).as_deref(),
            Some("Ann B")
        );
        assert_eq!(
            name_of(None, Some("Ann"), Some("B")).as_deref(),
            Some("Ann B")
        );
        assert_eq!(name_of(None, None, None), None);
        assert_eq!(name_of(Some(""), Some("Ann"), None), None);
    }
}

/// `AppointmentSchedulingCopy` (`appointment/copy.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulingCopy {
    pub booking_prompt: &'static str,
    pub booking_button: &'static str,
    pub range_prompt: &'static str,
    pub range_button: &'static str,
    pub no_slots_found: &'static str,
}

/// `getAppointmentSchedulingCopy`: Vietnamese for `vi`, else English.
pub fn scheduling_copy(language: Option<&str>) -> SchedulingCopy {
    if crate::contact::update_fields::normalize_language(language).as_deref() == Some("vi") {
        SchedulingCopy {
            booking_prompt: "Chọn thời gian đặt lịch",
            booking_button: "Chọn ngày",
            range_prompt: "Chọn khoảng ngày cần kiểm tra lịch trống",
            range_button: "Kiểm tra lịch trống",
            no_slots_found: "Không có lịch trống.",
        }
    } else {
        SchedulingCopy {
            booking_prompt: "Choose an appointment time",
            booking_button: "Select Date",
            range_prompt: "Choose a date range to check availability",
            range_button: "Check Availability",
            no_slots_found: "No available slots found.",
        }
    }
}

/// `createInboxBookingLink` result: the picker URL, its expiry, the calendar
/// `(id, name)` and the message (`text`, URL `button`).
#[derive(Debug, Clone, PartialEq)]
pub struct InboxBookingLink {
    pub url: String,
    pub expires_at: i64,
    pub calendar: (i64, String),
    pub text: String,
    pub button: Value,
}

/// `createInboxBookingLink`: a 24 h booking-picker link for the conversation's
/// contact (`appointmentCalendarInactive` 409 for an inactive calendar).
pub async fn create_inbox_booking_link(
    deps: Deps<'_>,
    workspace_id: i64,
    conversation: &Conversation,
    contact_inbox: &chatbotx_db::repo::models::ContactInbox,
    calendar_id: i64,
    app_url: &str,
) -> Result<InboxBookingLink> {
    let calendar = tx::read(deps.db, move |conn| {
        calendar::find_or_fail(conn, workspace_id, calendar_id)
    })
    .await?;
    if !calendar.active {
        return Err(BusinessError::exception(
            "Appointment calendar is inactive",
            "appointmentCalendarInactive",
            409,
        ));
    }
    let claims = AppointmentWebviewClaims {
        source: chatbotx_core::encryption::tokens::AppointmentTokenSource::Inbox,
        workspace_id: workspace_id.to_string(),
        calendar_id: calendar.id.to_string(),
        contact_id: conversation.contact_id.to_string(),
        conversation_id: conversation.id.to_string(),
        contact_inbox_id: contact_inbox.id.to_string(),
        ..Default::default()
    };
    let (token, expires_at) = deps
        .encryption()?
        .sign_appointment_webview_token_with_expiry(
            &claims,
            chatbotx_core::encryption::tokens::APPOINTMENT_INBOX_LINK_TTL_MS,
        )
        .map_err(token_error)?;
    let url = appointment_url(app_url, "/booking/picker", &token);
    let copy = scheduling_copy(contact_inbox.language.as_deref());
    Ok(InboxBookingLink {
        button: json!({
            "id": chatbotx_core::utils::id::create_id(),
            "label": copy.booking_button,
            "buttonType": "url",
            "url": url,
            "messengerExtensions": true,
        }),
        url,
        expires_at,
        calendar: (calendar.id, calendar.name),
        text: copy.booking_prompt.to_string(),
    })
}
