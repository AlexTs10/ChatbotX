//! Port of `repositories/google-ads-settings/repository.ts` (P3.2b).

use rusqlite::Connection;

use super::models::GoogleAdsSettings;
use super::{Json, Result};

/// `findByWorkspaceId`.
pub fn find_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<GoogleAdsSettings>> {
    let _ = (conn, workspace_id);
    todo!("P3.2b")
}

/// `upsertSettings`: inserts the document, or replaces only `settings` on a workspace
/// conflict.
pub fn upsert_settings(
    conn: &Connection,
    workspace_id: i64,
    settings: &Json,
) -> Result<GoogleAdsSettings> {
    let _ = (conn, workspace_id, settings);
    todo!("P3.2b")
}
