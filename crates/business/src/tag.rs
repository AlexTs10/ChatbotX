//! `packages/business/src/tag/service.ts`: the tags of a workspace and the
//! contact links (`"ContactToTag"`). Private queries on `"Tag"`,
//! `"ContactToTag"`, `"Contact"`, `"Folder"`. No tag-sync jobs, no
//! `tagApplied` / `tagRemoved` events, no ads-conversion enqueue, no read
//! cache, no template edit locks / delete guards (P3.6-D1 rule).

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::Tag;

use crate::errors::Result;

/// `CONTACT_CHUNK_SIZE`.
pub const CONTACT_CHUNK_SIZE: usize = 200;

/// `list` input (`folderId: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListTags {
    pub name: Option<String>,
    pub folder_id: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// One listed tag and its `contactsCount`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedTag {
    pub tag: Tag,
    pub contacts_count: i64,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct TagPage {
    pub data: Vec<ListedTag>,
    pub page_count: i64,
}

/// `attachByNamesToContacts` / `detachByNamesFromContacts` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactsOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// `list`: live tags, `sort` columns (then `id`), `name` an ILIKE substring.
pub async fn list(db: &Db, workspace_id: i64, input: ListTags) -> Result<TagPage> {
    let _ = (db, workspace_id, input);
    todo!("P3.6-D2")
}

/// `create`: a live tag with the same name is
/// `validation("name", "Name is already taken.")`; `folder_id: Some(0)` = root.
pub async fn create(db: &Db, workspace_id: i64, name: &str, folder_id: Option<i64>) -> Result<Tag> {
    let _ = (db, workspace_id, name, folder_id);
    todo!("P3.6-D2")
}

/// `update`: rename; `folder_id: None` keeps the folder, `Some(None)` /
/// `Some(Some(0))` = the root.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: &str,
    folder_id: Option<Option<i64>>,
) -> Result<Tag> {
    let _ = (db, workspace_id, id, name, folder_id);
    todo!("P3.6-D2")
}

/// `softDelete`: sets `deletedAt` on the live tags of `ids`; returns the ids
/// deleted.
pub async fn soft_delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}

/// `findByKey`: a numeric key is tried as a live tag id first, then as a name.
pub async fn find_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<Option<Tag>> {
    let _ = (db, workspace_id, key);
    todo!("P3.6-D2")
}

/// `findByKeyOrFail`: 404 "Tag not found".
pub async fn find_by_key_or_fail(db: &Db, workspace_id: i64, key: &str) -> Result<Tag> {
    let _ = (db, workspace_id, key);
    todo!("P3.6-D2")
}

/// `listForContact`: the contact's live tags by name.
pub async fn list_for_contact(db: &Db, workspace_id: i64, contact_id: i64) -> Result<Vec<Tag>> {
    let _ = (db, workspace_id, contact_id);
    todo!("P3.6-D2")
}

/// `upsertByNames`: trimmed, unique, non-empty names; missing ones are
/// created. Returns `(id, name)` of every live match.
pub async fn upsert_by_names(db: &Db, workspace_id: i64, names: &[String]) -> Result<Vec<(i64, String)>> {
    let _ = (db, workspace_id, names);
    todo!("P3.6-D2")
}

/// `attachToContact`: links the live workspace tags of `tag_ids` (others
/// ignored); 404 "Contact not found" for a contact of another workspace.
/// Returns the newly linked tag ids.
pub async fn attach_to_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    tag_ids: &[i64],
) -> Result<Vec<i64>> {
    let _ = (db, workspace_id, contact_id, tag_ids);
    todo!("P3.6-D2")
}

/// `detachFromContact`: unlinks `tag_ids`; returns the removed tag ids.
pub async fn detach_from_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    tag_ids: &[i64],
) -> Result<Vec<i64>> {
    let _ = (db, workspace_id, contact_id, tag_ids);
    todo!("P3.6-D2")
}

/// `attachByNamesToContacts` (contacts in chunks of 200).
pub async fn attach_by_names_to_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    names: &[String],
) -> Result<ContactsOutcome> {
    let _ = (db, workspace_id, contact_ids, names);
    todo!("P3.6-D2")
}

/// `detachByNamesFromContacts`: never creates a tag.
pub async fn detach_by_names_from_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    names: &[String],
) -> Result<ContactsOutcome> {
    let _ = (db, workspace_id, contact_ids, names);
    todo!("P3.6-D2")
}

/// `replaceContactTagsByNames`: the contact's tag set becomes `names`
/// (created when missing); returns the resolved tags.
pub async fn replace_contact_tags_by_names(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    names: &[String],
) -> Result<Vec<Tag>> {
    let _ = (db, workspace_id, contact_id, names);
    todo!("P3.6-D2")
}
