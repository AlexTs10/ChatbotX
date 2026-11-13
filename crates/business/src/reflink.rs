//! `reflink/service.ts` (`reflinkService`). Ref links and QR codes share the
//! `"Reflink"` table (`type` = `refLink` / `qrCode`). Template edit locks are
//! not checked (DECISIONS, P3.6-D1); the parent-flow workspace check of
//! `assertReferencable` is ported ([`assert_parent_flows_in_workspace`]).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Reflink;
use chatbotx_db::rusqlite::Connection;

pub use chatbotx_db::repo::reflink::{ReflinkListInput, ReflinkWithLogo, ReflinkWithRelations};

use crate::errors::Result;

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
    let _ = (conn, workspace_id, flow_ids);
    todo!("P3.6-D1")
}

/// `list`: one page and the page count.
pub async fn list(db: &Db, input: ReflinkListInput) -> Result<ReflinkPage> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `findOrFail` (`Reflink not found`).
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Reflink> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D1")
}

/// `findWidgetOrFail`.
pub async fn find_widget_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<ReflinkWithLogo> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D1")
}

/// `create` (`type` = `refLink`).
pub async fn create(db: &Db, workspace_id: i64, data: ReflinkData) -> Result<Reflink> {
    let _ = (db, workspace_id, data);
    todo!("P3.6-D1")
}

/// `update`: nothing sent returns the stored row.
pub async fn update(db: &Db, workspace_id: i64, id: i64, data: ReflinkData) -> Result<Reflink> {
    let _ = (db, workspace_id, id, data);
    todo!("P3.6-D1")
}

/// `updateWidgetSettings`: domains lower-cased and unique, hidden inboxes kept
/// only when they are this workspace's, empty logo / brand stored as null.
pub async fn update_widget_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    settings: WidgetSettings,
) -> Result<Reflink> {
    let _ = (db, workspace_id, id, settings);
    todo!("P3.6-D1")
}

/// `findForWidget`: `None` when the link is gone or its workspace is frozen.
pub async fn find_for_widget(db: &Db, id: i64) -> Result<Option<ReflinkWithLogo>> {
    let _ = (db, id);
    todo!("P3.6-D1")
}

/// `deleteMany` (ref links only).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D1")
}

/// `deleteInTx`: the deleted ids; `kind` limits the delete to one type.
pub fn delete_in_tx(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    kind: Option<&str>,
) -> Result<Vec<i64>> {
    let _ = (conn, workspace_id, ids, kind);
    todo!("P3.6-D1")
}
