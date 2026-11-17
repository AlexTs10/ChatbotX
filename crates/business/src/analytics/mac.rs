//! `macTrackingService.claimNewActiveContact(s)` and `anchoredPeriod`: the
//! write-time MAC ledger claim of brand-new contacts: the
//! `ContactActiveMonthly` and `ContactActiveHourly` presence rows and the
//! `WorkspaceMac` rollup. No Redis caches: the database rows are the only
//! state.

use chatbotx_db::repo;
use chatbotx_db::rusqlite::{Connection, params};
use chrono::{DateTime, Datelike, Months, Timelike, Utc};

const HOUR_MS: i64 = 3_600_000;

/// `addMonthsClampedUtc`: `anchor + months`, the day clamped to the target
/// month's last day (Postgres `+ interval 'N month'`).
fn add_months(anchor: DateTime<Utc>, months: i32) -> Option<DateTime<Utc>> {
    if months >= 0 {
        anchor.checked_add_months(Months::new(months as u32))
    } else {
        anchor.checked_sub_months(Months::new(months.unsigned_abs()))
    }
}

/// `anchoredPeriod`: the half-open billing period `[start, end)` that holds
/// `occurred_at`, anchored to the minute of `period_start` (ms).
pub fn anchored_period(occurred_at: i64, period_start: i64) -> Option<(i64, i64)> {
    let anchor = DateTime::from_timestamp_millis(period_start)?
        .with_second(0)?
        .with_nanosecond(0)?;
    let at = DateTime::from_timestamp_millis(occurred_at)?;
    let mut months = (at.year() - anchor.year()) * 12 + at.month() as i32 - anchor.month() as i32;
    let mut start = add_months(anchor, months)?;
    while start > at {
        months -= 1;
        start = add_months(anchor, months)?;
    }
    let mut end = add_months(anchor, months + 1)?;
    while end <= at {
        months += 1;
        start = end;
        end = add_months(anchor, months + 1)?;
    }
    Some((start.timestamp_millis(), end.timestamp_millis()))
}

