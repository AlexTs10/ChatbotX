//! `contact/service.ts`: `resolveIdByIdentifier`, `findPublicContactOrFail`,
//! `block` / `unblock` (`*AndRecord`), `delete` / `deleteAndRecord`,
//! `upsertByIdentifier`.

use chatbotx_db::Db;
use chatbotx_db::repo::contact::PublicContact;
use chatbotx_db::repo::models::Contact;

use crate::Result;

/// `resolveIdByIdentifier`.
pub async fn resolve_id_by_identifier(db: &Db, workspace_id: i64, identifier: &str) -> Result<i64> {
    let _ = (db, workspace_id, identifier);
    todo!("P3.6-D2")
}

/// `findPublicContactOrFail`.
pub async fn find_public_contact_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<PublicContact> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D2")
}

/// `blockAndRecord`: sets `blockedAt` to `now_ms`.
pub async fn block_and_record(db: &Db, workspace_id: i64, id: i64, now_ms: i64) -> Result<Contact> {
    let _ = (db, workspace_id, id, now_ms);
    todo!("P3.6-D2")
}

/// `unblockAndRecord`: clears `blockedAt`.
pub async fn unblock_and_record(db: &Db, workspace_id: i64, id: i64) -> Result<Contact> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D2")
}

/// `delete`: the workspace's contacts of `ids` with their sub-rows; one
/// `MessageCleanup` tombstone per contact inbox. Returns the deleted ids.
pub async fn delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}

/// `deleteAndRecord` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// `deleteAndRecord`.
pub async fn delete_and_record(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<DeleteOutcome> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}

/// The fields of an upsert (`None` = not given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertData {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub gender: Option<String>,
}

/// `upsertByIdentifier` result.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertOutcome {
    pub contact: Contact,
    pub is_new: bool,
}

/// `upsertByIdentifier`: updates the matching contact, or creates an
/// `email:` / `phone:` one on the workspace webchat inbox.
pub async fn upsert_by_identifier(
    db: &Db,
    workspace_id: i64,
    identifier: &str,
    data: UpsertData,
    source: &str,
    avatar: Option<String>,
) -> Result<UpsertOutcome> {
    let _ = (db, workspace_id, identifier, data, source, avatar);
    todo!("P3.6-D2")
}
