//! `packages/business/src/connection` + `packages/connections/src` (the
//! self-serve part: `api` and `webchat` channels; P3.6-D2). The provider
//! catalog, credential / OAuth connects and connect sessions come later.

pub mod providers;

use chatbotx_db::Db;

use crate::Result;

/// A `Connection` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub kind: String,
    pub channel: Option<String>,
    pub inbox_id: Option<i64>,
    pub integration_id: Option<i64>,
    pub source_id: String,
    pub display_name: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub last_error: Option<String>,
    pub auth_expires_at: Option<i64>,
    pub connected_at: Option<i64>,
    pub disconnected_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `connectionStateService.list` input (`status` is one status here).
#[derive(Debug, Clone, Default)]
pub struct ListConnections {
    pub workspace_id: i64,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub channel: Option<String>,
    pub status: Option<String>,
    pub page: i64,
    pub per_page: i64,
}

/// `connectionStateService.list`: rows ordered by kind, provider, display
/// name, id; plus the total count.
pub async fn list(_db: &Db, _input: ListConnections) -> Result<(Vec<ConnectionRow>, i64)> {
    todo!("P3.6-D2")
}

/// `connectionStateService.getForWorkspace`.
pub async fn get_for_workspace(_db: &Db, _id: i64, _workspace_id: i64) -> Result<Option<ConnectionRow>> {
    todo!("P3.6-D2")
}

/// `connectionStateService.updateDisplayName` (the name is already trimmed).
pub async fn update_display_name(
    _db: &Db,
    _id: i64,
    _workspace_id: i64,
    _display_name: &str,
) -> Result<ConnectionRow> {
    todo!("P3.6-D2")
}

/// `connectionService.disconnect` for a self-serve channel: `user.disconnect`
/// (status `disconnected`, reason `manual`), the inbox mirror, the satellite
/// row kept (`onDisconnect: "keep_row"`).
pub async fn disconnect(_db: &Db, _id: i64, _workspace_id: i64) -> Result<ConnectionRow> {
    todo!("P3.6-D2")
}

/// `connectionService.verify` for a self-serve channel (`verify` is always
/// ok): `verify.ok`. An inactive connection is `connectionInactive`.
pub async fn verify(_db: &Db, _id: i64, _workspace_id: i64) -> Result<ConnectionRow> {
    todo!("P3.6-D2")
}

/// `connectionService.refresh`: inactive = `connectionInactive`; a provider
/// without `refreshAuth` = `connectionNotRefreshable`.
pub async fn refresh(_db: &Db, _id: i64, _workspace_id: i64) -> Result<ConnectionRow> {
    todo!("P3.6-D2")
}

/// `startReconnect` up to the OAuth part: a provider that is not OAuth (or
/// has no platform credential) is `connectionNotConfigured`.
pub async fn reconnect(_db: &Db, _connection: &ConnectionRow) -> Result<ConnectionRow> {
    todo!("P3.6-D2")
}
