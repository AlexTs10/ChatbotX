//! Port of `repositories/integration-google-ads/repository.ts` (P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::IntegrationGoogleAds;
use super::patch::Patch;

/// `GoogleAdsSetupValues` keys (the columns `update_setup` may set).
pub const SETUP_FIELDS: &[&str] = &[
    "conversionCustomerId",
    "acceptedCustomerDataTerms",
    "conversionActions",
    "conversionActionsSyncedAt",
    "setupError",
    "setupErrorAt",
];

/// One row of `listSyncTargets`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncTarget {
    pub id: i64,
    pub workspace_id: i64,
}

/// `findByWorkspaceId`.
pub fn find_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<IntegrationGoogleAds>> {
    let _ = (conn, workspace_id);
    todo!("P3.2b")
}

/// `existsByWorkspaceId`.
pub fn exists_by_workspace_id(conn: &Connection, workspace_id: i64) -> Result<bool> {
    let _ = (conn, workspace_id);
    todo!("P3.2b")
}

/// `updateSetup`: writes only the given values (checked against [`SETUP_FIELDS`]).
pub fn update_setup(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<IntegrationGoogleAds>> {
    let _ = (conn, id, workspace_id, values);
    todo!("P3.2b")
}

/// `listSyncTargets`: keyset page by id ascending (`after_id` exclusive).
pub fn list_sync_targets(
    conn: &Connection,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<SyncTarget>> {
    let _ = (conn, after_id, limit);
    todo!("P3.2b")
}

/// `listConnectedCustomerIds`: distinct customer ids.
pub fn list_connected_customer_ids(conn: &Connection) -> Result<Vec<String>> {
    let _ = conn;
    todo!("P3.2b")
}
