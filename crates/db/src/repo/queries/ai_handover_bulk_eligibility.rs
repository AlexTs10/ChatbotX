//! Port of `queries/ai-handover-bulk-eligibility.ts`: the one definition of "this thread
//! is touched by a bulk AI hand-over run", shared by the page query and the count.
//!
//! Columns are qualified with the unaliased `"ContactInbox"` table.

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

/// `bulkEligibilityConditions`.
pub fn bulk_eligibility_conditions(input: &BulkEligibilityInput) -> Vec<Sql> {
    let _ = input;
    todo!("P3.2b")
}

/// `bulkAfterId`: `"ContactInbox"."id" > after_id` (none without a cursor).
pub fn bulk_after_id(after_id: Option<i64>) -> Option<Sql> {
    let _ = after_id;
    todo!("P3.2b")
}

/// `bulkEligibilityWhere`: the inbox, the conditions and the cursor.
pub fn bulk_eligibility_where(
    inbox_id: i64,
    input: &BulkEligibilityInput,
    after_id: Option<i64>,
) -> Sql {
    let _ = (inbox_id, input, after_id);
    todo!("P3.2b")
}
