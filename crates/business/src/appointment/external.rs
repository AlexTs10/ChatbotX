//! `appointment-external-calendar/service.ts` and the Google Calendar calls
//! of `integrations/google-calendar` (free/busy, insert / delete event,
//! token refresh, revoke) on reqwest. The hosts are Google's fixed API hosts
//! (no caller URL), so no SSRF check applies.
//!
//! The stored `IntegrationGoogleCalendar.auth` is an encrypted blob
//! (`Encryption::encrypt_object`, AAD `integration-google-calendar:{integrationId}`);
//! a plain `Oauth2AuthValue` object (rows written by the original) is read as is.

use std::sync::Arc;
use std::time::Duration;

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::Db;
use chatbotx_db::repo::models::IntegrationGoogleCalendar;
use chatbotx_db::repo::{Error as RepoError, find_one, iso, new_id, now_ms, parse_iso};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::calendar::Interval;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// Google endpoints (tests point them at a fake server).
#[derive(Debug, Clone)]
pub struct GoogleEndpoints {
    pub api_base: String,
    pub token_url: String,
    pub revoke_url: String,
    /// The OAuth consent page (`generateAuthUrl`).
    pub auth_url: String,
    /// `OAuth2Client.getTokenInfo`.
    pub tokeninfo_url: String,
}

