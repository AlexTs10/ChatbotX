//! `packages/business/src/folder/service.ts` (P3.6-D2 slice folders):
//! `list`, `find(OrFail)`, `create`, `update`, `bulkDelete`. Private
//! queries on `"Folder"`. `paths` is stored as a JSON array of id strings
//! (the original `bigint[]` as text). No template edit locks or delete
//! guards (P3.6-D1 rule; one SQLite writer).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Folder;

use crate::errors::Result;

/// `rootFolderId`: the "top level" parent sentinel.
pub const ROOT_FOLDER_ID: &str = "0";

/// `folderTypes`.
pub const FOLDER_TYPES: &[&str] = &[
    "tag",
    "flow",
    "customField",
    "automatedResponse",
    "trigger",
    "webhook",
    "sequence",
    "emailTopic",
    "fbComment",
    "igComment",
    "igStory",
    "outboundAutomatedResponse",
];

/// `list`: the workspace's folders of one type, oldest first; `parent_id`
/// `None` = every folder, `"0"` = the top level, else that parent's
/// children.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    folder_type: &str,
    parent_id: Option<&str>,
    is_trash: Option<bool>,
) -> Result<Vec<Folder>> {
    let _ = (db, workspace_id, folder_type, parent_id, is_trash);
    todo!("P3.6-D2")
}

/// `find`: the workspace's folder (of `folder_type` when given).
pub async fn find(
    db: &Db,
    workspace_id: i64,
    id: i64,
    folder_type: Option<&str>,
) -> Result<Option<Folder>> {
    let _ = (db, workspace_id, id, folder_type);
    todo!("P3.6-D2")
}

/// `findOrFail`: 404 "Folder not found".
pub async fn find_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
    folder_type: Option<&str>,
) -> Result<Folder> {
    let _ = (db, workspace_id, id, folder_type);
    todo!("P3.6-D2")
}

/// `create`: a parent must be a folder of the same workspace and type
/// (404 "Parent folder does not exist!"); `paths` = the parent's paths + its id.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: &str,
    folder_type: &str,
    parent_id: Option<i64>,
) -> Result<Folder> {
    let _ = (db, workspace_id, name, folder_type, parent_id);
    todo!("P3.6-D2")
}

/// `update`: renames the folder (404 "Folder not found").
pub async fn update(db: &Db, workspace_id: i64, id: i64, name: &str) -> Result<Folder> {
    let _ = (db, workspace_id, id, name);
    todo!("P3.6-D2")
}

/// `bulkDelete`: deletes each folder and its sub-folders (unknown ids are
/// skipped); the resources in them keep their rows (`folderId` set NULL).
pub async fn bulk_delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}
