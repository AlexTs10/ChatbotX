//! Port of `repositories/workspace-api-token/repository.ts` and the scope enum of
//! `partials/workspace-api-token.ts` (P3.3).
//!
//! `lockWorkspaceTokens` (`pg_advisory_xact_lock`) is a no-op: the one SQLite writer
//! serializes the count-then-insert of a create.

use rusqlite::Connection;

use super::models::WorkspaceApiToken;
use super::{Json, Result};

/// `workspaceApiTokenPermissions`.
pub const WORKSPACE_API_TOKEN_PERMISSIONS: &[&str] = &["full", "read_only"];

/// `workspaceApiTokenScopes` (declaration order).
pub const WORKSPACE_API_TOKEN_SCOPES: &[&str] = &[
    "contacts",
    "inbox",
    "automation",
    "broadcasts",
    "analytics",
    "ecommerce",
    "channels",
    "integrations",
    "minigames",
    "appointments",
    "media",
    "ads",
    "settings",
    "mini-apps",
    "leaderboards",
    "pipelines",
    "places",
    "polls",
    "templates",
];

/// `InsertWorkspaceApiTokenInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertWorkspaceApiTokenInput {
    pub workspace_id: i64,
    pub token_hash: String,
    pub name: String,
    pub permission: String,
    pub token_prefix: String,
    /// `None` = unrestricted ("All scopes").
    pub scopes: Option<Vec<String>>,
}

/// `InsertDefaultWorkspaceApiTokenInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertDefaultWorkspaceApiTokenInput {
    pub workspace_id: i64,
    pub token_hash: String,
    pub name: String,
    pub token_prefix: String,
    pub encrypted_token: Json,
}

/// `normalizeWorkspaceApiTokenScopes`: keeps a fully valid list as is; otherwise logs a
/// warning and keeps the known scopes once each, in order.
pub fn normalize_scopes(scopes: Option<&Json>) -> Option<Json> {
    let _ = scopes;
    todo!("P3.3")
}

/// `findByTokenHash` (scopes normalized).
pub fn find_by_token_hash(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<WorkspaceApiToken>> {
    let _ = (conn, token_hash);
    todo!("P3.3")
}

/// `listByWorkspaceId`: newest first (scopes normalized).
pub fn list_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<WorkspaceApiToken>> {
    let _ = (conn, workspace_id);
    todo!("P3.3")
}

/// `countByWorkspaceId`.
pub fn count_by_workspace_id(conn: &Connection, workspace_id: i64) -> Result<i64> {
    let _ = (conn, workspace_id);
    todo!("P3.3")
}

/// `lockWorkspaceTokens`: a no-op under the single writer (see the module doc).
pub fn lock_workspace_tokens(conn: &Connection, workspace_id: i64) -> Result<()> {
    let _ = (conn, workspace_id);
    todo!("P3.3")
}

/// `deleteByIdForWorkspace`: `true` when a row of that workspace was deleted.
pub fn delete_by_id_for_workspace(conn: &Connection, id: i64, workspace_id: i64) -> Result<bool> {
    let _ = (conn, id, workspace_id);
    todo!("P3.3")
}

/// `insert`: inserts and returns the new row.
pub fn insert(
    conn: &Connection,
    input: &InsertWorkspaceApiTokenInput,
) -> Result<WorkspaceApiToken> {
    let _ = (conn, input);
    todo!("P3.3")
}

/// `findDefaultByWorkspaceId` (scopes normalized).
pub fn find_default_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<WorkspaceApiToken>> {
    let _ = (conn, workspace_id);
    todo!("P3.3")
}

/// `insertDefault`: inserts the default `full` token; `None` when another default row of
/// the workspace already exists (partial unique index on `workspaceId WHERE isDefault`).
pub fn insert_default(
    conn: &Connection,
    input: &InsertDefaultWorkspaceApiTokenInput,
) -> Result<Option<WorkspaceApiToken>> {
    let _ = (conn, input);
    todo!("P3.3")
}

/// `setEncryptedToken`: sets the blob only while it is still NULL.
pub fn set_encrypted_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    encrypted_token: &Json,
) -> Result<()> {
    let _ = (conn, id, workspace_id, encrypted_token);
    todo!("P3.3")
}
