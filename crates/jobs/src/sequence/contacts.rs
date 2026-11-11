//! `contacts-on-sequences.ts`: `contactsOnSequencesUtils` and
//! `getContactInboxes`.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::Connection;

use crate::sequence::repo::{SequenceCtx, SequenceError, TxObserver};

/// `{ toAdd, toRemove }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceDiff {
    pub to_add: Vec<String>,
    pub to_remove: Vec<String>,
}

/// `calculateSequenceDiff(current, next)`: set semantics, first-seen order,
/// duplicates removed.
pub fn calculate_sequence_diff(current: &[&str], next: &[&str]) -> SequenceDiff {
    todo!()
}

/// `getAllSequenceIds(client, { contactId })`: the `sequenceId` of every
/// `ContactsOnSequence` row of the contact, in row order.
pub fn get_all_sequence_ids_in(
    conn: &Connection,
    contact_id: &str,
) -> Result<Vec<String>, SequenceError> {
    todo!()
}

/// [`get_all_sequence_ids_in`] on a reader connection.
pub async fn get_all_sequence_ids(db: &Db, contact_id: &str) -> Result<Vec<String>, SequenceError> {
    todo!()
}

/// A `ContactInbox` row as `getContactInboxes` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactInboxRow {
    pub id: String,
    pub contact_id: String,
    pub inbox_id: String,
}

/// `getContactInboxes(workspaceId, contactId)`: the inboxes of the
/// workspace, then the contact inboxes of the contact on those inboxes; no
/// inbox = no second query.
pub fn get_contact_inboxes_in(
    conn: &Connection,
    observer: Option<&dyn TxObserver>,
    workspace_id: &str,
    contact_id: &str,
) -> Result<Vec<ContactInboxRow>, SequenceError> {
    todo!()
}

/// [`get_contact_inboxes_in`] on a reader connection of `ctx.db`, reporting
/// statements to `ctx.observer`.
pub async fn get_contact_inboxes(
    ctx: &SequenceCtx,
    workspace_id: &str,
    contact_id: &str,
) -> Result<Vec<ContactInboxRow>, SequenceError> {
    todo!()
}
