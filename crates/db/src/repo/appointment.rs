//! Port of `repositories/appointment/repository.ts` and
//! `repositories/appointment-reminder-dispatch/repository.ts`. Ids are new
//! snowflake ids ([`new_id`]) where the TS used `createId()`.

use rusqlite::{Connection, ToSql};

use super::models::{Appointment, AppointmentReminderDispatch};
use super::utils::{
    PaginationInput, SortDir, SortItem, get_pagination_with_defaults, like_contains, order_by_sql,
    parse_order_by,
};
use super::{Model, Result, find_many, find_one, id_list, new_id, now_ms, returning};

/// `AppointmentListInput` (`tab`: `next` or `past`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppointmentListInput {
    pub workspace_id: i64,
    pub calendar_id: Option<i64>,
    pub search: Option<String>,
    pub past: bool,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub now_ms: i64,
}

/// One `list` row: the appointment with its calendar name and contact names.
#[derive(Debug, Clone, PartialEq)]
pub struct AppointmentListRow {
    pub appointment: Appointment,
    pub calendar_name: String,
    pub contact_avatar: Option<String>,
    pub contact_first_name: Option<String>,
    pub contact_last_name: Option<String>,
    pub contact_full_name: Option<String>,
}

const LIST_FROM: &str = "FROM \"Appointment\" a \
     JOIN \"AppointmentCalendar\" c ON c.\"id\" = a.\"calendarId\" \
     JOIN \"Contact\" t ON t.\"id\" = a.\"contactId\"";

/// `list`: one page by `startAt` desc (`pageCount`, total).
pub fn list(
    conn: &Connection,
    input: &AppointmentListInput,
) -> Result<(Vec<AppointmentListRow>, i64, i64)> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut filter = String::from(
        "a.\"workspaceId\" = ?1 AND a.\"deletedAt\" IS NULL AND (?2 IS NULL OR a.\"calendarId\" = ?2)",
    );
    filter.push_str(if input.past {
        " AND a.\"startAt\" <= ?3"
    } else {
        " AND a.\"startAt\" > ?3"
    });
    let search = input
        .search
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(like_contains);
    if search.is_some() {
        filter.push_str(
            " AND (ilike(t.\"fullName\", ?4) OR ilike(t.\"firstName\", ?4) OR ilike(t.\"lastName\", ?4))",
        );
    }
    let params: [&dyn ToSql; 4] = [
        &input.workspace_id,
        &input.calendar_id,
        &input.now_ms,
        &search,
    ];
    let count = if search.is_some() { 4 } else { 3 };
    let sql = format!(
        "SELECT {}, c.\"name\", t.\"avatar\", t.\"firstName\", t.\"lastName\", t.\"fullName\" {LIST_FROM} \
         WHERE {filter} ORDER BY a.\"startAt\" DESC LIMIT {} OFFSET {}",
        Appointment::columns_sql(Some("a")),
        pagination.limit,
        pagination.offset
    );
    let width = Appointment::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(&params[..count], |r| {
            Ok(AppointmentListRow {
                appointment: Appointment::from_row(r)?,
                calendar_name: r.get(width)?,
                contact_avatar: r.get(width + 1)?,
                contact_first_name: r.get(width + 2)?,
                contact_last_name: r.get(width + 3)?,
                contact_full_name: r.get(width + 4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) {LIST_FROM} WHERE {filter}"),
        &params[..count],
        |r| r.get(0),
    )?;
    Ok((
        rows,
        (total + pagination.limit - 1) / pagination.limit,
        total,
    ))
}

/// `findBy` (the row only; callers load the relations they need).
pub fn find_by(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    include_deleted: bool,
) -> Result<Option<Appointment>> {
    let deleted = if include_deleted {
        ""
    } else {
        " AND \"deletedAt\" IS NULL"
    };
    find_one(
        conn,
        &format!("\"id\" = ?1 AND \"workspaceId\" = ?2{deleted}"),
        &[&id, &workspace_id],
    )
}

/// `findLatestForContact`: the newest scheduled one.
pub fn find_latest_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<Appointment>> {
    Ok(find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"contactId\" = ?2 AND \"status\" = 'scheduled' \
         AND \"deletedAt\" IS NULL ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT 1",
        &[&workspace_id, &contact_id],
    )?
    .pop())
}

/// `CreateAppointmentInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateAppointment {
    pub workspace_id: i64,
    pub calendar_id: i64,
    pub contact_id: i64,
    pub conversation_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
    pub start_at: i64,
    pub end_at: i64,
    pub invitee_timezone: String,
    pub location_type: String,
    pub location_detail: Option<String>,
    pub external_sync_status: Option<String>,
    pub external_event_integration_id: Option<i64>,
    pub external_event_provider_calendar_id: Option<String>,
}

