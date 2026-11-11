//! `enroll-contact.ts`: `enrollContactInSequence` and
//! `enrollContactsInSequenceBulk`.

use crate::sequence::repo::{ClientMode, ContactInboxRef, SequenceCtx, SequenceError};

/// `enrollContactInSequence` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrollParams {
    pub workspace_id: String,
    pub contact_id: String,
    pub sequence_id: String,
    pub next_run_at_ms: i64,
    pub next_step_id: Option<String>,
    /// Contact inboxes the caller already has; `None` = look them up.
    pub contact_inboxes: Option<Vec<ContactInboxRef>>,
}

/// `{ enrolled }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollResult {
    pub enrolled: bool,
}

/// `enrollContactInSequence(params)`: `INSERT ... ON CONFLICT DO NOTHING
/// RETURNING id` of the `ContactsOnSequence` row (id from `ctx.ids`);
/// conflict = `{ enrolled: false }` and nothing else. With a next step, one
/// dispatch per contact inbox through `ctx.dispatches`, in the same
/// transaction (`ClientMode::Default`) or without one (`ClientMode::Root`);
/// the dispatches are added to `ctx.schedule` only after the commit.
pub async fn enroll_contact_in_sequence(
    ctx: &SequenceCtx,
    params: EnrollParams,
    mode: ClientMode,
) -> Result<EnrollResult, SequenceError> {
    todo!()
}

/// One enrollment of a bulk enroll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkEnrollment {
    pub contact_id: String,
    pub sequence_id: String,
    pub next_run_at_ms: Option<i64>,
    pub next_step_id: Option<String>,
}

/// `enrollContactsInSequenceBulk({ workspaceId, enrollments })`: one bulk
/// insert (`ON CONFLICT DO NOTHING RETURNING`); only inserted rows with both
/// a next step and a run time get dispatches (one per contact inbox), then
/// each is scheduled after the commit.
pub async fn enroll_contacts_in_sequence_bulk(
    ctx: &SequenceCtx,
    workspace_id: &str,
    enrollments: Vec<BulkEnrollment>,
) -> Result<(), SequenceError> {
    todo!()
}
