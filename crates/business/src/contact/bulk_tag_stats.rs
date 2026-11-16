//! `contacts.bulkTagByStats` (P3.6-D2 slice contacts bulk): builder
//! `features/contacts/lib/enqueue-bulk-tag-stats.ts` (`prepare`), the worker
//! job `default/handlers/bulk-tag-contacts.ts` (`run`) and the
//! `getContactIdsPage` queries of `analytics` `broadcast-stats` /
//! `sequence-stats` repositories (`contact_ids_page`).
//!
//! No default-queue worker is ported yet: the API runs the job in the
//! request ([`run`]); [`BulkTagJob::job_data`] keeps the original
//! `bulkTagContacts` payload for the worker port. The comment-automation
//! source finds no contacts (its stats / miss repositories belong to the
//! comment-automation analytics port). The member access scope
//! (`restrictToAssignedUserId`) is carried in the payload, not applied.

use serde_json::Value;

use chatbotx_db::Db;

use crate::errors::Result;

/// `DefaultJobAction.bulkTagContacts`.
pub const JOB_NAME: &str = "bulkTagContacts";

/// `loopableItemsCount` (worker-config): the page size of the job.
pub const LOOPABLE_ITEMS_COUNT: usize = 1000;

/// The stats event whose contacts are tagged (`BulkTagSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkTagSource {
    Broadcast {
        broadcast_id: i64,
        event_type: String,
    },
    SequenceStep {
        sequence_id: i64,
        step_id: i64,
        event_type: String,
    },
    CommentAutomation {
        automation_id: i64,
        event_type: String,
    },
}

/// `BulkTagStatsContactsRequest` (the parsed route / action input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkTagRequest {
    pub source: BulkTagSource,
    pub tags: Vec<String>,
    pub excluded_contact_ids: Vec<i64>,
}

/// `JobBulkTagContacts["data"]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkTagJob {
    pub workspace_id: i64,
    pub requested_user_id: i64,
    pub tag_ids: Vec<i64>,
    pub excluded_contact_ids: Vec<i64>,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub source: BulkTagSource,
}

impl BulkTagJob {
    /// The `defaultQueue.add(bulkTagContacts, ...)` payload:
    /// `{ type: "bulkTagContacts", data: { workspaceId, requestedUserId,
    /// tagIds, excludedContactIds, [restrictToAssignedUserId], source, ... } }`
    /// (ids as strings).
    pub fn job_data(&self) -> Value {
        todo!("P3.6-D2")
    }
}

/// One `getContactIdsPage` row: the cursor id and the contact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactIdRow {
    pub id: i64,
    pub contact_id: i64,
}

/// The route pre-check: a broadcast (`broadcastService.findByIdOrName`, 404
/// "Broadcast not found") or a sequence (`sequenceService.assertOwned`, 404
/// "Sequence not found") of another workspace; a comment automation is not
/// checked (scoped in the job).
pub async fn assert_source_owned(db: &Db, workspace_id: i64, source: &BulkTagSource) -> Result<()> {
    let _ = (db, workspace_id, source);
    todo!("P3.6-D2")
}

/// `enqueueBulkTagStatsContacts` without the enqueue: upserts the tags by
/// name; `None` when no tag name is usable (nothing to queue).
pub async fn prepare(
    db: &Db,
    workspace_id: i64,
    requested_user_id: i64,
    request: &BulkTagRequest,
    restrict_to_assigned_user_id: Option<i64>,
) -> Result<Option<BulkTagJob>> {
    let _ = (
        db,
        workspace_id,
        requested_user_id,
        request,
        restrict_to_assigned_user_id,
    );
    todo!("P3.6-D2")
}

/// `getContactIdsPage` of the job's source: broadcast rows by `contactId`
/// (cursor = contact id), sequence-step dispatches by `id` (cursor =
/// dispatch id), both without the excluded contacts.
pub async fn contact_ids_page(
    db: &Db,
    job: &BulkTagJob,
    cursor: Option<i64>,
    limit: usize,
) -> Result<Vec<ContactIdRow>> {
    let _ = (db, job, cursor, limit);
    todo!("P3.6-D2")
}

/// `handleBulkTagContacts`: pages through the source (`chunkById`) and
/// attaches the tags to each page's unique contacts. Returns the newly linked
/// `(contactId, tagId)` pairs (`tagApplied` events).
pub async fn run(db: &Db, job: &BulkTagJob) -> Result<Vec<(i64, i64)>> {
    let _ = (db, job);
    todo!("P3.6-D2")
}
