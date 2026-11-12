//! Port of `repositories/messaging-ads-connection/repository.ts` (P3.3). `workspaceId` is
//! required on every lookup (the integration FK alone does not prove the workspace).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `listForChannel`: the workspace's `active` connections of one channel.
pub fn list_for_channel(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
) -> Result<Vec<MessagingAdsConnection>> {
    todo!("P3.3")
}

/// `create`: inserts an `active` connection.
pub fn create(
    conn: &Connection,
    input: &CreateMessagingAdsConnectionInput,
) -> Result<MessagingAdsConnection> {
    todo!("P3.3")
}

/// `updateAuth`: replaces the token and resets the status to `active`.
pub fn update_auth(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    auth: &Json,
) -> Result<Option<MessagingAdsConnection>> {
    todo!("P3.3")
}

/// `updateStatus`: sets the status of the integration's connection (no-op when none).
pub fn update_status(conn: &Connection, input: IntegrationRef, status: &str) -> Result<()> {
    todo!("P3.3")
}

/// `remove`.
pub fn remove(conn: &Connection, id: i64, workspace_id: i64) -> Result<()> {
    todo!("P3.3")
}
