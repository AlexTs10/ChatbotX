//! `contacts-on-sequences.ts`: `contactsOnSequencesUtils` and
//! `getContactInboxes`.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};

use crate::sequence::repo::{
    SequenceCtx, SequenceError, StatementKind, TxObserver, note, parse_id, read_tx,
};

/// `{ toAdd, toRemove }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceDiff {
    pub to_add: Vec<String>,
    pub to_remove: Vec<String>,
}

/// `calculateSequenceDiff(current, next)`: set semantics, first-seen order,
/// duplicates removed.
pub fn calculate_sequence_diff(current: &[&str], next: &[&str]) -> SequenceDiff {
    let unique = |items: &[&str]| {
        let mut out: Vec<String> = Vec::new();
        for item in items {
            if !out.iter().any(|o| o == item) {
                out.push((*item).to_owned());
            }
        }
        out
    };
    let current = unique(current);
    let next = unique(next);
    SequenceDiff {
        to_add: next
            .iter()
            .filter(|id| !current.contains(id))
            .cloned()
            .collect(),
        to_remove: current
            .iter()
            .filter(|id| !next.contains(id))
            .cloned()
            .collect(),
    }
}

/// `getAllSequenceIds(client, { contactId })`: the `sequenceId` of every
/// `ContactsOnSequence` row of the contact, in row order.
pub fn get_all_sequence_ids_in(
    conn: &Connection,
    contact_id: &str,
) -> Result<Vec<String>, SequenceError> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT "sequenceId" FROM "ContactOnSequence" WHERE "contactId" = ?1 ORDER BY "id""#,
    )?;
    let rows = stmt
        .query_map(params![parse_id(contact_id)?], |r| r.get::<_, i64>(0))?
        .map(|r| r.map(|id| id.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// [`get_all_sequence_ids_in`] on a reader connection.
pub async fn get_all_sequence_ids(db: &Db, contact_id: &str) -> Result<Vec<String>, SequenceError> {
    let contact_id = contact_id.to_owned();
    read_tx(db, move |conn| get_all_sequence_ids_in(conn, &contact_id)).await
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
    note(observer, StatementKind::Select, "Inbox");
    let inboxes: Vec<i64> = conn
        .prepare_cached(r#"SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?1 ORDER BY "id""#)?
        .query_map(params![parse_id(workspace_id)?], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    if inboxes.is_empty() {
        return Ok(Vec::new());
    }
    note(observer, StatementKind::Select, "ContactInbox");
    let list = inboxes
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // The ids are integers from the query above: safe to inline.
    let sql = format!(
        r#"SELECT "id", "contactId", "inboxId" FROM "ContactInbox"
           WHERE "contactId" = ?1 AND "inboxId" IN ({list}) ORDER BY "id""#
    );
    let rows = conn
        .prepare(&sql)?
        .query_map(params![parse_id(contact_id)?], |r| {
            Ok(ContactInboxRow {
                id: r.get::<_, i64>(0)?.to_string(),
                contact_id: r.get::<_, i64>(1)?.to_string(),
                inbox_id: r.get::<_, i64>(2)?.to_string(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// [`get_contact_inboxes_in`] on a reader connection of `ctx.db`, reporting
/// statements to `ctx.observer`.
pub async fn get_contact_inboxes(
    ctx: &SequenceCtx,
    workspace_id: &str,
    contact_id: &str,
) -> Result<Vec<ContactInboxRow>, SequenceError> {
    let observer = ctx.observer.clone();
    let (workspace_id, contact_id) = (workspace_id.to_owned(), contact_id.to_owned());
    read_tx(&ctx.db, move |conn| {
        get_contact_inboxes_in(conn, observer.as_deref(), &workspace_id, &contact_id)
    })
    .await
}
