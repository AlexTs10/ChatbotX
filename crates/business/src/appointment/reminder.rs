//! `appointment-reminder/service.ts` (`appointmentReminderService`).
//!
//! No queue removal: the `Enqueuer` has no `remove`, so a stale reminder job
//! stays queued; [`send_dispatch`] checks the dispatch row (`pending`) and the
//! appointment first, so a cancelled or moved one sends nothing. No
//! distributed lock: the send runs its checks and the `sent` write in one
//! SQLite write transaction.

use chatbotx_db::Db;
use chatbotx_db::repo::appointment as repo;
use chatbotx_db::repo::appointment_calendar;
use chatbotx_db::repo::models::AppointmentReminderDispatch;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{Connection, OptionalExtension};
use chatbotx_jobs::enqueue::Enqueuer;
use chatbotx_jobs::options::{JobOptions, Retention};
use chatbotx_jobs::queue::QueueName;
use serde_json::json;

use crate::errors::{BusinessError, Result};
use crate::tx;

const DUE_REMINDER_BATCH_SIZE: i64 = 100;

/// `sendAppointmentReminderJobId`.
pub fn reminder_job_id(appointment_id: i64, reminder_config_id: i64) -> String {
    format!("appt-reminder-{appointment_id}-{reminder_config_id}")
}

/// `calculateSendAt`: `days` move the local wall clock back (DST-safe), the
/// other units subtract a fixed duration.
pub fn calculate_send_at(
    start_at: i64,
    timing_value: i64,
    timing_unit: &str,
    timezone: &str,
) -> i64 {
    match timing_unit {
        "days" => {
            use chatbotx_core::utils::tz::{DateArg, format_in_time_zone, from_zoned_time};
            let local =
                format_in_time_zone(DateArg::Ms(start_at), timezone, "yyyy-MM-dd'T'HH:mm:ss.SSS")
                    .ok()
                    .and_then(|text| {
                        chrono::NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.3f").ok()
                    });
            match local.and_then(|at| at.checked_sub_days(chrono::Days::new(timing_value as u64))) {
                Some(at) => {
                    from_zoned_time(&at.format("%Y-%m-%dT%H:%M:%S%.3f").to_string(), timezone)
                        .unwrap_or(start_at - timing_value * 86_400_000)
                }
                None => start_at - timing_value * 86_400_000,
            }
        }
        "hours" => start_at - timing_value * 3_600_000,
        _ => start_at - timing_value * 60_000,
    }
}

/// One `sendAppointmentReminder` job to add.
struct Enqueue {
    workspace_id: i64,
    appointment_id: i64,
    dispatch_id: i64,
    reminder_config_id: i64,
    send_at: i64,
    job_id: String,
}

async fn enqueue_dispatch(jobs: Option<&dyn Enqueuer>, job: Enqueue) {
    let Some(jobs) = jobs else {
        tracing::warn!(
            reminder_dispatch_id = job.dispatch_id,
            "Failed to enqueue appointment reminder job; scanner will retry"
        );
        return;
    };
    let opts = JobOptions {
        job_id: Some(job.job_id),
        delay_ms: Some(u64::try_from(job.send_at - now_ms()).unwrap_or(0)),
        remove_on_complete: Some(Retention::KeepAge {
            age_secs: 86_400,
            count: Some(10_000),
        }),
        remove_on_fail: Some(Retention::Remove),
        ..Default::default()
    };
    let data = json!({
        "type": "sendAppointmentReminder",
        "data": {
            "workspaceId": job.workspace_id.to_string(),
            "appointmentId": job.appointment_id.to_string(),
            "reminderDispatchId": job.dispatch_id.to_string(),
            "reminderConfigId": job.reminder_config_id.to_string(),
        },
    });
    if let Err(error) = jobs
        .add(QueueName::Default, "sendAppointmentReminder", data, opts)
        .await
    {
        tracing::warn!(
            workspace_id = job.workspace_id,
            appointment_id = job.appointment_id,
            reminder_dispatch_id = job.dispatch_id,
            err = %error,
            "Failed to enqueue appointment reminder job; scanner will retry"
        );
    }
}

