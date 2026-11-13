//! `reflink/service.ts` (`reflinkService`). Ref links and QR codes share the
//! `"Reflink"` table (`type` = `refLink` / `qrCode`). Template edit locks are
//! not checked (DECISIONS, P3.6-D1); the parent-flow workspace check of
//! `assertReferencable` is ported ([`assert_parent_flows_in_workspace`]).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Reflink;
use chatbotx_db::repo::{Model, is_unique_violation_error, new_id, reflink as repo};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::json;

pub use chatbotx_db::repo::reflink::{ReflinkListInput, ReflinkWithLogo, ReflinkWithRelations};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Reflink not found")
}

fn name_taken(error: chatbotx_db::repo::Error) -> BusinessError {
    if is_unique_violation_error(&error, None) {
        BusinessError::validation("name", "Name is already taken")
    } else {
        BusinessError::Repo(error)
    }
}

fn id_list(ids: &[i64]) -> String {
    json!(ids).to_string()
}

/// One `"Reflink"` row of the workspace and type (private query).
pub(crate) fn find_typed(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    kind: &str,
) -> Result<Option<Reflink>> {
    conn.query_row(
        &format!(
            "SELECT {} FROM \"Reflink\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"type\" = ?3",
            Reflink::columns_sql(None)
        ),
        params![id, workspace_id, kind],
        Reflink::from_row,
    )
    .optional()
    .map_err(db_error)
}

/// `UPDATE "Reflink" SET ... RETURNING *` of one row of the workspace and type.
pub(crate) fn update_typed(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    kind: &str,
    sets: Vec<(&'static str, Sql)>,
) -> std::result::Result<Option<Reflink>, chatbotx_db::repo::Error> {
    let mut assignments: Vec<String> = Vec::new();
    let mut args: Vec<Sql> = Vec::new();
    for (column, value) in sets {
        args.push(value);
        assignments.push(format!("\"{column}\" = ?{}", args.len()));
    }
    args.push(Sql::Integer(chatbotx_db::repo::now_ms()));
    assignments.push(format!("\"updatedAt\" = ?{}", args.len()));
    let at = args.len();
    args.push(Sql::Integer(id));
    args.push(Sql::Integer(workspace_id));
    args.push(Sql::Text(kind.to_string()));
    let sql = format!(
        "UPDATE \"Reflink\" SET {} WHERE \"id\" = ?{} AND \"workspaceId\" = ?{} AND \"type\" = ?{} RETURNING {}",
        assignments.join(", "),
        at + 1,
        at + 2,
        at + 3,
        Reflink::columns_sql(None)
    );
    Ok(conn
        .query_row(&sql, params_from_iter(args), Reflink::from_row)
        .optional()?)
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflinkPage {
    pub data: Vec<ReflinkWithRelations>,
    pub page_count: i64,
}

/// `ReflinkCreateData` / `ReflinkUpdateData` (`custom_field_id: Some(None)` = null).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReflinkData {
    pub name: Option<String>,
    pub flow_id: Option<i64>,
    pub custom_field_id: Option<Option<i64>>,
}

/// `ReflinkWidgetSettings` (ids as numbers; `logo_file_id: None` = no logo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WidgetSettings {
    pub authorized_domains: Vec<String>,
    pub hidden_inbox_ids: Vec<i64>,
    pub logo_file_id: Option<i64>,
    pub brand_name: String,
    pub brand_url: String,
    pub logo_background_color: String,
}

/// `assertParentFlowsInWorkspace` (`Flow not found`, 404).
pub fn assert_parent_flows_in_workspace(
    conn: &Connection,
    workspace_id: i64,
    flow_ids: &[i64],
) -> Result<()> {
    let mut unique: Vec<i64> = flow_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.is_empty() {
        return Ok(());
    }
    let found: i64 = conn
        .query_row(
            r#"SELECT count(*) FROM "Flow" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, id_list(&unique)],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if found != unique.len() as i64 {
        return Err(BusinessError::not_found("Flow not found"));
    }
    Ok(())
}

/// `list`: one page and the page count.
pub async fn list(db: &Db, input: ReflinkListInput) -> Result<ReflinkPage> {
    tx::read(db, move |conn| {
        let data = repo::list_paginated(conn, &input)?;
        let total = repo::count(conn, input.workspace_id, input.keyword.as_deref())?;
        let per_page = input.per_page.max(1);
        Ok(ReflinkPage {
            data,
            page_count: (total + per_page - 1) / per_page,
        })
    })
    .await
}

