//! Port of `repositories/facebook-marketing-messages/audience-repository.ts` (P3.2b).

use rusqlite::Connection;

use super::models::FacebookMarketingMessageAudience;
use super::sql::Sql;
use super::{Error, Result, id_list, now_ms};

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

const TABLE: &str = "\"FacebookMarketingMessageAudience\"";

/// `listByWorkspaceId` (createdAt DESC, id DESC).
pub fn list_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
    filter: &ListFilter,
) -> Result<Vec<FacebookMarketingMessageAudience>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM {TABLE} WHERE ",
        <FacebookMarketingMessageAudience as super::Model>::columns_sql(None)
    ));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        filter
            .ad_account_id
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|a| Sql::bind("\"adAccountId\" = ", a.clone())),
        filter
            .page_id
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|p| Sql::bind("\"pageId\" = ", p.clone())),
        filter
            .ready_only
            .then(|| Sql::raw("\"readyAt\" IS NOT NULL")),
    ]));
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC");
    q.models(conn)
}

/// `findForWorkspace`.
pub fn find_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<FacebookMarketingMessageAudience>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `create`.
pub fn create(conn: &Connection, input: &CreateInput) -> Result<FacebookMarketingMessageAudience> {
    super::returning::<FacebookMarketingMessageAudience>(
        conn,
        &format!(
            "INSERT INTO {TABLE} (\"id\", \"workspaceId\", \"customAudienceId\", \"adAccountId\", \
             \"pageId\", \"name\", \"description\", \"facebookUserId\", \"rowsUploaded\") \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)"
        ),
        &[
            &input.id,
            &input.workspace_id,
            &input.custom_audience_id,
            &input.ad_account_id,
            &input.page_id,
            &input.name,
            &input.description,
            &input.facebook_user_id,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to create FacebookMarketingMessageAudience"))
}

/// `incrementRowsUploaded` (atomic add).
pub fn increment_rows_uploaded(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    by: i64,
) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"rowsUploaded\" = \"rowsUploaded\" + ?1, \"updatedAt\" = ?2 \
             WHERE \"id\" = ?3 AND \"workspaceId\" = ?4"
        ),
        [by, now_ms(), id, workspace_id],
    )?;
    Ok(())
}

/// `markReady`.
pub fn mark_ready(conn: &Connection, ids: &[i64], workspace_id: i64, ready_at: i64) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"readyAt\" = ?1, \"updatedAt\" = ?2 WHERE \"id\" IN \
             (SELECT value FROM json_each(?3)) AND \"workspaceId\" = ?4"
        ),
        rusqlite::params![ready_at, now_ms(), id_list(ids), workspace_id],
    )?;
    Ok(())
}

/// `remove`.
pub fn remove(conn: &Connection, id: i64, workspace_id: i64) -> Result<()> {
    conn.execute(
        &format!("DELETE FROM {TABLE} WHERE \"id\" = ?1 AND \"workspaceId\" = ?2"),
        [id, workspace_id],
    )?;
    Ok(())
}
