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
