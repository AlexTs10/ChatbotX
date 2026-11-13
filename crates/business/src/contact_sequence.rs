//! `packages/business/src/contact-sequence/service.ts` (P3.6-D2 slice
//! contact sequences): `subscribeContacts`, `listByContactId`,
//! `removeContactSequencesForContacts`, `updateContactSequences`. Private
//! queries on `"Sequence"`, `"ContactOnSequence"`, `"Contact"`,
//! `"ContactInbox"`; dispatch rows through `chatbotx_jobs::sequence`
//! (`create_dispatch_in`, `cancel_pending_dispatches_in`). No scheduler
//! add / remove and no `sequenceUnsubscribed` events (the API state has no
//! scheduler or event bus).

use chatbotx_db::Db;

use crate::errors::Result;

/// `CHUNK_SIZE` of the contact lookups.
pub const CHUNK_SIZE: usize = 1000;

/// `{ processedContactIds, skippedContactIds }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscribeOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// One `listByContactId` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactSequenceRow {
    pub sequence_id: i64,
    pub sequence_name: String,
}

/// `subscribeContacts`: every sequence must be the workspace's (404
/// "Sequence not found", nothing enrolled); contacts that do not resolve are
/// skipped; an existing enrollment is kept.
pub async fn subscribe_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    sequence_ids: &[i64],
) -> Result<SubscribeOutcome> {
    let _ = (db, workspace_id, contact_ids, sequence_ids);
    todo!("P3.6-D2")
}

/// `listByContactId`: 404 "Contact not found" for a contact outside the
/// workspace.
pub async fn list_by_contact_id(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactSequenceRow>> {
    let _ = (db, workspace_id, contact_id);
    todo!("P3.6-D2")
}

/// `removeContactSequencesForContacts` (`subscription_removed`): deletes the
/// enrollments and cancels their pending dispatches, in one transaction.
pub async fn remove_contact_sequences_for_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    sequence_ids: &[i64],
) -> Result<()> {
    let _ = (db, workspace_id, contact_ids, sequence_ids);
    todo!("P3.6-D2")
}

/// `updateContactSequences`: replaces the contact's set in one transaction
/// (a sequence to add that is not the workspace's is 404 and changes nothing).
pub async fn update_contact_sequences(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    sequence_ids: &[i64],
) -> Result<Vec<ContactSequenceRow>> {
    let _ = (db, workspace_id, contact_id, sequence_ids);
    todo!("P3.6-D2")
}
