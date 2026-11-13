//! `packages/business/src/contact-note/service.ts`: internal notes of a
//! contact. Private queries on `"ContactNote"` / `"Contact"`. No read cache.

use chatbotx_db::Db;
use chatbotx_db::repo::models::ContactNote;

use crate::errors::Result;

/// `create`: 404 "Contact not found" for a contact outside the workspace.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    text: &str,
    created_by_id: Option<i64>,
) -> Result<ContactNote> {
    let _ = (db, workspace_id, contact_id, text, created_by_id);
    todo!("P3.6-D2")
}

/// `update`: 404 "Contact note not found" for a note of another contact.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    note_id: i64,
    text: Option<&str>,
) -> Result<ContactNote> {
    let _ = (db, workspace_id, contact_id, note_id, text);
    todo!("P3.6-D2")
}

/// `delete`: 404 "Contact note not found" when no row was deleted.
pub async fn delete(db: &Db, workspace_id: i64, contact_id: i64, note_id: i64) -> Result<()> {
    let _ = (db, workspace_id, contact_id, note_id);
    todo!("P3.6-D2")
}

/// `listByContactId`: newest first.
pub async fn list_by_contact_id(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactNote>> {
    let _ = (db, workspace_id, contact_id);
    todo!("P3.6-D2")
}