impl Default for GoogleEndpoints {
    fn default() -> GoogleEndpoints {
        GoogleEndpoints {
            api_base: "https://www.googleapis.com/calendar/v3".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            revoke_url: "https://oauth2.googleapis.com/revoke".into(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            tokeninfo_url: "https://oauth2.googleapis.com/tokeninfo".into(),
        }
    }
}

/// The external calendar provider calls (`integrationGoogleCalendar.runAction`).
/// Without `encryption` every provider call fails (no stored auth can be read).
#[derive(Clone, Default)]
pub struct ExternalCalendars {
    pub encryption: Option<Arc<Encryption>>,
    pub endpoints: GoogleEndpoints,
    pub http: reqwest::Client,
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Google Calendar connection not found")
}

/// A form body (reqwest is built without the `form` feature).
pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// A JSON body (reqwest is built without the `json` feature).
pub(crate) fn body_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

pub(crate) async fn read_json(response: reqwest::Response) -> std::result::Result<Value, String> {
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// `handleError`: an `SdkException` (default status 400), which the public
/// API answers as an undeclared `BAD_REQUEST` (`toKnownOrpcError`).
pub(crate) fn sdk_error(message: impl std::fmt::Display) -> BusinessError {
    BusinessError::exception(
        format!("Google Calendar API error: {message}"),
        "BAD_REQUEST",
        400,
    )
}

fn aad(integration_id: i64) -> String {
    format!("integration-google-calendar:{integration_id}")
}

/// `getGoogleConnectionForProviderCall`: the workspace's Google Calendar row.
pub fn google_connection(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
) -> Result<IntegrationGoogleCalendar> {
    find_one::<IntegrationGoogleCalendar>(
        conn,
        "\"workspaceId\" = ?1 AND \"integrationId\" = ?2 AND EXISTS (SELECT 1 FROM \"Integration\" i \
         WHERE i.\"id\" = \"integrationId\" AND i.\"integrationType\" = 'googleCalendar')",
        &[&workspace_id, &integration_id],
    )?
    .ok_or_else(not_found)
}

/// One `listWithConnectedCount` item (`connected_count` 0 for `listForSelect`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCalendarItem {
    pub id: i64,
    pub workspace_id: i64,
    pub label: String,
    pub provider_calendar_id: String,
    pub email: Option<String>,
    pub connected_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `listWithConnectedCount` (newest first); `listForSelect` is the same rows.
pub fn list_with_connected_count(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<ExternalCalendarItem>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT i.\"id\", i.\"workspaceId\", g.\"providerCalendarId\", g.\"email\", i.\"createdAt\", i.\"updatedAt\", \
             (SELECT count(*) FROM \"AppointmentCalendar\" c WHERE c.\"externalConnectionId\" = i.\"id\" AND c.\"deletedAt\" IS NULL) \
             FROM \"Integration\" i JOIN \"IntegrationGoogleCalendar\" g ON g.\"integrationId\" = i.\"id\" \
             WHERE i.\"workspaceId\" = ?1 AND i.\"integrationType\" = 'googleCalendar' ORDER BY i.\"createdAt\" DESC, i.\"id\" DESC",
        )
        .map_err(RepoError::from)?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            let provider_calendar_id: String = r.get(2)?;
            let email: Option<String> = r.get(3)?;
            Ok(ExternalCalendarItem {
                id: r.get(0)?,
                workspace_id: r.get(1)?,
                label: match &email {
                    Some(email) => format!("{email} ({provider_calendar_id})"),
                    None => provider_calendar_id.clone(),
                },
                provider_calendar_id,
                email,
                connected_count: r.get(6)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })
        .map_err(RepoError::from)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(RepoError::from)?;
    Ok(rows)
}

/// `createGoogleFromOAuthCallback`: updates the row of the same calendar or
/// inserts a new connection; the integration id. `auth` is the
/// `Oauth2AuthValue` object; it is stored encrypted.
pub fn create_google_from_oauth(
    conn: &Connection,
    encryption: &Encryption,
    workspace_id: i64,
    auth: &Value,
    provider_calendar_id: &str,
    email: Option<&str>,
) -> Result<i64> {
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT i.\"id\", g.\"id\" FROM \"IntegrationGoogleCalendar\" g JOIN \"Integration\" i ON i.\"id\" = g.\"integrationId\" \
             WHERE g.\"workspaceId\" = ?1 AND g.\"providerCalendarId\" = ?2 AND i.\"integrationType\" = 'googleCalendar' LIMIT 1",
            params![workspace_id, provider_calendar_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(RepoError::from)?;
    let integration_id = existing.map_or_else(new_id, |(id, _)| id);
    let sealed = seal(encryption, integration_id, auth)?;
    if let Some((integration_id, row_id)) = existing {
        conn.execute(
            "UPDATE \"IntegrationGoogleCalendar\" SET \"auth\" = ?2, \"providerCalendarId\" = ?3, \"email\" = ?4, \"updatedAt\" = ?5 WHERE \"id\" = ?1",
            params![row_id, sealed, provider_calendar_id, email, now_ms()],
        )
        .map_err(RepoError::from)?;
        return Ok(integration_id);
    }
    conn.execute(
        "INSERT INTO \"Integration\" (\"id\", \"workspaceId\", \"integrationType\") VALUES (?1, ?2, 'googleCalendar')",
        params![integration_id, workspace_id],
    )
    .map_err(RepoError::from)?;
    conn.execute(
        "INSERT INTO \"IntegrationGoogleCalendar\" (\"id\", \"workspaceId\", \"integrationId\", \"auth\", \"providerCalendarId\", \"email\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![new_id(), workspace_id, integration_id, sealed, provider_calendar_id, email],
    )
    .map_err(RepoError::from)?;
    Ok(integration_id)
}

pub(crate) fn seal(encryption: &Encryption, integration_id: i64, auth: &Value) -> Result<String> {
    let blob = encryption
        .encrypt_object(auth, Some(&aad(integration_id)))
        .map_err(|e| BusinessError::Repo(RepoError::msg(e.to_string())))?;
    serde_json::to_string(&blob).map_err(|e| BusinessError::Repo(RepoError::msg(e.to_string())))
}

/// The stored auth as the `Oauth2AuthValue` object.
pub fn open_auth(encryption: &Encryption, row: &IntegrationGoogleCalendar) -> Result<Value> {
    if row.auth.get("tokens").is_some() {
        return Ok(row.auth.0.clone());
    }
    let blob: EncryptedData = serde_json::from_value(row.auth.0.clone())
        .map_err(|e| BusinessError::Repo(RepoError::msg(e.to_string())))?;
    encryption
        .decrypt_object(&blob, Some(&aad(row.integration_id)))
        .map_err(|e| BusinessError::Repo(RepoError::msg(e.to_string())))
}

/// `getDisconnectableGoogleConnection` + `deleteByIntegrationIds`: refuses a
/// connection a live calendar uses (`connectionInUse` 409). Returns the
/// removed row's auth for the token revoke.
pub fn delete_google_connection(
    conn: &Connection,
    workspace_id: i64,
    integration_id: i64,
) -> Result<IntegrationGoogleCalendar> {
    let item = list_with_connected_count(conn, workspace_id)?
        .into_iter()
        .find(|item| item.id == integration_id)
        .ok_or_else(not_found)?;
    if item.connected_count > 0 {
        return Err(BusinessError::exception(
            "Connection is in use",
            "connectionInUse",
            409,
        ));
    }
    let row = google_connection(conn, workspace_id, integration_id)?;
    conn.execute(
        "DELETE FROM \"Integration\" WHERE \"workspaceId\" = ?1 AND \"id\" = ?2 \
         AND \"integrationType\" IN ('googleCalendar', 'outlookCalendar')",
        params![workspace_id, integration_id],
    )
    .map_err(RepoError::from)?;
    Ok(row)
}

/// `disconnect`: delete, then revoke the token (a revoke failure is logged).
pub async fn disconnect(
    db: &Db,
    external: &ExternalCalendars,
    workspace_id: i64,
    integration_id: i64,
) -> Result<()> {
    let row = tx::write(db, move |conn| {
        delete_google_connection(conn, workspace_id, integration_id)
    })
    .await?;
    if let Err(error) = external.revoke(&row).await {
        tracing::warn!(
            workspace_id,
            integration_id,
            err = %error.message(),
            "Failed to revoke Google Calendar token"
        );
    }
    Ok(())
}

/// The `{ start, end }` of a busy entry, or `None` for a bad / empty one.
fn busy_interval(entry: &Value) -> Option<Interval> {
    let start = parse_iso(entry.get("start")?.as_str()?)?;
    let end = parse_iso(entry.get("end")?.as_str()?)?;
    (start < end).then_some(Interval { start, end })
}

impl ExternalCalendars {
    fn encryption(&self) -> Result<&Encryption> {
        self.encryption
            .as_deref()
            .ok_or_else(|| sdk_error("encryption is not configured"))
    }

    /// The access token of `row`, refreshed (and stored) when it expired.
    async fn access_token(&self, db: &Db, row: &IntegrationGoogleCalendar) -> Result<String> {
        let auth = open_auth(self.encryption()?, row)?;
        let tokens = auth.get("tokens").cloned().unwrap_or(Value::Null);
        let token = tokens
            .get("accessToken")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let expires_at = tokens
            .get("expiresAt")
            .and_then(Value::as_str)
            .and_then(parse_iso);
        // `buildContext` refreshes a token that expires within a minute.
        if expires_at.is_none_or(|at| at > now_ms() + 60_000) {
            return Ok(token);
        }
        let refresh = tokens
            .get("refreshToken")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BusinessError::exception(
                    "Google Calendar refresh token not available",
                    "authException",
                    401,
                )
            })?
            .to_string();
        let (client_id, client_secret) = self.client_credential(db, row.workspace_id).await?;
        let response = self
            .http
            .post(&self.endpoints.token_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("refresh_token", refresh.as_str()),
                ("grant_type", "refresh_token"),
            ]))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(sdk_error)?;
        let status = response.status();
        let body: Value = read_json(response).await.unwrap_or(Value::Null);
        let Some(access) = body.get("access_token").and_then(Value::as_str) else {
            return Err(BusinessError::exception(
                if status.as_u16() == 400 {
                    "Google Calendar refresh token was revoked"
                } else {
                    "Google Calendar refresh returned no token"
                },
                "authException",
                401,
            ));
        };
        let mut next = auth.clone();
        next["tokens"]["accessToken"] = json!(access);
        if let Some(rotated) = body.get("refresh_token").and_then(Value::as_str) {
            next["tokens"]["refreshToken"] = json!(rotated);
        }
        if let Some(seconds) = body.get("expires_in").and_then(Value::as_i64) {
            next["tokens"]["expiresAt"] = json!(iso(now_ms() + seconds * 1000));
        }
        let sealed = seal(self.encryption()?, row.integration_id, &next)?;
        let id = row.id;
        tx::write(db, move |conn| {
            conn.execute(
                "UPDATE \"IntegrationGoogleCalendar\" SET \"auth\" = ?2, \"updatedAt\" = ?3 WHERE \"id\" = ?1",
                params![id, sealed, now_ms()],
            )
            .map_err(RepoError::from)?;
            Ok(())
        })
        .await?;
        Ok(access.to_string())
    }

    /// The Google OAuth app (`clientId`, `clientSecret`) for the workspace owner.
    pub(crate) async fn client_credential(
        &self,
        db: &Db,
        workspace_id: i64,
    ) -> Result<(String, String)> {
        let owner =
            crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None)
                .await?;
        let encryption = Arc::clone(
            self.encryption
                .as_ref()
                .ok_or_else(|| sdk_error("no key"))?,
        );
        tx::read(db, move |conn| {
            let credential = crate::platform_credential::resolve_for_owner(
                conn,
                &encryption,
                owner,
                "google",
                false,
            )?
            .ok_or_else(|| sdk_error("Google credentials are not configured"))?;
            Ok((
                credential.text("clientId").unwrap_or_default().to_string(),
                credential
                    .text("clientSecret")
                    .unwrap_or_default()
                    .to_string(),
            ))
        })
        .await
    }

    async fn connection(
        &self,
        db: &Db,
        workspace_id: i64,
        integration_id: i64,
    ) -> Result<(IntegrationGoogleCalendar, String)> {
        let row = tx::read(db, move |conn| {
            google_connection(conn, workspace_id, integration_id)
        })
        .await?;
        let token = self.access_token(db, &row).await?;
        Ok((row, token))
    }

    /// `getBusyIntervalsForAppointmentCalendar` (`freebusy.query`).
    #[allow(clippy::too_many_arguments)]
    pub async fn busy_intervals(
        &self,
        db: &Db,
        workspace_id: i64,
        integration_id: i64,
        time_min: i64,
        time_max: i64,
        time_zone: Option<&str>,
        timeout_ms: u64,
    ) -> Result<Vec<Interval>> {
        let (row, token) = self.connection(db, workspace_id, integration_id).await?;
        let mut body = json!({
            "timeMin": iso(time_min),
            "timeMax": iso(time_max),
            "items": [{ "id": row.provider_calendar_id }],
        });
        if let Some(zone) = time_zone {
            body["timeZone"] = json!(zone);
        }
        let response = self
            .http
            .post(format!("{}/freeBusy", self.endpoints.api_base))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_bytes(&body))
            .timeout(Duration::from_millis(timeout_ms))
            .send()
            .await
            .map_err(sdk_error)?;
        let status = response.status();
        let data: Value = read_json(response).await.map_err(sdk_error)?;
        if !status.is_success() {
            return Err(sdk_error(google_message(&data, status.as_u16())));
        }
        let busy = data["calendars"][row.provider_calendar_id.as_str()]["busy"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(busy.iter().filter_map(busy_interval).collect())
    }

    /// `createEvent` (`events.insert`, `sendUpdates=all`); a 409 reads the
    /// event that already has the id. The event id.
    pub async fn create_event(
        &self,
        db: &Db,
        workspace_id: i64,
        integration_id: i64,
        calendar_id: &str,
        event: &Value,
    ) -> Result<String> {
        let (_, token) = self.connection(db, workspace_id, integration_id).await?;
        let events = format!(
            "{}/calendars/{}/events",
            self.endpoints.api_base,
            urlencode(calendar_id)
        );
        let response = self
            .http
            .post(format!("{events}?sendUpdates=all"))
            .bearer_auth(&token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_bytes(event))
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(sdk_error)?;
        let mut status = response.status().as_u16();
        let mut data: Value = read_json(response).await.unwrap_or(Value::Null);
        if status == 409 {
            let id = event["id"].as_str().unwrap_or_default();
            let response = self
                .http
                .get(format!("{events}/{}", urlencode(id)))
                .bearer_auth(&token)
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .map_err(sdk_error)?;
            status = response.status().as_u16();
            data = read_json(response).await.unwrap_or(Value::Null);
        }
        match data.get("id").and_then(Value::as_str) {
            Some(id) if (200..300).contains(&status) && !id.is_empty() => Ok(id.to_string()),
            _ => Err(sdk_error(google_message(&data, status))),
        }
    }

    /// `cancelEvent` (`events.delete`); 404 / 410 = already gone.
    pub async fn cancel_event(
        &self,
        db: &Db,
        workspace_id: i64,
        integration_id: i64,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<()> {
        let (_, token) = self.connection(db, workspace_id, integration_id).await?;
        let response = self
            .http
            .delete(format!(
                "{}/calendars/{}/events/{}?sendUpdates=all",
                self.endpoints.api_base,
                urlencode(calendar_id),
                urlencode(event_id)
            ))
            .bearer_auth(token)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(sdk_error)?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) || status == 404 || status == 410 {
            return Ok(());
        }
        let data: Value = read_json(response).await.unwrap_or(Value::Null);
        Err(sdk_error(google_message(&data, status)))
    }

    /// `verifyCalendarAccess` (`calendarList.get`): the provider calendar id
    /// and its email (the id when it is an address, else the summary).
    pub async fn verify_calendar(
        &self,
        token: &str,
        calendar_id: &str,
    ) -> Result<(String, Option<String>)> {
        let response = self
            .http
            .get(format!(
                "{}/users/me/calendarList/{}",
                self.endpoints.api_base,
                urlencode(calendar_id)
            ))
            .bearer_auth(token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(sdk_error)?;
        let status = response.status();
        let data: Value = read_json(response).await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(sdk_error(google_message(&data, status.as_u16())));
        }
        let id = data["id"].as_str().unwrap_or(calendar_id).to_string();
        let email = if id.contains('@') {
            Some(id.clone())
        } else {
            data["summary"].as_str().map(str::to_string)
        };
        Ok((id, email))
    }

    /// `updateGoogleCalendarIdAction`: verify the calendar with the
    /// connection's token, then store its id and email.
    pub async fn update_calendar_id(
        &self,
        db: &Db,
        workspace_id: i64,
        integration_id: i64,
        calendar_id: &str,
    ) -> Result<()> {
        let (_, token) = self.connection(db, workspace_id, integration_id).await?;
        let (id, email) = self.verify_calendar(&token, calendar_id).await?;
        tx::write(db, move |conn| {
            google_connection(conn, workspace_id, integration_id)?;
            conn.execute(
                "UPDATE \"IntegrationGoogleCalendar\" SET \"providerCalendarId\" = ?3, \"email\" = ?4, \"updatedAt\" = ?5 \
                 WHERE \"workspaceId\" = ?1 AND \"integrationId\" = ?2",
                params![workspace_id, integration_id, id, email, now_ms()],
            )
            .map_err(RepoError::from)?;
            Ok(())
        })
        .await
    }

    /// `disconnect` (`revokeToken(accessToken)`: `POST /revoke?token=…`).
    pub async fn revoke(&self, row: &IntegrationGoogleCalendar) -> Result<()> {
        let auth = open_auth(self.encryption()?, row)?;
        let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
        self.http
            .post(revoke_url(&self.endpoints.revoke_url, token))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(sdk_error)?
            .error_for_status()
            .map_err(sdk_error)?;
        Ok(())
    }
}

