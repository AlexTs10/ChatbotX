//! Port of `repositories/messaging-ad-operation/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::MessagingAdOperation;
use super::sql::Sql;
use super::{Error, Json, Model, Result, now_ms};

/// `CreateMessagingAdOperationInput` (`id` is the caller's operation id).
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMessagingAdOperationInput {
    pub id: i64,
    pub workspace_id: i64,
    pub channel: String,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub ad_account_id: String,
    pub name: String,
    pub input: Json,
    pub created_by: Option<i64>,
}

/// `UpdateCreateProgressInput`: a `None` meta id is not written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateCreateProgressInput {
    pub id: i64,
    pub workspace_id: i64,
    pub create_state: String,
    pub meta_campaign_id: Option<String>,
    pub meta_ad_set_id: Option<String>,
    pub meta_ad_creative_id: Option<String>,
    pub meta_ad_id: Option<String>,
    pub last_error: Option<String>,
}

/// Filter of [`list_by_workspace_id`] (unset / `None` filters are not applied).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListOperationsInput {
    pub workspace_id: i64,
    pub channel: Option<String>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
}

/// `create`: inserts the operation record before the first Graph POST.
pub fn create(
    conn: &Connection,
    input: &CreateMessagingAdOperationInput,
) -> Result<MessagingAdOperation> {
    let now = now_ms();
    super::returning::<MessagingAdOperation>(
        conn,
        "INSERT INTO \"MessagingAdOperation\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"channel\", \
         \"integrationWhatsappId\", \"integrationMessengerId\", \"integrationInstagramId\", \"adAccountId\", \"name\", \
         \"input\", \"createdBy\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        &[
            &input.id,
            &now,
            &input.workspace_id,
            &input.channel,
            &input.integration_whatsapp_id,
            &input.integration_messenger_id,
            &input.integration_instagram_id,
            &input.ad_account_id,
            &input.name,
            &input.input,
            &input.created_by,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to create MessagingAdOperation"))
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<MessagingAdOperation>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `listByWorkspaceId`: newest first, optionally scoped to one channel + integration.
pub fn list_by_workspace_id(
    conn: &Connection,
    input: &ListOperationsInput,
) -> Result<Vec<MessagingAdOperation>> {
    let mut q = Sql::raw(format!("{} WHERE ", MessagingAdOperation::select_sql()));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        input
            .channel
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(|c| Sql::bind("\"channel\" = ", c.to_owned())),
        input
            .integration_whatsapp_id
            .map(|id| Sql::bind("\"integrationWhatsappId\" = ", id)),
        input
            .integration_messenger_id
            .map(|id| Sql::bind("\"integrationMessengerId\" = ", id)),
        input
            .integration_instagram_id
            .map(|id| Sql::bind("\"integrationInstagramId\" = ", id)),
    ]));
    q.push(" ORDER BY \"createdAt\" DESC");
    q.models(conn)
}

/// `claimForRetry`: CAS `failed` -> `pending` (clears `lastError`).
pub fn claim_for_retry(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<MessagingAdOperation>> {
    Ok(super::returning::<MessagingAdOperation>(
        conn,
        "UPDATE \"MessagingAdOperation\" SET \"createState\" = 'pending', \"lastError\" = NULL, \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"createState\" = 'failed'",
        &[&id, &workspace_id, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `updateCreateProgress`: writes the state, the given meta ids and `lastError`.
pub fn update_create_progress(
    conn: &Connection,
    input: &UpdateCreateProgressInput,
) -> Result<Option<MessagingAdOperation>> {
    let mut q = Sql::bind(
        "UPDATE \"MessagingAdOperation\" SET \"createState\" = ",
        input.create_state.clone(),
    );
    for (col, value) in [
        ("metaCampaignId", &input.meta_campaign_id),
        ("metaAdSetId", &input.meta_ad_set_id),
        ("metaAdCreativeId", &input.meta_ad_creative_id),
        ("metaAdId", &input.meta_ad_id),
    ] {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            q.push(&format!(", \"{col}\" = "));
            q.push_bind(v.to_owned());
        }
    }
    q.push(", \"lastError\" = ");
    q.push_bind(super::sql::opt_text(input.last_error.as_deref()));
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(input.id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(input.workspace_id);
    q.push(&format!(
        " RETURNING {}",
        MessagingAdOperation::columns_sql(None)
    ));
    q.model(conn)
}

/// `updatePublishState`.
pub fn update_publish_state(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    publish_state: &str,
    last_error: Option<&str>,
) -> Result<Option<MessagingAdOperation>> {
    Ok(super::returning::<MessagingAdOperation>(
        conn,
        "UPDATE \"MessagingAdOperation\" SET \"publishState\" = ?3, \"lastError\" = ?4, \"updatedAt\" = ?5 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id, &publish_state, &last_error, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `setCleanupError`.
pub fn set_cleanup_error(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    cleanup_error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE \"MessagingAdOperation\" SET \"cleanupError\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id, cleanup_error, now_ms()],
    )?;
    Ok(())
}
