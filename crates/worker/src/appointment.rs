//! The appointment jobs: `sendAppointmentReminder` and
//! `syncExternalCalendarEvent` (default queue, `default/handlers/*.ts`) and
//! the `scanAppointmentReminders` cron (`schedule/handlers/*.ts`).

use chatbotx_business::BusinessError;
use chatbotx_business::appointment::external::{ExternalCalendars, deterministic_event_id};
use chatbotx_business::appointment::reminder;
use chatbotx_business::tx;
use chatbotx_db::repo::appointment as repo;
use chatbotx_db::repo::models::{
    Appointment, AppointmentCalendar, Contact, ContactInbox, Conversation,
};
use chatbotx_db::repo::{appointment_calendar, find_one, now_ms};
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::{Value, json};

use crate::{Deps, fail, id_of};

const TITLE_MAX: usize = 1024;
const DESCRIPTION_MAX: usize = 8192;
const ATTENDEES_MAX: usize = 8192;

fn business(error: BusinessError) -> JobError {
    JobError::retryable(error.to_string())
}

fn ids(data: &Value, keys: &[&str]) -> Result<Vec<i64>, JobError> {
    keys.iter()
        .map(|k| id_of(&data[*k]).ok_or_else(|| fail(format!("appointment job: bad {k}"))))
        .collect()
}

/// `sendAppointmentReminder`.
pub async fn send_reminder(deps: &Deps, data: &Value) -> Result<Value, JobError> {
    let v = ids(
        data,
        &[
            "workspaceId",
            "appointmentId",
            "reminderDispatchId",
            "reminderConfigId",
        ],
    )?;
    let outcome = reminder::send_dispatch(&deps.db, deps.jobs.as_ref(), v[0], v[1], v[2], v[3])
        .await
        .map_err(|error| {
            tracing::error!(
                workspace_id = v[0],
                appointment_id = v[1],
                reminder_dispatch_id = v[2],
                err = %error,
                "Appointment reminder dispatch failed"
            );
            business(error)
        })?;
    let reason = match outcome {
        reminder::SendOutcome::Sent => "sent",
        reminder::SendOutcome::NotPending => "notPending",
        reminder::SendOutcome::StaleAppointment => "staleAppointment",
        reminder::SendOutcome::MissingConversation => "missingConversation",
        reminder::SendOutcome::MissingContactInbox => "missingContactInbox",
    };
    Ok(json!({ "sent": outcome == reminder::SendOutcome::Sent, "reason": reason }))
}

/// `scanAppointmentReminders`. ponytail: no distributed lock; the cron
/// runs one at a time per scheduler and the enqueue is idempotent on job id.
pub async fn scan_reminders(deps: &Deps) -> Result<(), JobError> {
    let (scanned, enqueued) =
        reminder::enqueue_due_pending(&deps.db, Some(deps.jobs.as_ref()), now_ms())
            .await
            .map_err(business)?;
    if enqueued > 0 {
        tracing::info!(
            scanned,
            enqueued,
            "Appointment reminder scanner enqueued due jobs"
        );
    }
    Ok(())
}

struct Loaded {
    appointment: Appointment,
    calendar: AppointmentCalendar,
    contact: Option<Contact>,
    conversation: Option<Conversation>,
    contact_inbox: Option<ContactInbox>,
}

async fn load(deps: &Deps, ws: i64, id: i64) -> Result<Loaded, BusinessError> {
    tx::read(&deps.db, move |conn| {
        let appointment = repo::find_by(conn, ws, id, true)?
            .ok_or_else(|| BusinessError::not_found("Appointment not found"))?;
        let calendar = appointment_calendar::find_by(conn, ws, appointment.calendar_id, true)?
            .ok_or_else(|| BusinessError::not_found("Appointment calendar not found"))?;
        let contact = find_one::<Contact>(conn, "\"id\" = ?1", &[&appointment.contact_id])?;
        let conversation = match appointment.conversation_id {
            Some(cid) => find_one::<Conversation>(
                conn,
                "\"id\" = ?1 AND \"workspaceId\" = ?2",
                &[&cid, &ws],
            )?,
            None => None,
        };
        let contact_inbox = match appointment.contact_inbox_id {
            Some(ci) => find_one::<ContactInbox>(conn, "\"id\" = ?1", &[&ci])?,
            None => None,
        };
        Ok(Loaded {
            appointment,
            calendar,
            contact,
            conversation,
            contact_inbox,
        })
    })
    .await
}

