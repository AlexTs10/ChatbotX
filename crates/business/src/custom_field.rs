//! `packages/business/src/custom-field/service.ts`: the custom field
//! definitions of a workspace (`list`, `findByKey(OrFail)`, `create`,
//! `update`, `delete`). Private queries on `"CustomField"` / `"Folder"`.
//! No read cache, no template edit locks / delete guards (P3.6-D1 rule),
//! no advisory name lock (one SQLite writer).

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::CustomField;

use crate::errors::Result;

/// `ListCustomFieldsInput` (`folderId: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListCustomFields {
    pub folder_id: Option<i64>,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `PaginatedResult<CustomFieldModel>`.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomFieldPage {
    pub data: Vec<CustomField>,
    pub page_count: i64,
}

/// `CreateCustomFieldData` (`folderId: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateCustomField {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub folder_id: Option<i64>,
}

/// `UpdateCustomFieldData`: `None` = keep. `folder_id: Some(None)` or
/// `Some(Some(0))` = the root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateCustomField {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub folder_id: Option<Option<i64>>,
}

/// `list`: one page, `sort` columns of the model (then `id`), `name` an
/// ILIKE substring.
pub async fn list(db: &Db, workspace_id: i64, input: ListCustomFields) -> Result<CustomFieldPage> {
    let _ = (db, workspace_id, input);
    todo!("P3.6-D2")
}

/// `findByKey`: a numeric key is tried as an id first, then as a name
/// (exact match).
pub async fn find_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<Option<CustomField>> {
    let _ = (db, workspace_id, key);
    todo!("P3.6-D2")
}

/// `findByKeyOrFail`: 404 "Custom field not found".
pub async fn find_by_key_or_fail(db: &Db, workspace_id: i64, key: &str) -> Result<CustomField> {
    let _ = (db, workspace_id, key);
    todo!("P3.6-D2")
}

/// `create` (`showInInbox: true`); a taken `(type, name)` is
/// `validation("name", "Name is already taken")`.
pub async fn create(db: &Db, workspace_id: i64, data: CreateCustomField) -> Result<CustomField> {
    let _ = (db, workspace_id, data);
    todo!("P3.6-D2")
}

/// `update` of the field `key` resolves to.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    key: &str,
    data: UpdateCustomField,
) -> Result<CustomField> {
    let _ = (db, workspace_id, key, data);
    todo!("P3.6-D2")
}

/// `delete`: the fields of `ids` in the workspace (others are ignored).
pub async fn delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}
