//! Port of `repositories/messaging-ads-connection/repository.ts` (P3.3). `workspaceId` is
//! required on every lookup (the integration FK alone does not prove the workspace).

use rusqlite::Connection;

use super::models::MessagingAdsConnection;
use super::sql::Sql;
use super::{Error, Json, Model, Result, now_ms};

/// `WorkspaceIntegrationFkRef` (at most one integration id is set).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntegrationRef {
    pub workspace_id: i64,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
}

/// `CreateMessagingAdsConnectionInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMessagingAdsConnectionInput {
    pub id: i64,
    pub integration: IntegrationRef,
    pub channel: String,
    pub auth: Json,
}

/// `findForIntegration`: scoped by workspace and the supplied integration FK.
pub fn find_for_integration(
    conn: &Connection,
    input: IntegrationRef,
) -> Result<Option<MessagingAdsConnection>> {
    let mut q = Sql::raw(format!("{} WHERE ", MessagingAdsConnection::select_sql()));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
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
    q.push(" LIMIT 1");
    q.model(conn)
}

/// `listForChannel`: the workspace's `active` connections of one channel.
pub fn list_for_channel(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
) -> Result<Vec<MessagingAdsConnection>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"channel\" = ?2 AND \"status\" = 'active'",
        &[&workspace_id, &channel],
    )
}

/// `create`: inserts an `active` connection.
pub fn create(
    conn: &Connection,
    input: &CreateMessagingAdsConnectionInput,
) -> Result<MessagingAdsConnection> {
    let now = now_ms();
    let i = input.integration;
    super::returning::<MessagingAdsConnection>(
        conn,
        "INSERT INTO \"MessagingAdsConnection\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"channel\", \
         \"integrationWhatsappId\", \"integrationMessengerId\", \"integrationInstagramId\", \"auth\", \"status\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active')",
        &[
            &input.id,
            &now,
            &i.workspace_id,
            &input.channel,
            &i.integration_whatsapp_id,
            &i.integration_messenger_id,
            &i.integration_instagram_id,
            &input.auth,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to create MessagingAdsConnection"))
}

/// `updateAuth`: replaces the token and resets the status to `active`.
pub fn update_auth(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    auth: &Json,
) -> Result<Option<MessagingAdsConnection>> {
    Ok(super::returning::<MessagingAdsConnection>(
        conn,
        "UPDATE \"MessagingAdsConnection\" SET \"auth\" = ?3, \"status\" = 'active', \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id, auth, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `updateStatus`: sets the status of the integration's connection (no-op when none).
pub fn update_status(conn: &Connection, input: IntegrationRef, status: &str) -> Result<()> {
    let Some(existing) = find_for_integration(conn, input)? else {
        return Ok(());
    };
    conn.execute(
        "UPDATE \"MessagingAdsConnection\" SET \"status\" = ?2, \"updatedAt\" = ?3 WHERE \"id\" = ?1",
        rusqlite::params![existing.id, status, now_ms()],
    )?;
    Ok(())
}

/// `remove`.
pub fn remove(conn: &Connection, id: i64, workspace_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MessagingAdsConnection\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id],
    )?;
    Ok(())
}