/// `findOrFail` (`Reflink not found`).
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Reflink> {
    tx::read(db, move |conn| {
        repo::find_by_id_and_workspace(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `findWidgetOrFail`.
pub async fn find_widget_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<ReflinkWithLogo> {
    tx::read(db, move |conn| {
        repo::find_widget_by_id_and_workspace(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `create` (`type` = `refLink`).
pub async fn create(db: &Db, workspace_id: i64, data: ReflinkData) -> Result<Reflink> {
    tx::write(db, move |conn| {
        let flow_id = data.flow_id.unwrap_or_default();
        assert_parent_flows_in_workspace(conn, workspace_id, &[flow_id])?;
        let id = new_id();
        conn.execute(
            r#"INSERT INTO "Reflink" ("id", "workspaceId", "type", "name", "flowId", "customFieldId") VALUES (?1, ?2, 'refLink', ?3, ?4, ?5)"#,
            params![id, workspace_id, data.name.unwrap_or_default(), flow_id, data.custom_field_id.flatten()],
        )
        .map_err(|e| name_taken(e.into()))?;
        find_typed(conn, workspace_id, id, "refLink")?.ok_or_else(not_found)
    })
    .await
}

/// `update`: nothing sent returns the stored row.
pub async fn update(db: &Db, workspace_id: i64, id: i64, data: ReflinkData) -> Result<Reflink> {
    if data == ReflinkData::default() {
        return find_or_fail(db, workspace_id, id).await;
    }
    tx::write(db, move |conn| {
        find_typed(conn, workspace_id, id, "refLink")?.ok_or_else(not_found)?;
        if let Some(flow_id) = data.flow_id {
            assert_parent_flows_in_workspace(conn, workspace_id, &[flow_id])?;
        }
        let mut sets: Vec<(&'static str, Sql)> = Vec::new();
        if let Some(name) = data.name {
            sets.push(("name", Sql::Text(name)));
        }
        if let Some(flow_id) = data.flow_id {
            sets.push(("flowId", Sql::Integer(flow_id)));
        }
        if let Some(custom_field_id) = data.custom_field_id {
            sets.push((
                "customFieldId",
                custom_field_id.map_or(Sql::Null, Sql::Integer),
            ));
        }
        update_typed(conn, workspace_id, id, "refLink", sets)
            .map_err(name_taken)?
            .ok_or_else(not_found)
    })
    .await
}

/// `updateWidgetSettings`: domains lower-cased and unique, hidden inboxes kept
/// only when they are this workspace's, empty logo / brand stored as null.
pub async fn update_widget_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    settings: WidgetSettings,
) -> Result<Reflink> {
    tx::write(db, move |conn| {
        find_typed(conn, workspace_id, id, "refLink")?.ok_or_else(not_found)?;
        let mut wanted = settings.hidden_inbox_ids.clone();
        wanted.sort_unstable();
        wanted.dedup();
        // `inboxService.listLabelsByIds`: only this workspace's inboxes, by id.
        let mut stmt = conn
            .prepare(r#"SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#)
            .map_err(db_error)?;
        let hidden: Vec<String> = stmt
            .query_map(params![workspace_id, id_list(&wanted)], |r| r.get::<_, i64>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?
            .into_iter()
            .map(|id| id.to_string())
            .collect();
        if let Some(file_id) = settings.logo_file_id {
            let mime: Option<String> = conn
                .query_row(
                    r#"SELECT "mimeType" FROM "MediaLibraryFile" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![file_id, workspace_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if !mime.is_some_and(|m| m.starts_with("image/")) {
                return Err(BusinessError::validation("logoFileId", "Logo must be an image"));
            }
        }
        let mut domains: Vec<String> = Vec::new();
        for domain in settings.authorized_domains.iter().map(|d| d.to_lowercase()) {
            if !domains.contains(&domain) {
                domains.push(domain);
            }
        }
        let text_or_null = |value: &str| {
            if value.is_empty() {
                Sql::Null
            } else {
                Sql::Text(value.to_string())
            }
        };
        let sets = vec![
            ("widgetAuthorizedDomains", Sql::Text(json!(domains).to_string())),
            ("widgetHiddenInboxIds", Sql::Text(json!(hidden).to_string())),
            ("widgetLogoFileId", settings.logo_file_id.map_or(Sql::Null, Sql::Integer)),
            ("widgetBrandName", text_or_null(&settings.brand_name)),
            ("widgetBrandUrl", text_or_null(&settings.brand_url)),
            ("widgetLogoBackgroundColor", Sql::Text(settings.logo_background_color.clone())),
        ];
        update_typed(conn, workspace_id, id, "refLink", sets)?.ok_or_else(not_found)
    })
    .await
}

/// `findForWidget`: `None` when the link is gone or its workspace is frozen.
pub async fn find_for_widget(db: &Db, id: i64) -> Result<Option<ReflinkWithLogo>> {
    tx::read(db, move |conn| {
        let Some(found) = repo::find_by_id(conn, id)? else {
            return Ok(None);
        };
        // `resolveWorkspaceFreezeReasonById` (row reasons; community edition).
        let scheduled: Option<Option<i64>> = conn
            .query_row(
                r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                [found.reflink.workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        Ok(match scheduled {
            Some(None) => Some(found),
            _ => None,
        })
    })
    .await
}

/// `deleteMany` (ref links only).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    tx::write(db, move |conn| {
        delete_in_tx(conn, workspace_id, &ids, Some("refLink")).map(|_| ())
    })
    .await
}

/// `deleteInTx`: the deleted ids; `kind` limits the delete to one type.
pub fn delete_in_tx(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    kind: Option<&str>,
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            r#"DELETE FROM "Reflink" WHERE "workspaceId" = ?1 AND (?2 IS NULL OR "type" = ?2)
               AND "id" IN (SELECT value FROM json_each(?3)) RETURNING "id""#,
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![workspace_id, kind, id_list(ids)], |r| r.get(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<i64>, _>>()
        .map_err(db_error)?;
    Ok(rows)
}
