//! Port of `repositories/google-ads-settings/repository.ts` (P3.2b).

use rusqlite::Connection;

use super::models::GoogleAdsSettings;
use super::{Error, Json, Result, new_id, now_ms};

/// `findByWorkspaceId`.
pub fn find_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<GoogleAdsSettings>> {
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `upsertSettings`: inserts the document, or replaces only `settings` (and `updatedAt`) on
/// a workspace conflict.
pub fn upsert_settings(
    conn: &Connection,
    workspace_id: i64,
    settings: &Json,
) -> Result<GoogleAdsSettings> {
    let now = now_ms();
    super::returning::<GoogleAdsSettings>(
        conn,
        "INSERT INTO \"GoogleAdsSettings\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \
         \"settings\") VALUES (?1, ?2, ?2, ?3, ?4) ON CONFLICT (\"workspaceId\") DO UPDATE SET \
         \"settings\" = excluded.\"settings\", \"updatedAt\" = excluded.\"updatedAt\"",
        &[&new_id(), &now, &workspace_id, settings],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to upsert Google Ads settings"))
}