/// `syncExternalCalendarEvent`. ponytail: no distributed lock; the job id
/// (`sync-external-event-{id}-{operation}`) keeps one job per operation.
pub async fn sync_external(deps: &Deps, job: &JobCtx, data: &Value) -> Result<Value, JobError> {
    let v = ids(data, &["workspaceId", "appointmentId"])?;
    let (ws, id) = (v[0], v[1]);
    let operation = crate::str_of(data, "operation").unwrap_or_default();
    if operation != "create" && operation != "cancel" {
        return Err(JobError::unrecoverable(
            "syncExternalCalendarEvent: bad operation",
        ));
    }
    let loaded = load(deps, ws, id).await.map_err(business)?;
    let wanted = if operation == "create" {
        "scheduled"
    } else {
        "cancelled"
    };
    if loaded.appointment.status != wanted {
        return Ok(Value::Null);
    }
    let external = ExternalCalendars {
        encryption: crate::channels::meta_webhook::encryption_from_env(),
        ..Default::default()
    };
    match run_sync(deps, &external, &loaded, operation).await {
        Ok(()) => Ok(Value::Null),
        Err(error) => {
            let message = error.to_string();
            let failed = if operation == "create" {
                "scheduled"
            } else {
                "cancelled"
            };
            let _ = tx::write(&deps.db, move |conn| {
                Ok(repo::set_external_sync_status(
                    conn, ws, id, failed, "failed",
                )?)
            })
            .await;
            if message.starts_with("appointment_external_event_") {
                return Err(JobError::unrecoverable(message));
            }
            tracing::error!(
                workspace_id = ws,
                appointment_id = id,
                operation,
                reason = "appointmentExternalCalendarSyncFailed",
                err = %message,
                "External calendar event sync failed"
            );
            if job.is_final_attempt() {
                crate::events::log_provider_errors(
                    deps.events.as_ref(),
                    &[crate::events::ProviderError {
                        provider: "google-calendar".into(),
                        workspace_id: ws.to_string(),
                        contact_id: Some(loaded.appointment.contact_id.to_string()),
                        source_id: loaded.contact_inbox.as_ref().map(|ci| ci.source_id.clone()),
                        error: json!({ "message": message }),
                        http_code: None,
                        stack_trace: None,
                    }],
                )
                .await;
            }
            Err(JobError::retryable(message))
        }
    }
}

async fn run_sync(
    deps: &Deps,
    external: &ExternalCalendars,
    loaded: &Loaded,
    operation: &str,
) -> Result<(), BusinessError> {
    let a = &loaded.appointment;
    let (ws, id) = (a.workspace_id, a.id);
    let Some((integration_id, provider_calendar_id)) =
        resolve_destination(deps, loaded, operation).await?
    else {
        return Ok(());
    };
    if operation == "create" {
        let Some(contact_inbox) = &loaded.contact_inbox else {
            return Err(template_error("contact_inbox_missing"));
        };
        let rendered = render_templates(deps, loaded, contact_inbox).await?;
        let event_id = deterministic_event_id(id);
        let event = json!({
            "id": event_id,
            "summary": rendered.0,
            "description": rendered.1,
            "location": a.location_detail,
            "start": { "dateTime": chatbotx_db::repo::iso(a.start_at), "timeZone": loaded.calendar.timezone },
            "end": { "dateTime": chatbotx_db::repo::iso(a.end_at), "timeZone": loaded.calendar.timezone },
            "attendees": rendered.2,
        });
        external
            .create_event(&deps.db, ws, integration_id, &provider_calendar_id, &event)
            .await?;
        let synced_id = event_id.clone();
        let updated = tx::write(&deps.db, move |conn| {
            Ok(repo::mark_external_create_synced(conn, ws, id, &synced_id)?)
        })
        .await?;
        if updated.is_none() {
            external
                .cancel_event(
                    &deps.db,
                    ws,
                    integration_id,
                    &provider_calendar_id,
                    &event_id,
                )
                .await?;
        }
        return Ok(());
    }
    let event_id = a
        .external_event_id
        .clone()
        .unwrap_or_else(|| deterministic_event_id(id));
    external
        .cancel_event(
            &deps.db,
            ws,
            integration_id,
            &provider_calendar_id,
            &event_id,
        )
        .await?;
    tx::write(&deps.db, move |conn| {
        Ok(repo::set_external_sync_status(
            conn,
            ws,
            id,
            "cancelled",
            "synced",
        )?)
    })
    .await?;
    Ok(())
}

fn template_error(what: &str) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(format!(
        "appointment_external_event_{what}"
    )))
}