/// `create`.
pub fn create(conn: &Connection, input: &CreateAppointment) -> Result<Appointment> {
    let mut rows = returning::<Appointment>(
        conn,
        "INSERT INTO \"Appointment\" (\"id\", \"workspaceId\", \"calendarId\", \"contactId\", \
         \"conversationId\", \"contactInboxId\", \"startAt\", \"endAt\", \"inviteeTimezone\", \
         \"locationType\", \"locationDetail\", \"externalSyncStatus\", \"externalEventIntegrationId\", \
         \"externalEventProviderCalendarId\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        &[
            &new_id(),
            &input.workspace_id,
            &input.calendar_id,
            &input.contact_id,
            &input.conversation_id,
            &input.contact_inbox_id,
            &input.start_at,
            &input.end_at,
            &input.invitee_timezone,
            &input.location_type,
            &input.location_detail,
            &input.external_sync_status,
            &input.external_event_integration_id,
            &input.external_event_provider_calendar_id,
        ],
    )?;
    rows.pop()
        .ok_or_else(|| super::Error::msg("appointment insert returned no row"))
}

/// `listFutureScheduledForContact`: by `startAt` asc.
pub fn list_future_scheduled_for_contact(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: i64,
    now: i64,
) -> Result<Vec<Appointment>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"calendarId\" = ?2 AND \"contactId\" = ?3 \
         AND \"status\" = 'scheduled' AND \"startAt\" > ?4 AND \"deletedAt\" IS NULL ORDER BY \"startAt\" ASC",
        &[&workspace_id, &calendar_id, &contact_id, &now],
    )
}

/// `existsScheduledForContactAt`.
pub fn exists_scheduled_for_contact_at(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: i64,
    start_at: i64,
) -> Result<bool> {
    Ok(find_one::<Appointment>(
        conn,
        "\"workspaceId\" = ?1 AND \"calendarId\" = ?2 AND \"contactId\" = ?3 \
         AND \"status\" = 'scheduled' AND \"startAt\" = ?4 AND \"deletedAt\" IS NULL",
        &[&workspace_id, &calendar_id, &contact_id, &start_at],
    )?
    .is_some())
}

/// `listFutureScheduledByCalendar`: by `startAt` asc.
pub fn list_future_scheduled_by_calendar(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    now: i64,
) -> Result<Vec<Appointment>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"calendarId\" = ?2 AND \"status\" = 'scheduled' \
         AND \"startAt\" > ?3 AND \"deletedAt\" IS NULL ORDER BY \"startAt\" ASC",
        &[&workspace_id, &calendar_id, &now],
    )
}

/// The scheduled, live `startAt`s of a calendar in `[from, to]`.
pub fn scheduled_start_ats(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    from: i64,
    to: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"startAt\" FROM \"Appointment\" WHERE \"workspaceId\" = ?1 AND \"calendarId\" = ?2 \
         AND \"status\" = 'scheduled' AND \"startAt\" >= ?3 AND \"startAt\" <= ?4 AND \"deletedAt\" IS NULL",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, calendar_id, from, to],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// The count of a contact's future scheduled appointments on a calendar
/// (`tx.$count` of `generateAvailableSlots`; no `deletedAt` filter, as the TS).
pub fn count_future_scheduled_for_contact(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    contact_id: i64,
    now: i64,
) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"Appointment\" WHERE \"workspaceId\" = ?1 AND \"calendarId\" = ?2 \
         AND \"contactId\" = ?3 AND \"status\" = 'scheduled' AND \"startAt\" >= ?4",
        rusqlite::params![workspace_id, calendar_id, contact_id, now],
        |r| r.get(0),
    )?)
}

/// One `listByContact` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactAppointmentRow {
    pub appointment: Appointment,
    pub calendar_name: String,
}

