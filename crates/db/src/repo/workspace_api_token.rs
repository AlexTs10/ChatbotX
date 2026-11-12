//! Port of `repositories/workspace-api-token/repository.ts` and the scope enum of
//! `partials/workspace-api-token.ts` (P3.3).
//!
//! `lockWorkspaceTokens` (`pg_advisory_xact_lock`) is a no-op: the one SQLite writer
//! serializes the count-then-insert of a create.

use rusqlite::Connection;

use super::models::WorkspaceApiToken;
use super::{Json, Result, new_id, now_ms};

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
    let value = scopes?;
    if value.is_null() {
        return None;
    }
    let items = value.as_array();
    let valid = items.is_some_and(|list| {
        list.iter().all(|s| {
            s.as_str()
                .is_some_and(|s| WORKSPACE_API_TOKEN_SCOPES.contains(&s))
        })
    });
    if valid {
        return Some(value.clone());
    }
    tracing::warn!(
        err = "invalid workspace API token scopes",
        "Dropping unrecognized workspace API token scopes"
    );
    let mut kept: Vec<serde_json::Value> = Vec::new();
    for scope in items.into_iter().flatten() {
        if let Some(name) = scope.as_str()
            && WORKSPACE_API_TOKEN_SCOPES.contains(&name)
            && !kept.iter().any(|k| k.as_str() == Some(name))
        {
            kept.push(scope.clone());
        }
    }
    Some(Json(serde_json::Value::Array(kept)))
}

fn normalized(mut row: WorkspaceApiToken) -> WorkspaceApiToken {
    row.scopes = normalize_scopes(row.scopes.as_ref());
    row
}

/// `findByTokenHash` (scopes normalized).
pub fn find_by_token_hash(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<WorkspaceApiToken>> {
    Ok(
        super::find_one::<WorkspaceApiToken>(conn, "\"tokenHash\" = ?1", &[&token_hash])?
            .map(normalized),
    )
}

/// `listByWorkspaceId`: newest first (scopes normalized).
pub fn list_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<WorkspaceApiToken>> {
    Ok(super::find_many::<WorkspaceApiToken>(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" DESC",
        &[&workspace_id],
    )?
    .into_iter()
    .map(normalized)
    .collect())
}

/// `countByWorkspaceId`.
pub fn count_by_workspace_id(conn: &Connection, workspace_id: i64) -> Result<i64> {
    super::client::count_where(
        conn,
        "WorkspaceApiToken",
        Some("\"workspaceId\" = ?1"),
        &[&workspace_id],
    )
}

/// `lockWorkspaceTokens`: a no-op under the single writer (see the module doc).
pub fn lock_workspace_tokens(conn: &Connection, workspace_id: i64) -> Result<()> {
    let _ = (conn, workspace_id);
    Ok(())
}

/// `deleteByIdForWorkspace`: `true` when a row of that workspace was deleted.
pub fn delete_by_id_for_workspace(conn: &Connection, id: i64, workspace_id: i64) -> Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM \"WorkspaceApiToken\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id],
    )?;
    Ok(deleted > 0)
}

/// `insert`: inserts and returns the new row.
pub fn insert(
    conn: &Connection,
    input: &InsertWorkspaceApiTokenInput,
) -> Result<WorkspaceApiToken> {
    let now = now_ms();
    let scopes = match &input.scopes {
        Some(list) => Some(super::json_text(list)?),
        None => None,
    };
    super::returning::<WorkspaceApiToken>(
        conn,
        "INSERT INTO \"WorkspaceApiToken\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"tokenHash\", \
         \"name\", \"permission\", \"tokenPrefix\", \"scopes\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.token_hash,
            &input.name,
            &input.permission,
            &input.token_prefix,
            &scopes,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to insert workspace API token"))
}

/// `findDefaultByWorkspaceId` (scopes normalized).
pub fn find_default_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<WorkspaceApiToken>> {
    Ok(super::find_one::<WorkspaceApiToken>(
        conn,
        "\"workspaceId\" = ?1 AND \"isDefault\" = 1",
        &[&workspace_id],
    )?
    .map(normalized))
}

/// `insertDefault`: inserts the default `full` token; `None` when another default row of
/// the workspace already exists (partial unique index on `workspaceId WHERE isDefault`).
pub fn insert_default(
    conn: &Connection,
    input: &InsertDefaultWorkspaceApiTokenInput,
) -> Result<Option<WorkspaceApiToken>> {
    let now = now_ms();
    let rows = super::returning::<WorkspaceApiToken>(
        conn,
        "INSERT INTO \"WorkspaceApiToken\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"tokenHash\", \
         \"name\", \"permission\", \"tokenPrefix\", \"isDefault\", \"encryptedToken\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'full', ?6, 1, ?7) \
         ON CONFLICT (\"workspaceId\") WHERE \"isDefault\" DO NOTHING",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.token_hash,
            &input.name,
            &input.token_prefix,
            &input.encrypted_token,
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `setEncryptedToken`: sets the blob only while it is still NULL.
pub fn set_encrypted_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    encrypted_token: &Json,
) -> Result<()> {
    conn.execute(
        "UPDATE \"WorkspaceApiToken\" SET \"encryptedToken\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"encryptedToken\" IS NULL",
        rusqlite::params![id, workspace_id, encrypted_token, now_ms()],
    )?;
    Ok(())
}
