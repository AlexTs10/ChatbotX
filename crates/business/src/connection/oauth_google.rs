//! The Google Calendar and Google Sheets `connection` adapters
//! (`integrations/google-{calendar,sheets}/src/integration.ts`,
//! `googleOAuthConnection` of `packages/sdk/src/lib/connection-helpers.ts`):
//! single-account, the grant is its one candidate. The stores are the
//! workspace integration bindings `IntegrationGoogleCalendar` (auth sealed
//! per integration, as the builder connect) and `IntegrationGoogleSheet`.

use chatbotx_core::encryption::Encryption;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use super::credentials::{Upstream, UpstreamRequest};
use super::oauth::{ProviderFailure, oauth2_auth, text};
use super::{ConnectionRow, find_or_fail, repo_err};
use crate::google_ads_api as api;
use crate::google_oauth::{self, Kind};
use crate::{BusinessError, Result};

/// The Calendar API (`calendarList.get`).
const CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3";

/// `googleCalendar` / `googleSheets`.
pub fn handles(provider: &str) -> bool {
    Kind::from_provider(provider).is_some()
}

/// `authorizeUrl` (`generateAuthUrl`: offline, consent, the scopes, the
/// state verbatim).
pub fn authorize_url(provider: &str, config: &Value, callback_url: &str, state: &str) -> String {
    let kind = Kind::from_provider(provider).unwrap_or(Kind::Calendar);
    google_oauth::authorize_url(
        api::AUTH_URL,
        text(config, "clientId"),
        callback_url,
        kind,
        state,
    )
}

fn rejected(message: impl Into<String>) -> ProviderFailure {
    ProviderFailure::Rejected(message.into())
}

/// A Google API answer: 2xx JSON, a 4xx other than 408 / 429 is a
/// rejection, the rest is retryable.
async fn google_get(
    up: &impl Upstream,
    url: String,
    bearer: Option<&str>,
) -> std::result::Result<Value, ProviderFailure> {
    let headers = bearer
        .map(|token| vec![("authorization".to_string(), format!("Bearer {token}"))])
        .unwrap_or_default();
    match up
        .send(UpstreamRequest {
            method: "GET",
            url,
            headers,
            body: None,
        })
        .await
    {
        Ok((status, body)) if (200..300).contains(&status) => {
            Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
        }
        Ok((status, body)) if (400..500).contains(&status) && status != 408 && status != 429 => {
            let answer: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            Err(rejected(
                answer["error"]["message"]
                    .as_str()
                    .or_else(|| answer["error_description"].as_str())
                    .unwrap_or("The provider rejected the request."),
            ))
        }
        Ok((408 | 429, _)) | Err(_) => Err(ProviderFailure::Unavailable(503)),
        Ok(_) => Err(ProviderFailure::Unavailable(502)),
    }
}

/// `exchangeCode`: `getToken` + `googleTokensToAuth` (an access and a
/// refresh token are required), then `mapAuth` (Calendar: the primary
/// calendar must answer; Sheets: the stable account id of `getTokenInfo`).
pub async fn exchange_code(
    up: &impl Upstream,
    provider: &str,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    let kind = Kind::from_provider(provider).ok_or_else(|| rejected("Unknown provider"))?;
    let tokens = api::token_request(
        up,
        &[
            ("code", code),
            ("client_id", text(config, "clientId")),
            ("client_secret", text(config, "clientSecret")),
            ("redirect_uri", callback_url),
            ("grant_type", "authorization_code"),
        ],
    )
    .await
    .map_err(|(revoked, message)| {
        if revoked {
            rejected(message)
        } else {
            ProviderFailure::Unavailable(503)
        }
    })?;
    let access = text(&tokens, "access_token");
    if access.is_empty() {
        return Err(rejected("Google OAuth response has no access token"));
    }
    let Some(refresh) = tokens["refresh_token"].as_str().filter(|t| !t.is_empty()) else {
        return Err(rejected("Google OAuth response has no refresh token"));
    };
    let mut token_value = json!({ "accessToken": access, "refreshToken": refresh });
    if let Some(seconds) = tokens["expires_in"].as_i64() {
        token_value["expiresAt"] = json!(iso(now_ms() + seconds * 1000));
    }
    let mut metadata = json!({});
    if let Some(scope) = tokens["scope"].as_str() {
        metadata["scope"] = json!(scope);
    }
    match kind {
        Kind::Calendar => {
            // `verifyCalendarAccess(auth, "primary")`.
            let calendar = google_get(
                up,
                format!("{CALENDAR_API}/users/me/calendarList/primary"),
                Some(access),
            )
            .await?;
            let id = calendar["id"].as_str().unwrap_or("primary").to_string();
            let email = if id.contains('@') {
                Some(id.clone())
            } else {
                calendar["summary"].as_str().map(str::to_string)
            };
            metadata["providerCalendarId"] = json!(id);
            if let Some(email) = email {
                metadata["email"] = json!(email);
            }
        }
        Kind::Sheets => {
            let info = google_get(
                up,
                format!(
                    "{}?{}",
                    api::TOKENINFO_URL,
                    url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("access_token", access)
                        .finish()
                ),
                None,
            )
            .await?;
            let account = info["sub"]
                .as_str()
                .or_else(|| info["user_id"].as_str())
                .ok_or_else(|| rejected("Google Sheets token info has no stable account id"))?;
            metadata["accountId"] = json!(account);
            if let Some(email) = info["email"].as_str() {
                metadata["email"] = json!(email);
            }
        }
    }
    Ok(oauth2_auth(
        config,
        callback_url,
        token_value,
        Some(metadata),
    ))
}

