//! `integration-api/service.ts` (`createWithToken`, `connect`) and the
//! `api` connect adapter of `packages/connections/src/registry.ts`.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::json;

use crate::connection::{self, ConnectionRow};
use crate::net::ssrf_guard::{DohFetch, assert_public_url_with};
use crate::workspace_api_token::credentials::{
    generate_api_channel_token, generate_signing_secret,
};
use crate::{BusinessError, Result, tx};

/// `CreateApiWithTokenInput` (an existing workspace).
#[derive(Debug, Clone)]
pub struct CreateApiChannel {
    pub workspace_id: i64,
    /// `actorUserId` (`Connection.createdBy`); a token actor has none.
    pub actor_user_id: Option<i64>,
    pub name: String,
    pub callback_url: Option<String>,
}

/// `integrationApiService.createWithToken`: the SSRF check of `callbackUrl`
/// (`422 invalidRequestData`), a new token, the inbox, the `IntegrationApi`
/// row and the `connected` connection in one transaction. Returns the
/// connection and the one-time token.
pub async fn create_with_token(
    db: &Db,
    input: CreateApiChannel,
    doh: &impl DohFetch,
    now: i64,
) -> Result<(ConnectionRow, String)> {
    if let Some(callback_url) = &input.callback_url {
        assert_public_url_with(callback_url, "API channel callback URL", doh)
            .await
            .map_err(|message| BusinessError::exception(message, "invalidRequestData", 422))?;
    }
    let credentials = generate_api_channel_token();
    let auth = json!({
        "authType": "custom",
        "callbackUrl": input.callback_url,
        "signingSecret": generate_signing_secret(),
    })
    .to_string();
    let token = credentials.token.clone();
    let row = tx::write(db, move |tx| {
        let api_id: i64 = chatbotx_core::utils::id::create_id()
            .parse()
            .unwrap_or_default();
        connection::insert_inbox(tx, api_id, input.workspace_id, "api", &input.name, now)?;
        // `CONNECTION_STORE_BINDINGS.api`: the satellite row keyed by the
        // source id (= the inbox id).
        tx.execute(
            r#"INSERT INTO "IntegrationApi" ("id", "createdAt", "updatedAt", "auth", "name",
                   "tokenHash", "tokenPrefix", "callbackUrl", "enabled", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?1)"#,
            params![
                api_id,
                now,
                auth,
                input.name,
                credentials.token_hash,
                credentials.token_prefix,
                input.callback_url,
                input.workspace_id,
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        connection::insert_connected_channel(
            tx,
            input.workspace_id,
            "api",
            api_id,
            &input.name,
            input.actor_user_id,
            now,
        )
    })
    .await?;
    Ok((row, token))
}

/// `findIntegrationApiByWorkspaceAndId`: 404 `Integration API not found`.
fn find_auth(
    tx: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    id: i64,
) -> Result<(String, i64)> {
    use chatbotx_db::rusqlite::OptionalExtension as _;
    tx.query_row(
        r#"SELECT "auth", "inboxId" FROM "IntegrationApi" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![id, workspace_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| BusinessError::Repo(e.into()))?
    .ok_or_else(|| BusinessError::not_found("Integration API not found"))
}

/// `updateApiAction`: the SSRF check of a new `callbackUrl`, then `name` /
/// `callbackUrl` (each only when given) and the auth's `callbackUrl`.
/// `callback_url: Some(None)` clears it.
pub async fn update_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    callback_url: Option<Option<String>>,
    doh: &impl DohFetch,
) -> Result<()> {
    if let Some(Some(url)) = &callback_url {
        // The guard throws a plain `Error`: the action answers the default
        // `serverError` (`handleServerError`).
        assert_public_url_with(url, "API channel callback URL", doh)
            .await
            .map_err(|_| {
                BusinessError::exception(
                    "Something went wrong while executing the operation.",
                    "internalServerError",
                    500,
                )
            })?;
    }
    tx::write(db, move |tx| {
        let (auth, _) = find_auth(tx, workspace_id, id)?;
        let mut auth: serde_json::Value = serde_json::from_str(&auth).unwrap_or(json!({}));
        if let (Some(url), Some(object)) = (&callback_url, auth.as_object_mut()) {
            object.insert("callbackUrl".into(), json!(url));
        }
        tx.execute(
            r#"UPDATE "IntegrationApi" SET
                   "name" = COALESCE(?3, "name"),
                   "callbackUrl" = CASE WHEN ?4 THEN ?5 ELSE "callbackUrl" END,
                   "auth" = ?6, "updatedAt" = ?7
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                id,
                workspace_id,
                name,
                callback_url.is_some(),
                callback_url.flatten(),
                auth.to_string(),
                chatbotx_db::repo::now_ms(),
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// `rotateApiTokenAction`: a new token (hash + prefix stored); returns it.
pub async fn rotate_token(db: &Db, workspace_id: i64, id: i64) -> Result<String> {
    let credentials = generate_api_channel_token();
    let token = credentials.token.clone();
    tx::write(db, move |tx| {
        find_auth(tx, workspace_id, id)?;
        tx.execute(
            r#"UPDATE "IntegrationApi" SET "tokenHash" = ?3, "tokenPrefix" = ?4, "updatedAt" = ?5
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                id,
                workspace_id,
                credentials.token_hash,
                credentials.token_prefix,
                chatbotx_db::repo::now_ms(),
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await?;
    Ok(token)
}

/// `deleteApiAction` → `integrationApiService.disconnect`: the row goes, the
/// inbox's connection is marked disconnected (`disconnectInbox`). The API
/// channel has no provider to call (`integrationApi.disconnect` is a no-op).
pub async fn disconnect(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    let now = chatbotx_db::repo::now_ms();
    tx::write(db, move |tx| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        let (_, inbox_id) = find_auth(tx, workspace_id, id)?;
        tx.execute(
            r#"DELETE FROM "IntegrationApi" WHERE "id" = ?1"#,
            params![id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        let current = tx
            .query_row(
                &format!(
                    r#"SELECT {} FROM "Connection" WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
                    connection::COLUMNS
                ),
                params![inbox_id, workspace_id],
                connection::row,
            )
            .optional()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        if let Some(current) = current {
            connection::mark_disconnected(tx, &current, now)?;
        }
        Ok(())
    })
    .await
}