/// The joined `error.errors[].message` of a Google error body.
pub(crate) fn google_message(data: &Value, status: u16) -> String {
    let messages: Vec<&str> = data["error"]["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["message"].as_str())
        .filter(|m| !m.is_empty())
        .collect();
    if messages.is_empty() {
        data["error"]["message"].as_str().map_or_else(
            || format!("Request failed with status code {status}"),
            str::to_string,
        )
    } else {
        messages.join(", ")
    }
}

/// google-auth-library `getRevokeTokenURL`: the token is a query parameter.
fn revoke_url(base: &str, token: &str) -> String {
    format!("{base}?{}", form(&[("token", token)]))
}

pub(crate) fn urlencode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// `createDeterministicGoogleEventId`: `abe` + sha256(appointment id) as hex.
pub fn deterministic_event_id(appointment_id: i64) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(appointment_id.to_string().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("abe{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_entries_drop_bad_and_empty_ranges() {
        let ok = json!({ "start": "2026-11-17T10:00:00Z", "end": "2026-11-17T11:00:00Z" });
        assert!(busy_interval(&ok).is_some());
        let empty = json!({ "start": "2026-11-17T10:00:00Z", "end": "2026-11-17T10:00:00Z" });
        assert!(busy_interval(&empty).is_none());
        assert!(busy_interval(&json!({ "start": "x", "end": "y" })).is_none());
    }

    #[test]
    fn revoke_sends_the_token_in_the_query() {
        assert_eq!(
            revoke_url("https://oauth2.googleapis.com/revoke", "ya29.a/b+c"),
            "https://oauth2.googleapis.com/revoke?token=ya29.a%2Fb%2Bc"
        );
    }

    #[test]
    fn google_message_joins_errors() {
        let data = json!({ "error": { "errors": [{ "message": "a" }, { "message": "b" }] } });
        assert_eq!(google_message(&data, 400), "a, b");
        assert_eq!(
            google_message(&Value::Null, 503),
            "Request failed with status code 503"
        );
    }
}
