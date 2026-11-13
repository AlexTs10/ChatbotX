//! Port of `repositories/facebook-marketing-messages/audience-repository.ts` (P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::FacebookMarketingMessageAudience;

/// `listByWorkspaceId` filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    pub ad_account_id: Option<String>,
    pub page_id: Option<String>,
    pub ready_only: bool,
}

/// `create` input (`rowsUploaded` = 0).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateInput {
    pub id: i64,
    pub workspace_id: i64,
    pub custom_audience_id: String,
    pub ad_account_id: String,
    pub page_id: String,
    pub name: String,
    pub description: Option<String>,
    pub facebook_user_id: Option<String>,
}

/// `listByWorkspaceId` (createdAt DESC).
pub fn list_by_workspace_id(
    _conn: &Connection,
    _workspace_id: i64,
    _filter: &ListFilter,
) -> Result<Vec<FacebookMarketingMessageAudience>> {
    todo!("P3.2b")
}

/// `findForWorkspace`.
pub fn find_for_workspace(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
) -> Result<Option<FacebookMarketingMessageAudience>> {
    todo!("P3.2b")
}

/// `create`.
pub fn create(
    _conn: &Connection,
    _input: &CreateInput,
) -> Result<FacebookMarketingMessageAudience> {
    todo!("P3.2b")
}

/// `incrementRowsUploaded` (atomic add).
pub fn increment_rows_uploaded(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _by: i64,
) -> Result<()> {
    todo!("P3.2b")
}

/// `markReady`.
pub fn mark_ready(
    _conn: &Connection,
    _ids: &[i64],
    _workspace_id: i64,
    _ready_at: i64,
) -> Result<()> {
    todo!("P3.2b")
}

/// `remove`.
pub fn remove(_conn: &Connection, _id: i64, _workspace_id: i64) -> Result<()> {
    todo!("P3.2b")
}