/// `listByContact`: upcoming first (soonest), then past (newest).
pub fn list_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    limit: i64,
    now: i64,
) -> Result<Vec<ContactAppointmentRow>> {
    let sql = format!(
        "SELECT {}, c.\"name\" FROM \"Appointment\" a JOIN \"AppointmentCalendar\" c ON c.\"id\" = a.\"calendarId\" \
         WHERE a.\"workspaceId\" = ?1 AND a.\"contactId\" = ?2 AND a.\"deletedAt\" IS NULL \
         ORDER BY CASE WHEN a.\"startAt\" >= ?3 THEN 0 ELSE 1 END, \
         CASE WHEN a.\"startAt\" >= ?3 THEN a.\"startAt\" END ASC, \
         CASE WHEN a.\"startAt\" < ?3 THEN a.\"startAt\" END DESC LIMIT ?4",
        Appointment::columns_sql(Some("a"))
    );
    let width = Appointment::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, contact_id, now, limit],
            |r| {
                Ok(ContactAppointmentRow {
                    appointment: Appointment::from_row(r)?,
                    calendar_name: r.get(width)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `cancelScheduled`: a live, scheduled, future row becomes `cancelled`.
/// `external_sync_status` `None` keeps the column (drizzle skips undefined).
pub fn cancel_scheduled(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    cancelled_at: i64,
    external_sync_status: Option<&str>,
) -> Result<Option<Appointment>> {
    Ok(returning::<Appointment>(
        conn,
        "UPDATE \"Appointment\" SET \"status\" = 'cancelled', \"cancelledAt\" = ?3, \
         \"externalSyncStatus\" = coalesce(?4, \"externalSyncStatus\"), \"updatedAt\" = ?5 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'scheduled' AND \"startAt\" > ?5 \
         AND \"deletedAt\" IS NULL",
        &[
            &id,
            &workspace_id,
            &cancelled_at,
            &external_sync_status,
            &now_ms(),
        ],
    )?
    .pop())
}

/// `softDelete`.
pub fn soft_delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Appointment>> {
    Ok(returning::<Appointment>(
        conn,
        "UPDATE \"Appointment\" SET \"deletedAt\" = ?3, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"deletedAt\" IS NULL",
        &[&id, &workspace_id, &now_ms()],
    )?
    .pop())
}

/// The external-sync writes (`markExternal*`, `persistExternalDestinationIfScheduled`):
/// `set` (SQL with `?4` / `?5`) on a live row whose `status` is `when_status`.
fn update_when(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    when_status: &str,
    set: &str,
    a: &dyn ToSql,
    b: &dyn ToSql,
) -> Result<Option<Appointment>> {
    Ok(returning::<Appointment>(
        conn,
        &format!(
            "UPDATE \"Appointment\" SET {set}, \"updatedAt\" = ?6 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 \
             AND \"status\" = ?3 AND \"deletedAt\" IS NULL"
        ),
        &[&id, &workspace_id, &when_status, a, b, &now_ms()],
    )?
    .pop())
}

/// `markExternalCreateFailedIfScheduled` / `markExternalCancel*IfCancelled`.
pub fn set_external_sync_status(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    when_status: &str,
    status: &str,
) -> Result<Option<Appointment>> {
    update_when(
        conn,
        workspace_id,
        id,
        when_status,
        "\"externalSyncStatus\" = ?4",
        &status,
        &None::<i64>,
    )
}

/// `markExternalCreateSucceededIfScheduled`.
pub fn mark_external_create_synced(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    external_event_id: &str,
) -> Result<Option<Appointment>> {
    update_when(
        conn,
        workspace_id,
        id,
        "scheduled",
        "\"externalSyncStatus\" = 'synced', \"externalEventId\" = ?4",
        &external_event_id,
        &None::<i64>,
    )
}

/// `persistExternalDestinationIfScheduled`.
pub fn persist_external_destination(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    integration_id: i64,
    provider_calendar_id: &str,
) -> Result<Option<Appointment>> {
    update_when(
        conn,
        workspace_id,
        id,
        "scheduled",
        "\"externalEventProviderCalendarId\" = ?4, \"externalEventIntegrationId\" = ?5",
        &provider_calendar_id,
        &integration_id,
    )
}

// ---- reminder dispatches ---------------------------------------------------

/// `appointmentReminderDispatchRepository.list` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchListInput {
    pub workspace_id: Option<i64>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `list`: one page (sort, else `sendAt` asc); `(rows, pageCount, total)`.
pub fn list_dispatches(
    conn: &Connection,
    input: &DispatchListInput,
) -> Result<(Vec<AppointmentReminderDispatch>, i64, i64)> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut order = parse_order_by(
        AppointmentReminderDispatch::COLUMNS,
        input.sort.as_deref(),
        None,
    );
    if order.is_empty() {
        order = vec![("sendAt".to_owned(), SortDir::Asc)];
    }
    let filter = "(?1 IS NULL OR \"workspaceId\" = ?1) AND (?2 IS NULL OR \"status\" = ?2)";
    let params: [&dyn ToSql; 2] = [&input.workspace_id, &input.status];
    let rows = super::query_models(
        conn,
        &format!(
            "{} WHERE {filter} {} LIMIT {} OFFSET {}",
            AppointmentReminderDispatch::select_sql(),
            order_by_sql(&order, None),
            pagination.limit,
            pagination.offset
        ),
        &params,
    )?;
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM \"AppointmentReminderDispatch\" WHERE {filter}"),
        &params[..],
        |r| r.get(0),
    )?;
    Ok((
        rows,
        (total + pagination.limit - 1) / pagination.limit,
        total,
    ))
}