/// `scheduleForAppointment`: a pending dispatch (and its delayed job) per
/// reminder whose send time is still ahead.
pub async fn schedule_for_appointment(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    workspace_id: i64,
    appointment_id: i64,
    calendar_id: i64,
    contact_inbox_id: Option<i64>,
) -> Result<Vec<AppointmentReminderDispatch>> {
    let now = now_ms();
    let created = tx::write(db, move |conn| {
        let calendar = appointment_calendar::get_for_edit(conn, workspace_id, calendar_id)?
            .ok_or_else(|| {
                BusinessError::Repo(chatbotx_db::repo::Error::msg(
                    "Appointment calendar not found",
                ))
            })?;
        let appointment =
            repo::find_by(conn, workspace_id, appointment_id, false)?.ok_or_else(|| {
                BusinessError::Repo(chatbotx_db::repo::Error::msg("Appointment not found"))
            })?;
        if appointment.status != "scheduled" {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for reminder in &calendar.reminders {
            let send_at = calculate_send_at(
                appointment.start_at,
                reminder.timing_value,
                &reminder.timing_unit,
                &calendar.calendar.timezone,
            );
            if send_at <= now {
                continue;
            }
            let job_id = reminder_job_id(appointment_id, reminder.id);
            if let Some(dispatch) = repo::create_pending_dispatch(
                conn,
                workspace_id,
                appointment_id,
                reminder.id,
                contact_inbox_id,
                send_at,
                &job_id,
            )? {
                out.push((dispatch, send_at));
            }
        }
        Ok(out)
    })
    .await?;
    let mut scheduled = Vec::with_capacity(created.len());
    for (dispatch, send_at) in created {
        enqueue_dispatch(
            jobs,
            Enqueue {
                workspace_id,
                appointment_id,
                dispatch_id: dispatch.id,
                reminder_config_id: dispatch.reminder_config_id,
                send_at,
                job_id: dispatch.job_id.clone(),
            },
        )
        .await;
        scheduled.push(dispatch);
    }
    Ok(scheduled)
}

/// `rescheduleFutureForCalendar`: schedules the reminders of every future
/// appointment again (after a calendar edit); the dispatches created.
pub async fn reschedule_future_for_calendar(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    workspace_id: i64,
    calendar_id: i64,
    now: i64,
) -> Result<usize> {
    let appointments = tx::read(db, move |conn| {
        Ok(repo::list_future_scheduled_by_calendar(
            conn,
            workspace_id,
            calendar_id,
            now,
        )?)
    })
    .await?;
    let mut created = 0;
    for appointment in &appointments {
        match schedule_for_appointment(db, jobs, workspace_id, appointment.id, calendar_id, None)
            .await
        {
            Ok(rows) => created += rows.len(),
            Err(error) => tracing::warn!(
                workspace_id,
                calendar_id,
                appointment_id = appointment.id,
                err = %error.message(),
                "Failed to reschedule appointment reminders"
            ),
        }
    }
    Ok(created)
}

/// `cancelPendingForAppointment`.
pub async fn cancel_pending_for_appointment(
    db: &Db,
    workspace_id: i64,
    appointment_id: i64,
) -> Result<Vec<AppointmentReminderDispatch>> {
    tx::write(db, move |conn| {
        Ok(repo::mark_dispatches_cancelled(
            conn,
            workspace_id,
            appointment_id,
            &[],
        )?)
    })
    .await
}

/// `enqueueDuePending` (the `scanAppointmentReminders` cron): `(scanned, enqueued)`.
pub async fn enqueue_due_pending(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    now: i64,
) -> Result<(usize, usize)> {
    let rows = tx::read(db, move |conn| {
        Ok(repo::list_due_pending_dispatches(
            conn,
            now,
            DUE_REMINDER_BATCH_SIZE,
        )?)
    })
    .await?;
    for row in &rows {
        enqueue_dispatch(
            jobs,
            Enqueue {
                workspace_id: row.workspace_id,
                appointment_id: row.appointment_id,
                dispatch_id: row.id,
                reminder_config_id: row.reminder_config_id,
                send_at: row.send_at,
                job_id: row.job_id.clone(),
            },
        )
        .await;
    }
    Ok((rows.len(), rows.len()))
}

/// The outcome of [`send_dispatch`] (`reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    NotPending,
    StaleAppointment,
    MissingConversation,
    MissingContactInbox,
}

/// The `sendFlow` job of a due reminder, from inside the send transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderFlow {
    pub job_id: String,
    pub data: serde_json::Value,
}