/// `describe`: Calendar = the provider calendar id; Sheets = the Google
/// account id (`sourceId`), the email as the display name.
pub fn describe(provider: &str, auth: &Value) -> Result<(String, String)> {
    let metadata = &auth["metadata"];
    let (source, fallback, missing) = match Kind::from_provider(provider) {
        Some(Kind::Calendar) => (
            text(metadata, "providerCalendarId"),
            "Google Calendar",
            "Google Calendar auth has no calendar identity",
        ),
        _ => (
            text(metadata, "accountId"),
            "Google Sheets",
            "Google Sheets auth has no account identity",
        ),
    };
    if source.is_empty() {
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(missing)));
    }
    let name = metadata["email"].as_str().unwrap_or(fallback);
    Ok((source.to_string(), name.to_string()))
}

/// `connectAndPersist` for the grant: an active connection of the same
/// account refuses (`connectionAlreadyConnected`); an inactive one is
/// revived with the new auth.
pub fn persist(
    conn: &Connection,
    encryption: &Encryption,
    provider: &str,
    workspace_id: i64,
    auth: &Value,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let kind =
        Kind::from_provider(provider).ok_or_else(|| super::oauth::not_configured(provider))?;
    let (source, name) = describe(provider, auth)?;
    let existing: Option<(i64, String, Option<i64>)> = conn
        .query_row(
            r#"SELECT "id", "status", "integrationId" FROM "Connection"
               WHERE "workspaceId" = ?1 AND "provider" = ?2 AND "sourceId" = ?3"#,
            params![workspace_id, provider, source],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(repo_err)?;
    if existing
        .as_ref()
        .is_some_and(|(_, s, _)| super::is_active(s))
    {
        return Err(super::credentials::already_connected());
    }
    let integration_id = match kind {
        Kind::Calendar => crate::appointment::external::create_google_from_oauth(
            conn,
            encryption,
            workspace_id,
            auth,
            &source,
            auth["metadata"]["email"].as_str(),
        )?,
        Kind::Sheets => {
            let sealed = crate::email_marketing::seal_auth(auth)?;
            let revived = match existing.as_ref().and_then(|(_, _, id)| *id) {
                Some(integration_id) => conn
                    .execute(
                        r#"UPDATE "IntegrationGoogleSheet" SET "auth" = ?3, "updatedAt" = ?4
                           WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                        params![integration_id, workspace_id, sealed, now],
                    )
                    .map_err(repo_err)?
                    .gt(&0)
                    .then_some(integration_id),
                None => None,
            };
            match revived {
                Some(id) => id,
                None => {
                    let integration_id = new_id();
                    conn.execute(
                        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'googleSheets')"#,
                        params![integration_id, workspace_id],
                    )
                    .map_err(repo_err)?;
                    conn.execute(
                        r#"INSERT INTO "IntegrationGoogleSheet" ("id", "workspaceId", "integrationId", "auth") VALUES (?1, ?2, ?3, ?4)"#,
                        params![new_id(), workspace_id, integration_id, sealed],
                    )
                    .map_err(repo_err)?;
                    integration_id
                }
            }
        }
    };
    let id = match existing {
        Some((id, _, _)) => {
            conn.execute(
                r#"UPDATE "Connection" SET "integrationId" = ?3, "displayName" = ?4, "lastError" = NULL,
                       "status" = 'connected', "statusReason" = NULL, "disconnectedAt" = NULL,
                       "connectedAt" = ?5, "updatedAt" = ?5
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id, integration_id, name, now],
            )
            .map_err(repo_err)?;
            id
        }
        None => {
            let id = new_id();
            conn.execute(
                r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                       "kind", "integrationId", "sourceId", "displayName", "status", "createdBy", "connectedAt")
                   VALUES (?1, ?2, ?2, ?3, ?4, 'integration', ?5, ?6, ?7, 'connected', ?8, ?2)"#,
                params![id, now, workspace_id, provider, integration_id, source, name, actor_user_id],
            )
            .map_err(repo_err)?;
            id
        }
    };
    find_or_fail(conn, id, workspace_id)
}

/// `saveAuthByForeignKey` of a reconnect: the integration's satellite row
/// takes the re-granted auth. `false` = no row.
pub fn save_reconnect_auth(
    conn: &Connection,
    encryption: &Encryption,
    provider: &str,
    integration_id: Option<i64>,
    workspace_id: i64,
    auth: &Value,
    now: i64,
) -> Result<bool> {
    let Some(integration_id) = integration_id else {
        return Ok(false);
    };
    let updated = match Kind::from_provider(provider) {
        Some(Kind::Calendar) => {
            let sealed = crate::appointment::external::seal(encryption, integration_id, auth)?;
            conn.execute(
                r#"UPDATE "IntegrationGoogleCalendar" SET "auth" = ?3, "updatedAt" = ?4
                   WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                params![integration_id, workspace_id, sealed, now],
            )
        }
        _ => {
            let sealed = crate::email_marketing::seal_auth(auth)?;
            conn.execute(
                r#"UPDATE "IntegrationGoogleSheet" SET "auth" = ?3, "updatedAt" = ?4
                   WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                params![integration_id, workspace_id, sealed, now],
            )
        }
    }
    .map_err(repo_err)?;
    Ok(updated > 0)
}
