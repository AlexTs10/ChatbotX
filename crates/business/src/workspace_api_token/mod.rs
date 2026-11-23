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

/// `defaultTokenAad`: binds the blob to its workspace.
fn default_token_aad(workspace_id: i64) -> String {
    format!("workspace-api-token:{workspace_id}")
}

/// `resolveDefaultTokenPlaintext`: the workspace's default token, minted on
/// first use; a legacy row without a blob recovers `Workspace.token` and
/// stores it encrypted. `None` = a legacy row with no workspace token.
pub async fn resolve_default_token_plaintext(
    db: &Db,
    encryption: &chatbotx_core::encryption::Encryption,
    workspace_id: i64,
) -> Result<Option<String>> {
    let aad = default_token_aad(workspace_id);
    let system = |e: String| BusinessError::exception(e, "systemError", 500);
    let encrypt = |text: &str| {
        encryption
            .encrypt_text(text, Some(&aad))
            .map_err(|e| system(e.to_string()))
            .and_then(|blob| serde_json::to_value(blob).map_err(|e| system(e.to_string())))
    };
    let fresh = credentials::generate_workspace_token();
    let input = repo::InsertDefaultWorkspaceApiTokenInput {
        workspace_id,
        token_hash: fresh.token_hash.clone(),
        name: "Default token".into(),
        token_prefix: fresh.token_prefix.clone(),
        encrypted_token: chatbotx_db::repo::Json(encrypt(&fresh.token)?),
    };
    // Find, else insert; a lost insert race re-selects the winner.
    let row = tx::write(db, move |tx| {
        if let Some(row) = repo::find_default_by_workspace_id(tx, workspace_id)? {
            return Ok(Some(row));
        }
        if repo::insert_default(tx, &input)?.is_some() {
            return Ok(None);
        }
        Ok(repo::find_default_by_workspace_id(tx, workspace_id)?)
    })
    .await?;
    let Some(row) = row else {
        return Ok(Some(fresh.token));
    };
    if let Some(blob) = row.encrypted_token {
        let blob = serde_json::from_value(blob.0).map_err(|e| system(e.to_string()))?;
        return encryption
            .decrypt_text(&blob, Some(&aad))
            .map(Some)
            .map_err(|e| system(e.to_string()));
    }
    let token = tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        Ok(conn
            .query_row(
                r#"SELECT "token" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?
            .flatten())
    })
    .await?
    .filter(|t| !t.is_empty());
    let Some(token) = token else {
        tracing::warn!(
            workspace_id,
            token_id = row.id,
            "Default workspace API token has no encryptedToken and Workspace.token is empty"
        );
        return Ok(None);
    };
    let blob = chatbotx_db::repo::Json(encrypt(&token)?);
    let id = row.id;
    tx::write(db, move |tx| {
        Ok(repo::set_encrypted_token(tx, id, workspace_id, &blob)?)
    })
    .await?;
    Ok(Some(token))
}
