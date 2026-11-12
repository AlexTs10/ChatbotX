//! Port of `repositories/automation-throttle/repository.ts`: atomic claim / release /
//! purge of `AutomationThrottle` rows. `now` (unix ms) replaces SQL `now()`.

use rusqlite::Connection;

use super::Result;

/// Retention of a stale throttle row (`STALE_RETENTION_HOURS`).
pub const STALE_RETENTION_HOURS: i64 = 48;

/// `ThrottleSubject`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThrottleSubject {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    /// `defaultReply`.
    pub throttle_type: String,
    pub subject_id: i64,
}

/// `ThrottleClaimOutcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThrottleClaimOutcome {
    Won { claim_id: String, remaining_seconds: i64 },
    Denied { remaining_seconds: i64 },
}

/// `greatest(0, ceil((lastTriggeredAt + window - now) / 1s))`.
fn remaining_sql(window_seconds: i64, now: i64) -> String {
    format!(
        "max(0, CAST(ceil((\"lastTriggeredAt\" + {} - {now}) / 1000.0) AS INTEGER))",
        window_seconds * 1000
    )
}

/// `claimAutomationThrottle`: inserts the claim, or takes over the row when its window
/// elapsed (`window_seconds = 0`: always). Denied -> the remaining seconds of the holder
/// (0 when the row vanished meanwhile).
pub fn claim_automation_throttle(
    conn: &Connection,
    subject: &ThrottleSubject,
    window_seconds: i64,
    claim_id: &str,
    now: i64,
) -> Result<ThrottleClaimOutcome> {
    todo!("P3.2");
    let set_where = if window_seconds == 0 {
        String::new()
    } else {
        format!(
            " WHERE \"AutomationThrottle\".\"lastTriggeredAt\" <= {}",
            now - window_seconds * 1000
        )
    };
    let sql = format!(
        "INSERT INTO \"AutomationThrottle\" (\"workspaceId\", \"contactInboxId\", \"throttleType\", \"subjectId\", \"lastTriggeredAt\", \"claimId\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (\"workspaceId\", \"contactInboxId\", \"throttleType\", \"subjectId\") \
         DO UPDATE SET \"lastTriggeredAt\" = excluded.\"lastTriggeredAt\", \"claimId\" = excluded.\"claimId\"{set_where} \
         RETURNING {}",
        remaining_sql(window_seconds, now)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params![
        subject.workspace_id,
        subject.contact_inbox_id,
        subject.throttle_type,
        subject.subject_id,
        now,
        claim_id
    ])?;
    if let Some(row) = rows.next()? {
        return Ok(ThrottleClaimOutcome::Won {
            claim_id: claim_id.to_owned(),
            remaining_seconds: row.get(0)?,
        });
    }
    drop(rows);
    let sql = format!(
        "SELECT {} FROM \"AutomationThrottle\" WHERE \"workspaceId\" = ?1 AND \"contactInboxId\" = ?2 AND \"throttleType\" = ?3 AND \"subjectId\" = ?4",
        remaining_sql(window_seconds, now)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params![
        subject.workspace_id,
        subject.contact_inbox_id,
        subject.throttle_type,
        subject.subject_id
    ])?;
    let remaining_seconds = match rows.next()? {
        Some(row) => row.get(0)?,
        None => 0,
    };
    Ok(ThrottleClaimOutcome::Denied { remaining_seconds })
}

/// `releaseAutomationThrottle`: deletes the row only while `claim_id` still owns it.
pub fn release_automation_throttle(
    conn: &Connection,
    subject: &ThrottleSubject,
    claim_id: &str,
) -> Result<bool> {
    todo!("P3.2");
    let n = conn.execute(
        "DELETE FROM \"AutomationThrottle\" WHERE \"workspaceId\" = ?1 AND \"contactInboxId\" = ?2 AND \"throttleType\" = ?3 AND \"subjectId\" = ?4 AND \"claimId\" = ?5",
        rusqlite::params![
            subject.workspace_id,
            subject.contact_inbox_id,
            subject.throttle_type,
            subject.subject_id,
            claim_id
        ],
    )?;
    Ok(n > 0)
}

/// `purgeStaleAutomationThrottles`: deletes rows last triggered more than
/// `older_than_hours` ago; returns the deleted count.
pub fn purge_stale_automation_throttles(conn: &Connection, older_than_hours: i64, now: i64) -> Result<usize> {
    todo!("P3.2");
    let cutoff = now - older_than_hours * 3_600_000;
    Ok(conn.execute(
        "DELETE FROM \"AutomationThrottle\" WHERE \"lastTriggeredAt\" < ?1",
        [cutoff],
    )?)
}
