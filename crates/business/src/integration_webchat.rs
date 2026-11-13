//! `integration-webchat/service.ts` (create / find / list / update / delete)
//! and `branding.ts` (`brandWebchatMenus`, community edition).

use chatbotx_db::Db;
use serde_json::Value;

use crate::Result;
use crate::connection::ConnectionRow;

/// `BRANDING_TITLE`.
pub const BRANDING_TITLE: &str = "⚡ Built with chatbotx.io";

/// `CreateWebchatRequest` after the zod defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct WebchatData {
    pub name: String,
    pub welcome_flow_id: Option<i64>,
    pub authorized_domains: Vec<String>,
    /// `webchatConversationStarter` objects (already validated).
    pub conversation_starters: Vec<Value>,
    /// `webchatPersistentMenu` objects (already validated).
    pub persistent_menus: Vec<Value>,
    pub brand_color: String,
    pub hide_header: bool,
    pub show_logo: bool,
    pub hide_message_input: bool,
    pub custom_css: Option<String>,
    pub enable: bool,
}

/// `UpdateWebchatRequest`: `None` = key omitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebchatPatch {
    pub name: Option<String>,
    pub welcome_flow_id: Option<Option<i64>>,
    pub authorized_domains: Option<Vec<String>>,
    pub conversation_starters: Option<Vec<Value>>,
    pub persistent_menus: Option<Vec<Value>>,
    pub brand_color: Option<String>,
    pub hide_header: Option<bool>,
    pub show_logo: Option<bool>,
    pub hide_message_input: Option<bool>,
    pub custom_css: Option<Option<String>>,
    pub enable: Option<bool>,
}

/// An `IntegrationWebchat` row (without `workspaceId` and `auth`).
#[derive(Debug, Clone, PartialEq)]
pub struct WebchatRow {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: String,
    pub enable: bool,
    pub authorized_domains: Vec<String>,
    pub conversation_starters: Vec<Value>,
    pub persistent_menus: Vec<Value>,
    pub brand_color: String,
    pub hide_header: bool,
    pub show_logo: bool,
    pub hide_message_input: bool,
    pub custom_css: Option<String>,
    pub inbox_id: i64,
    pub welcome_flow_id: Option<i64>,
}

/// `buildBrandingUrl(appUrl, "webchat", true)`.
pub fn branding_url(_app_url: &str) -> String {
    todo!("P3.6-D2")
}

/// `brandWebchatMenus` (community edition): the branding URL menu last,
/// exactly once.
pub fn brand_webchat_menus(_menus: Vec<Value>, _app_url: &str) -> Vec<Value> {
    todo!("P3.6-D2")
}

/// `integrationWebchatService.create` (existing workspace): the welcome-flow
/// check (`404 Welcome flow not found`), the inbox, the webchat row and the
/// `connected` connection in one transaction.
pub async fn create(
    _db: &Db,
    _workspace_id: i64,
    _created_by: Option<i64>,
    _data: WebchatData,
    _app_url: &str,
    _now: i64,
) -> Result<(WebchatRow, ConnectionRow)> {
    todo!("P3.6-D2")
}

/// `findByIdForWorkspace` (`404 Integration Webchat not found`).
pub async fn find_by_id_for_workspace(_db: &Db, _id: i64, _workspace_id: i64) -> Result<WebchatRow> {
    todo!("P3.6-D2")
}

/// `list`: rows ordered by id, one page; plus the page count.
pub async fn list(_db: &Db, _workspace_id: i64, _page: i64, _per_page: i64) -> Result<(Vec<WebchatRow>, i64)> {
    todo!("P3.6-D2")
}

/// `update`: the supplied keys only (menus branded); a new name also renames
/// the connection.
pub async fn update(
    _db: &Db,
    _workspace_id: i64,
    _id: i64,
    _patch: WebchatPatch,
    _app_url: &str,
) -> Result<()> {
    todo!("P3.6-D2")
}

/// `delete`: the webchat row and the connection / inbox disconnect.
pub async fn delete(_db: &Db, _workspace_id: i64, _id: i64) -> Result<()> {
    todo!("P3.6-D2")
}