/// `sendDispatch` checks (in one write): the `sendFlow` job to add and the
/// outcome. The caller adds the job, then calls [`mark_sent`].
pub fn prepare_send(
    conn: &Connection,
    workspace_id: i64,
    appointment_id: i64,
    dispatch_id: i64,
    reminder_config_id: i64,
    now: i64,
) -> Result<(SendOutcome, Option<ReminderFlow>)> {
    let Some(dispatch) = repo::find_dispatch_for_send(
        conn,
        workspace_id,
        dispatch_id,
        appointment_id,
        reminder_config_id,
    )?
    else {
        return Ok((SendOutcome::NotPending, None));
    };
    if dispatch.status != "pending" {
        return Ok((SendOutcome::NotPending, None));
    }
    let appointment =
        repo::find_by(conn, workspace_id, appointment_id, true)?.ok_or_else(|| {
            BusinessError::Repo(chatbotx_db::repo::Error::msg("Appointment not found"))
        })?;
    if appointment.status != "scheduled" || appointment.start_at <= now {
        repo::mark_dispatches_cancelled(conn, workspace_id, appointment.id, &[dispatch.id])?;
        return Ok((SendOutcome::StaleAppointment, None));
    }
    let Some(conversation_id) = appointment.conversation_id else {
        repo::mark_dispatch_failed(
            conn,
            workspace_id,
            dispatch.id,
            Some("Appointment has no conversation for reminder flow"),
        )?;
        return Ok((SendOutcome::MissingConversation, None));
    };
    let contact_inbox_id: Option<i64> = match dispatch.contact_inbox_id {
        Some(id) => Some(id),
        None => conn
            .query_row(
                "SELECT ci.\"id\" FROM \"ContactInbox\" ci JOIN \"Conversation\" c ON c.\"contactId\" = ci.\"contactId\" \
                 WHERE c.\"id\" = ?1 ORDER BY ci.\"id\" LIMIT 1",
                [conversation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?,
    };
    let Some(contact_inbox_id) = contact_inbox_id else {
        repo::mark_dispatch_failed(
            conn,
            workspace_id,
            dispatch.id,
            Some("Appointment contact has no contact inbox"),
        )?;
        return Ok((SendOutcome::MissingContactInbox, None));
    };
    let flow_id: i64 = conn
        .query_row(
            "SELECT \"flowId\" FROM \"AppointmentCalendarReminder\" WHERE \"id\" = ?1",
            [dispatch.reminder_config_id],
            |r| r.get(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok((
        SendOutcome::Sent,
        Some(ReminderFlow {
            job_id: format!("appt-reminder-flow-{}", dispatch.id),
            data: json!({
                "type": "sendFlow",
                "data": {
                    "conversationId": conversation_id.to_string(),
                    "contactInboxId": contact_inbox_id.to_string(),
                    "flowId": flow_id.to_string(),
                    "origin": "channel",
                    "appointmentId": appointment_id.to_string(),
                },
            }),
        }),
    ))
}

/// `sendDispatch`: checks the dispatch, adds the reminder flow job, marks it sent.
pub async fn send_dispatch(
    db: &Db,
    jobs: &dyn Enqueuer,
    workspace_id: i64,
    appointment_id: i64,
    dispatch_id: i64,
    reminder_config_id: i64,
) -> Result<SendOutcome> {
    let now = now_ms();
    let (outcome, flow) = tx::write(db, move |conn| {
        prepare_send(
            conn,
            workspace_id,
            appointment_id,
            dispatch_id,
            reminder_config_id,
            now,
        )
    })
    .await?;
    let Some(flow) = flow else {
        return Ok(outcome);
    };
    jobs.add(
        QueueName::Integration,
        "sendFlow",
        flow.data,
        JobOptions {
            job_id: Some(flow.job_id),
            ..Default::default()
        },
    )
    .await
    .map_err(|error| BusinessError::Repo(chatbotx_db::repo::Error::msg(error.to_string())))?;
    tx::write(db, move |conn| {
        repo::mark_dispatch_sent(conn, workspace_id, dispatch_id)?;
        Ok(())
    })
    .await?;
    Ok(SendOutcome::Sent)
}

/// `markFailed` (the reason is cut at 500 characters).
pub async fn mark_failed(db: &Db, workspace_id: i64, id: i64, error: &str) -> Result<()> {
    let reason: String = error.chars().take(500).collect();
    tx::write(db, move |conn| {
        repo::mark_dispatch_failed(conn, workspace_id, id, Some(&reason))?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_at_subtracts_minutes_and_hours() {
        assert_eq!(
            calculate_send_at(10_000_000, 10, "minutes", "UTC"),
            10_000_000 - 600_000
        );
        assert_eq!(
            calculate_send_at(10_000_000, 1, "hours", "UTC"),
            10_000_000 - 3_600_000
        );
    }

    #[test]
    fn send_at_days_keep_the_local_wall_clock_across_dst() {
        use chatbotx_core::utils::tz::from_zoned_time;
        // 2026-03-09 10:00 New York (EDT) minus 1 day = 2026-03-08 10:00 (EDT, DST began 02:00).
        let start = from_zoned_time("2026-03-09T10:00:00.000", "America/New_York").unwrap();
        let want = from_zoned_time("2026-03-08T10:00:00.000", "America/New_York").unwrap();
        assert_eq!(
            calculate_send_at(start, 1, "days", "America/New_York"),
            want
        );
        let start = from_zoned_time("2026-03-08T10:00:00.000", "America/New_York").unwrap();
        let want = from_zoned_time("2026-03-07T10:00:00.000", "America/New_York").unwrap();
        assert_eq!(
            calculate_send_at(start, 1, "days", "America/New_York"),
            want
        );
        assert_ne!(want, start - 86_400_000);
    }
}
