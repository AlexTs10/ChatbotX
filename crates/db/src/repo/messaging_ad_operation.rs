//! Port of `repositories/messaging-ad-operation/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<MessagingAdOperation>> {
    todo!("P3.3")
}

/// `listByWorkspaceId`: newest first, optionally scoped to one channel + integration.
pub fn list_by_workspace_id(
    conn: &Connection,
    input: &ListOperationsInput,
) -> Result<Vec<MessagingAdOperation>> {
    todo!("P3.3")
}

/// `claimForRetry`: CAS `failed` -> `pending` (clears `lastError`).
pub fn claim_for_retry(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<MessagingAdOperation>> {
    todo!("P3.3")
}

/// `updateCreateProgress`: writes the state, the given meta ids and `lastError`.
pub fn update_create_progress(
    conn: &Connection,
    input: &UpdateCreateProgressInput,
) -> Result<Option<MessagingAdOperation>> {
    todo!("P3.3")
}

/// `updatePublishState`.
pub fn update_publish_state(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    publish_state: &str,
    last_error: Option<&str>,
) -> Result<Option<MessagingAdOperation>> {
    todo!("P3.3")
}

/// `setCleanupError`.
pub fn set_cleanup_error(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    cleanup_error: Option<&str>,
) -> Result<()> {
    todo!("P3.3")
}
