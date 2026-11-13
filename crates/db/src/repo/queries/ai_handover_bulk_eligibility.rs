//! Port of `queries/ai-handover-bulk-eligibility.ts`: the one definition of "this thread
//! is touched by a bulk AI hand-over run", shared by the page query and the count.
//!
//! Columns are qualified with the unaliased `"ContactInbox"` table.

use rusqlite::types::Value;

use crate::repo::partials::thread_control::THREAD_IDLE_AFTER_MS;
use crate::repo::sql::Sql;

/// `AiHandoverBulkAction` (`partials/ai-handover`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiHandoverBulkAction {
    Enable,
    Disable,
}

/// `BulkEligibilityInput` (instants in Unix ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkEligibilityInput {
    pub action: AiHandoverBulkAction,
    /// Run's request time: a thread changed after it is respected and skipped.
    pub requested_at: i64,
    pub now: i64,
    /// Disable: the channel's takeover-message window; enable: the activity window.
    pub last_incoming_since: i64,
}

/// `"ContactInbox".` column prefix.
const CI: &str = "\"ContactInbox\".";

/// `isHeldStandby`: the SQL form of `resolveThreadControlState(..) === "standby"`. SQLite
/// `max(a, b)` is NULL when either is NULL, so the `COALESCE` falls back to the other
/// column (Postgres `GREATEST` ignores NULLs); with no reference time at all the stored
/// state is kept (`'infinity'` = `i64::MAX`).
fn is_held_standby(now: i64) -> Sql {
    let idle_before = now - THREAD_IDLE_AFTER_MS;
    Sql::new(
        format!(
            "({CI}\"threadControlState\" = 'standby' AND CASE \
             WHEN {CI}\"threadOwnerExpiresAt\" IS NOT NULL THEN {CI}\"threadOwnerExpiresAt\" > ? \
             ELSE COALESCE(max({CI}\"lastIncomingMessageAt\", {CI}\"threadControlUpdatedAt\"), \
             {CI}\"lastIncomingMessageAt\", {CI}\"threadControlUpdatedAt\", {}) > ? END)",
            i64::MAX
        ),
        vec![Value::Integer(now), Value::Integer(idle_before)],
    )
}

/// `notNewerThanRequest`.
fn not_newer_than_request(requested_at: i64) -> Sql {
    Sql::bind(
        format!("{CI}\"threadControlUpdatedAt\" IS NULL OR {CI}\"threadControlUpdatedAt\" <= "),
        requested_at,
    )
}

/// `bulkEligibilityConditions`.
pub fn bulk_eligibility_conditions(input: &BulkEligibilityInput) -> Vec<Sql> {
    let mut out = vec![
        Sql::raw(format!("{CI}\"sourceId\" IS NOT NULL")),
        Sql::bind(
            format!("{CI}\"lastIncomingMessageAt\" >= "),
            input.last_incoming_since,
        ),
        not_newer_than_request(input.requested_at),
    ];
    match input.action {
        AiHandoverBulkAction::Disable => {
            out.push(Sql::bind(
                format!("{CI}\"threadControlState\" = "),
                "standby".to_owned(),
            ));
            out.push(Sql::bind(
                format!("{CI}\"threadOwnerRole\" = "),
                "ai_agent".to_owned(),
            ));
            out.push(is_held_standby(input.now));
        }
        AiHandoverBulkAction::Enable => {
            // COALESCE: a never-observed thread has a NULL state, which makes the
            // held-standby test NULL, and `NOT NULL` is NULL (the row would be dropped).
            let mut q = Sql::raw("NOT COALESCE(");
            q.append(is_held_standby(input.now));
            q.push(", 0)");
            out.push(q);
        }
    }
    out
}

/// `bulkAfterId`: `"ContactInbox"."id" > after_id` (none without a cursor).
pub fn bulk_after_id(after_id: Option<i64>) -> Option<Sql> {
    after_id.map(|id| Sql::bind(format!("{CI}\"id\" > "), id))
}

/// `bulkEligibilityWhere`: the inbox, the conditions and the cursor.
pub fn bulk_eligibility_where(
    inbox_id: i64,
    input: &BulkEligibilityInput,
    after_id: Option<i64>,
) -> Sql {
    let mut parts = vec![Some(Sql::bind(format!("{CI}\"inboxId\" = "), inbox_id))];
    parts.extend(bulk_eligibility_conditions(input).into_iter().map(Some));
    parts.push(bulk_after_id(after_id));
    Sql::and(parts)
}
