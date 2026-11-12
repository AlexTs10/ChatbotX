//! Port of `repositories/reflink/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

fn select_reflink() -> String {
    format!(
        "SELECT {}, (SELECT f.\"path\" FROM \"MediaLibraryFile\" f WHERE f.\"id\" = r.\"widgetLogoFileId\")",
        Reflink::columns_sql(Some("r"))
    )
}

/// `count`: same filter as [`list_paginated`].
pub fn count(conn: &Connection, workspace_id: i64, keyword: Option<&str>) -> Result<i64> {
    todo!("P3.3")
}

/// `findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Reflink>> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `listIdsByFlowIds`: ids of the ref links and QR codes of `flow_ids` (one statement).
pub fn list_ids_by_flow_ids(
    conn: &Connection,
    workspace_id: i64,
    flow_ids: &[i64],
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `findById`: unscoped lookup for the public chat widget.
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<ReflinkWithLogo>> {
    todo!("P3.3")
}
