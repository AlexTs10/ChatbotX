//! `packages/business/src/custom-field/service.ts`: the custom field
//! definitions of a workspace (`list`, `findByKey(OrFail)`, `create`,
//! `update`, `delete`). Private queries on `"CustomField"` / `"Folder"`.
//! No read cache, no template edit locks / delete guards (P3.6-D1 rule),
//! no advisory name lock (one SQLite writer).

use chatbotx_db::Db;
use chatbotx_db::repo::models::CustomField;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by, with_id_tie_break};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error,
    like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `toStoredFolderId`: `0` (the root) is `NULL`.
fn stored_folder_id(folder_id: Option<i64>) -> Option<i64> {
    folder_id.filter(|id| *id != 0)
}

/// `folderService.ensureExists` (a `customField` folder of the workspace).
fn ensure_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'customField'"#,
        params![folder_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Folder not found"))
}

fn find_by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<CustomField>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            CustomField::select_sql()
        ),
        params![id, workspace_id],
        CustomField::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// `findByKey` on a connection.
pub fn find_by_key_in(
    conn: &Connection,
    workspace_id: i64,
    key: &str,
) -> Result<Option<CustomField>> {
    if chatbotx_db::repo::is_bigint_id(key)
        && let Some(field) = find_by_id(conn, workspace_id, key.parse().unwrap_or_default())?
    {
        return Ok(Some(field));
    }
    conn.query_row(
        &format!(
            r#"{} WHERE "name" = ?1 AND "workspaceId" = ?2 ORDER BY "id" LIMIT 1"#,
            CustomField::select_sql()
        ),
        params![key, workspace_id],
        CustomField::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// A unique `(workspaceId, type, name)` violation is the name check.
fn name_taken(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = chatbotx_db::repo::Error::from(error);
    if is_unique_violation_error(&error, None) {
        BusinessError::validation("name", "Name is already taken")
    } else {
        BusinessError::Repo(error)
    }
}

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
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let order = with_id_tie_break(&parse_order_by(
            CustomField::COLUMNS,
            input.sort.as_deref(),
            None,
        ));
        let mut filter = String::from(r#""workspaceId" = ?"#);
        let mut binds: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        match input.folder_id {
            Some(0) => filter.push_str(r#" AND "folderId" IS NULL"#),
            Some(folder) => {
                filter.push_str(r#" AND "folderId" = ?"#);
                binds.push(Sql::Integer(folder));
            }
            None => {}
        }
        if let Some(name) = input.name.as_deref().filter(|n| !n.is_empty()) {
            filter.push_str(r#" AND ilike("name", ?)"#);
            binds.push(Sql::Text(like_contains(name)));
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "CustomField" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let sql = format!(
            "{} WHERE {filter} {} LIMIT ? OFFSET ?",
            CustomField::select_sql(),
            order_by_sql(&order, None)
        );
        binds.push(Sql::Integer(pagination.limit));
        binds.push(Sql::Integer(pagination.offset));
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(binds.iter()), CustomField::from_row)
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            1
        };
        Ok(CustomFieldPage { data, page_count })
    })
    .await
}

/// `findByKey`: a numeric key is tried as an id first, then as a name
/// (exact match).
pub async fn find_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<Option<CustomField>> {
    let key = key.to_string();
    tx::read(db, move |conn| find_by_key_in(conn, workspace_id, &key)).await
}

/// `findByKeyOrFail`: 404 "Custom field not found".
pub async fn find_by_key_or_fail(db: &Db, workspace_id: i64, key: &str) -> Result<CustomField> {
    find_by_key(db, workspace_id, key)
        .await?
        .ok_or_else(|| BusinessError::not_found("Custom field not found"))
}

/// `create` (`showInInbox: true`); a taken `(type, name)` is
/// `validation("name", "Name is already taken")`.
pub async fn create(db: &Db, workspace_id: i64, data: CreateCustomField) -> Result<CustomField> {
    tx::write(db, move |conn| {
        let folder_id = stored_folder_id(data.folder_id);
        if let Some(folder) = folder_id {
            ensure_folder(conn, workspace_id, folder)?;
        }
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "CustomField" ("id", "createdAt", "updatedAt", "name", "type", "description",
                   "folderId", "showInInbox", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 1, ?7)"#,
            params![id, now, data.name, data.r#type, data.description, folder_id, workspace_id],
        )
        .map_err(name_taken)?;
        find_by_id(conn, workspace_id, id)?.ok_or_else(|| BusinessError::not_found("Custom field not found"))
    })
    .await
}

/// `update` of the field `key` resolves to.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    key: &str,
    data: UpdateCustomField,
) -> Result<CustomField> {
    let key = key.to_string();
    tx::write(db, move |conn| {
        let existing = find_by_key_in(conn, workspace_id, &key)?
            .ok_or_else(|| BusinessError::not_found("Custom field not found"))?;
        let folder_id = data.folder_id.map(stored_folder_id);
        if let Some(Some(folder)) = folder_id
            && Some(folder) != existing.folder_id
        {
            ensure_folder(conn, workspace_id, folder)?;
        }
        let mut sets = vec![r#""updatedAt" = ?"#.to_string()];
        let mut binds: Vec<Sql> = vec![Sql::Integer(now_ms())];
        if let Some(name) = data.name {
            sets.push(r#""name" = ?"#.to_string());
            binds.push(Sql::Text(name));
        }
        if let Some(description) = data.description {
            sets.push(r#""description" = ?"#.to_string());
            binds.push(description.map_or(Sql::Null, Sql::Text));
        }
        if let Some(folder) = folder_id {
            sets.push(r#""folderId" = ?"#.to_string());
            binds.push(folder.map_or(Sql::Null, Sql::Integer));
        }
        binds.push(Sql::Integer(existing.id));
        conn.execute(
            &format!(
                r#"UPDATE "CustomField" SET {} WHERE "id" = ?"#,
                sets.join(", ")
            ),
            params_from_iter(binds.iter()),
        )
        .map_err(name_taken)?;
        find_by_id(conn, workspace_id, existing.id)?
            .ok_or_else(|| BusinessError::not_found("Custom field not found"))
    })
    .await
}

/// `delete`: the fields of `ids` in the workspace (others are ignored).
pub async fn delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let ids = ids.to_vec();
    tx::write(db, move |conn| {
        for id in ids {
            conn.execute(
                r#"DELETE FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, id],
            )
            .map_err(repo_err)?;
        }
        Ok(())
    })
    .await
}