/// `macRepository.ensureWorkspaceMac`: the `WorkspaceMac` row of the period
/// (created with the workspace's owner and tenant). `None` for an unknown
/// workspace.
fn ensure_workspace_mac(
    conn: &Connection,
    workspace_id: i64,
    (start, end): (i64, i64),
) -> repo::Result<Option<i64>> {
    let id: i64 = chatbotx_core::utils::id::create_id()
        .parse()
        .unwrap_or_default();
    let mut stmt = conn.prepare(
        r#"INSERT INTO "WorkspaceMac" ("id", "workspaceId", "ownerId", "tenantId", "periodStart", "periodEnd", "macCount")
           SELECT ?1, "id", "ownerId", "tenantId", ?3, ?4, 0 FROM "Workspace" WHERE "id" = ?2
           ON CONFLICT ("workspaceId", "periodStart", "periodEnd") DO UPDATE SET
             "ownerId" = COALESCE("WorkspaceMac"."ownerId", excluded."ownerId"),
             "tenantId" = COALESCE("WorkspaceMac"."tenantId", excluded."tenantId"),
             "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
           RETURNING "id""#,
    )?;
    let mut rows = stmt.query(params![id, workspace_id, start, end])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// `claimNewActiveContacts`, inside the caller's write transaction:
/// `contacts` are `(contactId, contactInboxId)` of one inbox. Returns the
/// monthly presence rows newly inserted (a contact inbox already active this
/// period is not counted again); the hourly rows are written either way.
pub fn claim_new_active_contacts(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    period_start: i64,
    occurred_at: i64,
    contacts: &[(i64, i64)],
) -> repo::Result<i64> {
    if contacts.is_empty() {
        return Ok(0);
    }
    let Some(period) = anchored_period(occurred_at, period_start) else {
        return Ok(0);
    };
    let Some(mac_id) = ensure_workspace_mac(conn, workspace_id, period)? else {
        return Ok(0);
    };
    let hour_bucket = occurred_at - occurred_at.rem_euclid(HOUR_MS);
    let mut counted = 0;
    for &(contact_id, contact_inbox_id) in contacts {
        counted += conn.execute(
            r#"INSERT INTO "ContactActiveMonthly" ("workspaceId", "contactId", "contactInboxId", "periodStart", "inboxId", "workspaceMacId")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING"#,
            params![workspace_id, contact_id, contact_inbox_id, period.0, inbox_id, mac_id],
        )? as i64;
        conn.execute(
            r#"INSERT INTO "ContactActiveHourly" ("workspaceId", "contactId", "contactInboxId", "hourBucket", "inboxId")
               VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
            params![workspace_id, contact_id, contact_inbox_id, hour_bucket, inbox_id],
        )?;
    }
    if counted > 0 {
        conn.execute(
            r#"UPDATE "WorkspaceMac" SET "macCount" = "macCount" + ?1,
               "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) WHERE "id" = ?2"#,
            params![counted, mac_id],
        )?;
    }
    Ok(counted)
}

/// `claimNewActiveContact`: [`claim_new_active_contacts`] of one contact;
/// `true` when it was counted.
pub fn claim_new_active_contact(
    conn: &Connection,
    workspace_id: i64,
    contact: (i64, i64),
    inbox_id: i64,
    period_start: i64,
    occurred_at: i64,
) -> repo::Result<bool> {
    Ok(claim_new_active_contacts(
        conn,
        workspace_id,
        inbox_id,
        period_start,
        occurred_at,
        &[contact],
    )? > 0)
}

#[cfg(test)]
mod tests {
    use super::anchored_period;

    fn ms(iso: &str) -> i64 {
        chatbotx_core::js::parse_date(iso).unwrap()
    }

    fn iso(ms: i64) -> String {
        chatbotx_core::js::to_iso_string(ms).unwrap()
    }

    /// `mac-period.test.ts` `anchoredPeriod`.
    #[test]
    fn anchored_period_cases() {
        for (created, at, start, end) in [
            (
                "2026-01-15T12:30:45.000Z",
                "2026-01-20T08:00:00.000Z",
                "2026-01-15T12:30:00.000Z",
                "2026-02-15T12:30:00.000Z",
            ),
            (
                "2026-01-15T12:30:00.000Z",
                "2026-02-10T00:00:00.000Z",
                "2026-01-15T12:30:00.000Z",
                "2026-02-15T12:30:00.000Z",
            ),
            (
                "2026-01-31T12:30:00.000Z",
                "2026-02-05T00:00:00.000Z",
                "2026-01-31T12:30:00.000Z",
                "2026-02-28T12:30:00.000Z",
            ),
            (
                "2026-01-31T12:30:00.000Z",
                "2026-03-01T00:00:00.000Z",
                "2026-02-28T12:30:00.000Z",
                "2026-03-31T12:30:00.000Z",
            ),
            (
                "2026-01-31T12:30:00.000Z",
                "2026-03-31T12:30:00.000Z",
                "2026-03-31T12:30:00.000Z",
                "2026-04-30T12:30:00.000Z",
            ),
            (
                "2024-01-31T00:00:00.000Z",
                "2024-02-15T00:00:00.000Z",
                "2024-01-31T00:00:00.000Z",
                "2024-02-29T00:00:00.000Z",
            ),
            (
                "2025-12-20T09:00:00.000Z",
                "2026-01-10T00:00:00.000Z",
                "2025-12-20T09:00:00.000Z",
                "2026-01-20T09:00:00.000Z",
            ),
            (
                "2026-01-10T09:00:00.000Z",
                "2026-08-25T00:00:00.000Z",
                "2026-08-10T09:00:00.000Z",
                "2026-09-10T09:00:00.000Z",
            ),
            (
                "2026-01-10T09:00:00.000Z",
                "2026-01-10T09:00:00.000Z",
                "2026-01-10T09:00:00.000Z",
                "2026-02-10T09:00:00.000Z",
            ),
        ] {
            let (s, e) = anchored_period(ms(at), ms(created)).unwrap();
            assert_eq!(
                (iso(s), iso(e)),
                (start.to_string(), end.to_string()),
                "{created} {at}"
            );
        }
    }
}
