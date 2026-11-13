//! `packages/business/src/folder/service.ts` (P3.6-D2 slice folders):
//! `list`, `find(OrFail)`, `create`, `update`, `bulkDelete`. Private
//! queries on `"Folder"`. `paths` is stored as a JSON array of id strings
//! (the original `bigint[]` as text). No template edit locks or delete
//! guards (P3.6-D1 rule; one SQLite writer).

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::repo::models::Folder;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{Model, new_id};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value as Json;

use crate::errors::{BusinessError, Result};
use crate::tx;

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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn folder_where(conn: &Connection, filter: Sql) -> repo::Result<Vec<Folder>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Folder\" WHERE ",
        Folder::columns_sql(None)
    ));
    q.append(filter);
    q.push(" ORDER BY \"createdAt\" ASC, \"id\" ASC");
    q.models(conn)
}

fn find_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    folder_type: Option<&str>,
) -> repo::Result<Option<Folder>> {
    let mut filter = Sql::bind("\"workspaceId\" = ", workspace_id);
    filter.push(" AND \"id\" = ");
    filter.push_bind(id);
    if let Some(folder_type) = folder_type {
        filter.push(" AND \"folderType\" = ");
        filter.push_bind(folder_type.to_string());
    }
    Ok(folder_where(conn, filter)?.into_iter().next())
}

fn folder_not_found() -> BusinessError {
    BusinessError::not_found("Folder not found")
}

/// `list`: the workspace's folders of one type, oldest first; `parent_id`
/// `None` = every folder, `"0"` = the top level, else that parent's
/// children (an id that is not a bigint matches nothing).
pub async fn list(
    db: &Db,
    workspace_id: i64,
    folder_type: &str,
    parent_id: Option<&str>,
    is_trash: Option<bool>,
) -> Result<Vec<Folder>> {
    let folder_type = folder_type.to_string();
    let parent_id = parent_id.map(str::to_string);
    tx::read(db, move |conn| {
        let mut filter = Sql::bind("\"workspaceId\" = ", workspace_id);
        filter.push(" AND \"folderType\" = ");
        filter.push_bind(folder_type);
        if let Some(is_trash) = is_trash {
            filter.push(" AND \"isTrash\" = ");
            filter.push_bind(i64::from(is_trash));
        }
        match parent_id.as_deref() {
            None => {}
            Some(ROOT_FOLDER_ID) => {
                filter.push(" AND \"parentId\" IS NULL");
            }
            Some(text) => match text.parse::<i64>() {
                Ok(id) => {
                    filter.push(" AND \"parentId\" = ");
                    filter.push_bind(id);
                }
                Err(_) => {
                    filter.push(" AND FALSE");
                }
            },
        }
        Ok(folder_where(conn, filter)?)
    })
    .await
}

/// `find`: the workspace's folder (of `folder_type` when given).
pub async fn find(
    db: &Db,
    workspace_id: i64,
    id: i64,
    folder_type: Option<&str>,
) -> Result<Option<Folder>> {
    let folder_type = folder_type.map(str::to_string);
    tx::read(db, move |conn| {
        Ok(find_in(conn, workspace_id, id, folder_type.as_deref())?)
    })
    .await
}

/// `findOrFail`: 404 "Folder not found".
pub async fn find_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
    folder_type: Option<&str>,
) -> Result<Folder> {
    find(db, workspace_id, id, folder_type)
        .await?
        .ok_or_else(folder_not_found)
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
    let name = name.to_string();
    let folder_type = folder_type.to_string();
    tx::write(db, move |tx| {
        let mut paths: Vec<Json> = Vec::new();
        if let Some(parent_id) = parent_id {
            let parent = find_in(tx, workspace_id, parent_id, Some(&folder_type))?
                .ok_or_else(|| BusinessError::not_found("Parent folder does not exist!"))?;
            paths = parent.paths.0.as_array().cloned().unwrap_or_default();
            paths.push(Json::String(parent.id.to_string()));
        }
        let id = new_id();
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO "Folder" ("id", "createdAt", "updatedAt", "name", "folderType", "parentId", "workspaceId", "paths")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![id, now, name, folder_type, parent_id, workspace_id, Json::Array(paths).to_string()],
        )
        .map_err(repo::Error::from)?;
        find_in(tx, workspace_id, id, None)?.ok_or_else(folder_not_found)
    })
    .await
}

/// `update`: renames the folder (404 "Folder not found").
pub async fn update(db: &Db, workspace_id: i64, id: i64, name: &str) -> Result<Folder> {
    let name = name.to_string();
    tx::write(db, move |tx| {
        find_in(tx, workspace_id, id, None)?.ok_or_else(folder_not_found)?;
        tx.execute(
            r#"UPDATE "Folder" SET "name" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
            params![name, now_ms(), id],
        )
        .map_err(repo::Error::from)?;
        find_in(tx, workspace_id, id, None)?.ok_or_else(folder_not_found)
    })
    .await
}

/// `bulkDelete`: deletes each folder and its sub-folders (unknown ids are
/// skipped); the resources in them keep their rows (`folderId` set NULL).
pub async fn bulk_delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let ids = ids.to_vec();
    tx::write(db, move |tx| {
        for id in ids {
            tx.execute(
                r#"DELETE FROM "Folder" WHERE "workspaceId" = ?1 AND ("id" = ?2
                   OR EXISTS (SELECT 1 FROM json_each("Folder"."paths") AS p WHERE CAST(p."value" AS TEXT) = CAST(?2 AS TEXT)))"#,
                params![workspace_id, id],
            )
            .map_err(repo::Error::from)?;
        }
        Ok(())
    })
    .await
}
