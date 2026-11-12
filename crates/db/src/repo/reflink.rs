//! Port of `repositories/reflink/repository.ts` (P3.3).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{CustomField, Flow, Reflink};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, get_pagination_with_defaults, like_contains, order_by_sql,
    parse_order_by,
};
use super::{Model, Result, id_list};

/// `ReflinkListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReflinkListInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub page: i64,
    pub per_page: i64,
    pub sort: Option<Vec<SortItem>>,
}

/// A reflink with the relations `listPaginated` loads (`flow`, `customField`,
/// `widgetLogoFile.path`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReflinkWithRelations {
    pub reflink: Reflink,
    pub flow: Option<Flow>,
    pub custom_field: Option<CustomField>,
    pub widget_logo_file_path: Option<String>,
}

/// A reflink with `widgetLogoFile.path`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflinkWithLogo {
    pub reflink: Reflink,
    pub widget_logo_file_path: Option<String>,
}

fn build_where(workspace_id: i64, keyword: Option<&str>) -> Sql {
    Sql::and([
        Some(Sql::bind("r.\"workspaceId\" = ", workspace_id)),
        Some(Sql::raw("r.\"type\" = 'refLink'")),
        keyword
            .filter(|k| !k.is_empty())
            .map(|k| Sql::bind("ilike(r.\"name\", ", like_contains(k)).closed(")")),
    ])
}

/// `listPaginated`: the workspace's ref links (never QR codes) with their relations.
pub fn list_paginated(
    conn: &Connection,
    input: &ReflinkListInput,
) -> Result<Vec<ReflinkWithRelations>> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: Some(input.page),
        per_page: Some(input.per_page),
    });
    let order = parse_order_by(Reflink::COLUMNS, input.sort.as_deref(), None);
    let mut q = Sql::raw(format!("{} FROM \"Reflink\" r WHERE ", select_reflink()));
    q.append(build_where(input.workspace_id, input.keyword.as_deref()));
    q.push(" ");
    q.push(&order_by_sql(&order, Some("r")));
    q.push(" LIMIT ");
    q.push_bind(pagination.limit);
    q.push(" OFFSET ");
    q.push_bind(pagination.offset);
    let rows = q.query_map(conn, |row| {
        Ok((
            Reflink::from_row(row)?,
            row.get::<_, Option<String>>(Reflink::COLUMNS.len())?,
        ))
    })?;
    let mut out = Vec::with_capacity(rows.len());
    for (reflink, widget_logo_file_path) in rows {
        let flow = super::find_one::<Flow>(conn, "\"id\" = ?1", &[&reflink.flow_id])?;
        let custom_field = match reflink.custom_field_id {
            Some(id) => super::find_one::<CustomField>(conn, "\"id\" = ?1", &[&id])?,
            None => None,
        };
        out.push(ReflinkWithRelations {
            reflink,
            flow,
            custom_field,
            widget_logo_file_path,
        });
    }
    Ok(out)
}

fn select_reflink() -> String {
    format!(
        "SELECT {}, (SELECT f.\"path\" FROM \"MediaLibraryFile\" f WHERE f.\"id\" = r.\"widgetLogoFileId\")",
        Reflink::columns_sql(Some("r"))
    )
}

/// `count`: same filter as [`list_paginated`].
pub fn count(conn: &Connection, workspace_id: i64, keyword: Option<&str>) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Reflink\" r WHERE ");
    q.append(build_where(workspace_id, keyword));
    Ok(q.query_map(conn, |row| row.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Reflink>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"type\" = 'refLink'",
        &[&id, &workspace_id],
    )
}

fn with_logo(conn: &Connection, filter: Sql) -> Result<Option<ReflinkWithLogo>> {
    let mut q = Sql::raw(format!("{} FROM \"Reflink\" r WHERE ", select_reflink()));
    q.append(filter);
    q.push(" LIMIT 1");
    Ok(q.query_map(conn, |row| {
        Ok(ReflinkWithLogo {
            reflink: Reflink::from_row(row)?,
            widget_logo_file_path: row.get(Reflink::COLUMNS.len())?,
        })
    })?
    .into_iter()
    .next())
}

/// `findWidgetByIdAndWorkspace`: workspace-scoped lookup with the widget logo path.
pub fn find_widget_by_id_and_workspace(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<ReflinkWithLogo>> {
    with_logo(
        conn,
        Sql::new(
            "r.\"id\" = ? AND r.\"workspaceId\" = ? AND r.\"type\" = 'refLink'",
            vec![Value::Integer(id), Value::Integer(workspace_id)],
        ),
    )
}

/// `listIdsByFlowIds`: ids of the ref links and QR codes of `flow_ids` (one statement).
pub fn list_ids_by_flow_ids(
    conn: &Connection,
    workspace_id: i64,
    flow_ids: &[i64],
) -> Result<Vec<i64>> {
    if flow_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"Reflink\" WHERE \"workspaceId\" = ?1 AND \"flowId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let ids = stmt
        .query_map(rusqlite::params![workspace_id, id_list(flow_ids)], |row| {
            row.get(0)
        })?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// `findById`: unscoped lookup for the public chat widget.
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<ReflinkWithLogo>> {
    with_logo(
        conn,
        Sql::new(
            "r.\"id\" = ? AND r.\"type\" = 'refLink'",
            vec![Value::Integer(id)],
        ),
    )
}
