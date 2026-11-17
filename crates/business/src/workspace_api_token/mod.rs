//! `packages/business/src/workspace-api-token`.

pub mod credentials;

use chatbotx_db::Db;
use chatbotx_db::repo::models::WorkspaceApiToken;
use chatbotx_db::repo::workspace_api_token as repo;

use crate::{BusinessError, Result, tx};

/// `MAX_WORKSPACE_API_TOKENS`.
pub const MAX_WORKSPACE_API_TOKENS: i64 = 10;

/// `listTokens`: newest first.
pub async fn list_tokens(db: &Db, workspace_id: i64) -> Result<Vec<WorkspaceApiToken>> {
    tx::read(db, move |conn| {
        Ok(repo::list_by_workspace_id(conn, workspace_id)?)
    })
    .await
}

/// A created token and its audit detail (never the token or its hash).
#[derive(Debug, Clone)]
pub struct CreatedToken {
    pub token: WorkspaceApiToken,
    pub audit: String,
}

/// `createToken`: count-then-insert in one write transaction (the cap is
/// `workspaceApiTokenLimitReached`, 400); the caller audits `create`.
pub async fn create_token(
    db: &Db,
    input: repo::InsertWorkspaceApiTokenInput,
) -> Result<CreatedToken> {
    tx::write(db, move |tx| {
        repo::lock_workspace_tokens(tx, input.workspace_id)?;
        if repo::count_by_workspace_id(tx, input.workspace_id)? >= MAX_WORKSPACE_API_TOKENS {
            return Err(BusinessError::exception(
                format!(
                    "Workspace has reached the maximum of {MAX_WORKSPACE_API_TOKENS} API tokens"
                ),
                "workspaceApiTokenLimitReached",
                400,
            ));
        }
        let token = repo::insert(tx, &input)?;
        let scopes = match &input.scopes {
            Some(list) if !list.is_empty() => list.join(","),
            _ => "all".to_string(),
        };
        Ok(CreatedToken {
            audit: format!(
                "created workspace API token \"{}\" ({}, scopes: {scopes})",
                input.name, input.permission
            ),
            token,
        })
    })
    .await
}

/// `deleteToken`: `true` when a row of the workspace was deleted (the caller
/// then audits `delete` "deleted workspace API token \"<id>\"").
pub async fn delete_token(db: &Db, workspace_id: i64, id: i64) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(repo::delete_by_id_for_workspace(tx, id, workspace_id)?)
    })
    .await
}
