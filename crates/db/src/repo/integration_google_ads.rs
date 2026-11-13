//! Port of `repositories/integration-google-ads/repository.ts` (P3.2b).

use rusqlite::Connection;

use super::models::IntegrationGoogleAds;
use super::patch::{self, Patch};
use super::sql::Sql;
use super::{Result, now_ms};

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
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `existsByWorkspaceId`.
pub fn exists_by_workspace_id(conn: &Connection, workspace_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"IntegrationGoogleAds\" WHERE \"workspaceId\" = ?1)",
        [workspace_id],
        |r| r.get::<_, bool>(0),
    )?)
}

/// `updateSetup`: writes only the given values (checked against [`SETUP_FIELDS`]) and
/// `updatedAt`.
pub fn update_setup(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<IntegrationGoogleAds>> {
    let mut q = Sql::raw("UPDATE \"IntegrationGoogleAds\" SET ");
    if !values.is_empty() {
        q.append(patch::set_clause(SETUP_FIELDS, values)?);
        q.push(", ");
    }
    q.push("\"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(
        " RETURNING {}",
        <IntegrationGoogleAds as super::Model>::columns_sql(None)
    ));
    q.model(conn)
}

/// `listSyncTargets`: keyset page by id ascending (`after_id` exclusive).
pub fn list_sync_targets(
    conn: &Connection,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<SyncTarget>> {
    let mut q = Sql::raw("SELECT \"id\", \"workspaceId\" FROM \"IntegrationGoogleAds\"");
    if let Some(after) = after_id {
        q.push(" WHERE \"id\" > ");
        q.push_bind(after);
    }
    q.push(" ORDER BY \"id\" ASC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |r| {
        Ok(SyncTarget {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
        })
    })
}

/// `listConnectedCustomerIds`: distinct customer ids (by customer id).
pub fn list_connected_customer_ids(conn: &Connection) -> Result<Vec<String>> {
    Sql::raw("SELECT DISTINCT \"customerId\" FROM \"IntegrationGoogleAds\" ORDER BY \"customerId\"")
        .query_map(conn, |r| r.get(0))
}