/// `createPending`: insert, or the row that already holds `job_id`.
pub fn create_pending_dispatch(
    conn: &Connection,
    workspace_id: i64,
    appointment_id: i64,
    reminder_config_id: i64,
    contact_inbox_id: Option<i64>,
    send_at: i64,
    job_id: &str,
) -> Result<Option<AppointmentReminderDispatch>> {
    let inserted = returning::<AppointmentReminderDispatch>(
        conn,
        "INSERT INTO \"AppointmentReminderDispatch\" (\"id\", \"workspaceId\", \"appointmentId\", \
         \"reminderConfigId\", \"contactInboxId\", \"sendAt\", \"status\", \"jobId\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7) ON CONFLICT (\"jobId\") DO NOTHING",
        &[
            &new_id(),
            &workspace_id,
            &appointment_id,
            &reminder_config_id,
            &contact_inbox_id,
            &send_at,
            &job_id,
        ],
    )?;
    if let Some(row) = inserted.into_iter().next() {
        return Ok(Some(row));
    }
    find_one(conn, "\"jobId\" = ?1", &[&job_id])
}

/// `findForSend` (the dispatch row; the caller loads the relations).
pub fn find_dispatch_for_send(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    appointment_id: i64,
    reminder_config_id: i64,
) -> Result<Option<AppointmentReminderDispatch>> {
    find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"appointmentId\" = ?3 AND \"reminderConfigId\" = ?4",
        &[&id, &workspace_id, &appointment_id, &reminder_config_id],
    )
}

/// `listPendingJobIdsForFutureCalendar`.
pub fn list_pending_job_ids_for_future_calendar(
    conn: &Connection,
    workspace_id: i64,
    calendar_id: i64,
    now: i64,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT d.\"jobId\" FROM \"AppointmentReminderDispatch\" d JOIN \"Appointment\" a ON a.\"id\" = d.\"appointmentId\" \
         WHERE d.\"workspaceId\" = ?1 AND a.\"workspaceId\" = ?1 AND a.\"calendarId\" = ?2 AND a.\"status\" = 'scheduled' \
         AND a.\"startAt\" > ?3 AND a.\"deletedAt\" IS NULL AND d.\"status\" = 'pending' ORDER BY a.\"startAt\" ASC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, calendar_id, now], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

/// `markSent`: a pending row becomes `sent`.
pub fn mark_dispatch_sent(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    let now = now_ms();
    Ok(conn.execute(
        "UPDATE \"AppointmentReminderDispatch\" SET \"status\" = 'sent', \"sentAt\" = ?3, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'pending'",
        rusqlite::params![id, workspace_id, now],
    )? > 0)
}

/// `markCancelledByAppointment` (only `dispatch_ids` when not empty).
pub fn mark_dispatches_cancelled(
    conn: &Connection,
    workspace_id: i64,
    appointment_id: i64,
    dispatch_ids: &[i64],
) -> Result<Vec<AppointmentReminderDispatch>> {
    let ids = (!dispatch_ids.is_empty()).then(|| id_list(dispatch_ids));
    returning(
        conn,
        "UPDATE \"AppointmentReminderDispatch\" SET \"status\" = 'cancelled', \"cancelledAt\" = ?4, \"updatedAt\" = ?4 \
         WHERE \"workspaceId\" = ?1 AND \"appointmentId\" = ?2 AND \"status\" = 'pending' \
         AND (?3 IS NULL OR \"id\" IN (SELECT value FROM json_each(?3)))",
        &[&workspace_id, &appointment_id, &ids, &now_ms()],
    )
}

/// `listDuePending`.
pub fn list_due_pending_dispatches(
    conn: &Connection,
    now: i64,
    limit: i64,
) -> Result<Vec<AppointmentReminderDispatch>> {
    find_many(
        conn,
        "WHERE \"status\" = 'pending' AND \"sendAt\" <= ?1 LIMIT ?2",
        &[&now, &limit],
    )
}

/// `markFailedForRetry`.
pub fn mark_dispatch_failed(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    failed_reason: Option<&str>,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE \"AppointmentReminderDispatch\" SET \"status\" = 'failed', \"failedReason\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id, failed_reason, now_ms()],
    )? > 0)
}