async fn resolve_destination(
    deps: &Deps,
    loaded: &Loaded,
    operation: &str,
) -> Result<Option<(i64, String)>, BusinessError> {
    let a = &loaded.appointment;
    if let (Some(integration), Some(calendar)) = (
        a.external_event_integration_id,
        a.external_event_provider_calendar_id.clone(),
    ) {
        return Ok(Some((integration, calendar)));
    }
    let Some(integration_id) = loaded.calendar.external_connection_id else {
        return Ok(None);
    };
    if operation == "cancel" && a.external_event_id.is_none() {
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
            "appointment_external_calendar_legacy_cancel_missing_event",
        )));
    }
    let (ws, id) = (a.workspace_id, a.id);
    let connection = tx::read(&deps.db, move |conn| {
        chatbotx_business::appointment::external::google_connection(conn, ws, integration_id)
    })
    .await?;
    let provider_calendar_id = connection.provider_calendar_id;
    if operation == "create" {
        let pc = provider_calendar_id.clone();
        let persisted = tx::write(&deps.db, move |conn| {
            Ok(repo::persist_external_destination(
                conn,
                ws,
                id,
                integration_id,
                &pc,
            )?)
        })
        .await?;
        if persisted.is_none() {
            return Ok(None);
        }
    }
    Ok(Some((integration_id, provider_calendar_id)))
}

/// `renderAppointmentExternalCalendarEventTemplate`: (summary, description, attendees).
async fn render_templates(
    deps: &Deps,
    loaded: &Loaded,
    contact_inbox: &ContactInbox,
) -> Result<(String, Option<String>, Option<Vec<Value>>), BusinessError> {
    let c = &loaded.calendar;
    let templates = json!([
        c.external_event_title_template.clone().unwrap_or_default(),
        c.external_event_description_template
            .clone()
            .unwrap_or_default(),
        c.external_event_attendees_template
            .clone()
            .unwrap_or_default(),
    ]);
    // ponytail: without a conversation (or `ENCRYPTION_KEY`) the templates
    // are used as written; `{{variables}}` need the conversation context.
    let rendered = match (&deps.variables, &loaded.conversation) {
        (Some(vars), Some(conversation)) => vars
            .resolve(
                conversation,
                contact_inbox,
                &templates,
                Some(loaded.appointment.id.to_string()),
                None,
                false,
            )
            .await
            .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e)))?,
        _ => templates,
    };
    let text = |i: usize| rendered[i].as_str().unwrap_or_default().to_string();
    let (title, description, attendees) = (text(0), text(1), text(2));
    for (value, max, field) in [
        (&title, TITLE_MAX, "title"),
        (&description, DESCRIPTION_MAX, "description"),
        (&attendees, ATTENDEES_MAX, "attendees"),
    ] {
        if value.encode_utf16().count() > max {
            return Err(template_error(&format!("{field}_too_long")));
        }
    }
    let attendees = if c.external_event_attendees_template.is_none() {
        loaded
            .contact
            .as_ref()
            .and_then(|contact| contact.email.clone())
            .filter(|e| !e.is_empty())
            .map(|email| vec![json!({ "email": email })])
    } else {
        parse_attendees(&attendees)
    };
    let summary = match title.trim() {
        "" => format!("Appointment: {}", c.name),
        t => t.to_string(),
    };
    let description = Some(description.trim().to_string()).filter(|d| !d.is_empty());
    Ok((summary, description, attendees))
}

/// `z.string().email()` (simplified: one `@`, a dotted domain, no spaces).
fn is_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && !value.chars().any(char::is_whitespace)
        && domain.split('.').count() >= 2
        && domain.split('.').all(|part| !part.is_empty())
}

fn parse_attendees(value: &str) -> Option<Vec<Value>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut invalid = 0;
    for candidate in value.split(['\n', ',', ';']) {
        let email = candidate.trim();
        if email.is_empty() {
            continue;
        }
        if !is_email(email) {
            invalid += 1;
            continue;
        }
        if seen.insert(email.to_lowercase()) {
            out.push(json!({ "email": email }));
        }
    }
    if invalid > 0 {
        tracing::warn!(
            invalid_count = invalid,
            reason = "invalidAppointmentExternalEventAttendees",
            "Skipped invalid appointment external-calendar attendee addresses"
        );
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attendees_are_split_validated_and_deduplicated() {
        let parsed = parse_attendees("a@x.io, A@x.io;bad\nb@y.com").unwrap();
        assert_eq!(
            parsed,
            vec![json!({ "email": "a@x.io" }), json!({ "email": "b@y.com" })]
        );
        assert!(parse_attendees(" , bad").is_none());
    }

    #[test]
    fn event_id_is_deterministic() {
        let id = deterministic_event_id(42);
        assert!(id.starts_with("abe") && id.len() == 67);
        assert_eq!(id, deterministic_event_id(42));
    }
}
