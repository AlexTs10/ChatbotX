//! Q.D9 / Q.I4: the Google OAuth connect of the integration connections:
//! `connectGoogleCalendarAction`, `connectGoogleSheets`, the `googleCalendar`
//! and `googleSheets` cases of `app/integrations/[...integration]/callback.ts`
//! and the `generateAuthUrl` / `callbackHandler` of
//! `integrations/google-{calendar,sheets}`.
//!
//! The original state is unsigned base64 JSON (`workspaceId`, `referer`).
//! Here the connect action starts a connect session ([`connect_session`]):
//! the state is `{sessionId}.{nonce}` (32 random bytes, stored as a SHA-256
//! hash), the session holds the workspace and the acting user, and the
//! callback accepts it only for that signed-in user while they are still a
//! member. The return URL is an app-relative path (no open redirect). The
//! tokens are stored encrypted (Calendar: AAD
//! `integration-google-calendar:{id}`; Sheets: the `encryptObject` of
//! `IntegrationGoogleSheet.auth`). The hosts are Google's fixed hosts.

use std::sync::Arc;
use std::time::Duration;

use chatbotx_core::encryption::Encryption;
use chatbotx_db::Db;
use chatbotx_db::repo::{self, iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::appointment::external::{
    ExternalCalendars, create_google_from_oauth, form, google_message, read_json, sdk_error,
    urlencode,
};
use crate::connect_session::{self, NewSession, SessionRow};
use crate::errors::{BusinessError, Result};
use crate::tx;

const AUTHORIZED: &[&str] = &["authorized"];

/// The Google integration a session connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Calendar,
    Sheets,
}

impl Kind {
    /// The callback path slug (`/integrations/{slug}/callback`).
    pub fn from_slug(slug: &str) -> Option<Kind> {
        match slug {
            "google-calendar" => Some(Kind::Calendar),
            "google-sheets" => Some(Kind::Sheets),
            _ => None,
        }
    }

    /// The session `provider` (the integration type).
    pub fn from_provider(provider: &str) -> Option<Kind> {
        match provider {
            "googleCalendar" => Some(Kind::Calendar),
            "googleSheets" => Some(Kind::Sheets),
            _ => None,
        }
    }

    pub fn provider(self) -> &'static str {
        match self {
            Kind::Calendar => "googleCalendar",
            Kind::Sheets => "googleSheets",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Kind::Calendar => "google-calendar",
            Kind::Sheets => "google-sheets",
        }
    }

    /// `GOOGLE_CALENDAR_SCOPES` / `GOOGLE_SHEETS_SCOPES`.
    fn scopes(self) -> &'static str {
        match self {
            Kind::Calendar => {
                "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events"
            }
            Kind::Sheets => "https://www.googleapis.com/auth/spreadsheets openid email",
        }
    }

    fn not_configured(self) -> BusinessError {
        BusinessError::exception(
            match self {
                Kind::Calendar => "Google App settings is not valid",
                Kind::Sheets => "Google Sheets App settings is not valid",
            },
            "systemError",
            400,
        )
    }
}

/// `buildProviderCallbackUrl` (the app's own host).
pub fn callback_url(app_url: &str, kind: Kind) -> String {
    format!(
        "{}/integrations/{}/callback",
        app_url.trim_end_matches('/'),
        kind.slug()
    )
}

/// `generateAuthUrl` (`access_type=offline`, `prompt=consent`).
pub fn authorize_url(
    base: &str,
    client_id: &str,
    redirect_uri: &str,
    kind: Kind,
    state: &str,
) -> String {
    let mut url = url::Url::parse(base).unwrap_or_else(|_| {
        url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("static url")
    });
    url.query_pairs_mut()
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("scope", kind.scopes())
        .append_pair("state", state)
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri);
    url.into()
}

/// `sanitizeReferer`: the referer as an app-relative return path. An
/// absolute URL must have the app's origin; anything else is dropped.
pub fn return_path(app_url: &str, referer: Option<&str>) -> Option<String> {
    let referer = referer?.trim();
    let relative = match url::Url::parse(referer) {
        Ok(url) => {
            let app = url::Url::parse(app_url).ok()?;
            if url.origin() != app.origin() {
                return None;
            }
            let mut path = url.path().to_string();
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            path
        }
        Err(_) => referer.to_string(),
    };
    connect_session::validate_return_url(Some(&relative))
        .ok()
        .flatten()
}

/// The connect action: a `pending` session bound to `(workspace, user)`;
/// the Google consent URL to open.
pub async fn start(
    db: &Db,
    google: &ExternalCalendars,
    app_url: &str,
    kind: Kind,
    workspace_id: i64,
    user_id: i64,
    referer: Option<&str>,
) -> Result<String> {
    let (client_id, _) = google
        .client_credential(db, workspace_id)
        .await
        .map_err(|_| kind.not_configured())?;
    if client_id.is_empty() {
        return Err(kind.not_configured());
    }
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    let redirect = callback_url(app_url, kind);
    let base = google.endpoints.auth_url.clone();
    let session = connect_session::create(
        db,
        NewSession {
            workspace_id,
            provider: kind.provider().to_string(),
            purpose: "connect".to_string(),
            actor_user_id: Some(user_id),
            platform_owner_id: Some(owner.to_string()),
            return_url: return_path(app_url, referer),
            ..NewSession::default()
        },
        move |id, nonce| {
            json!({
                "type": "open_url",
                "url": authorize_url(&base, &client_id, &redirect, kind, &format!("{id}.{nonce}")),
            })
        },
    )
    .await?;
    session
        .next_action
        .as_ref()
        .and_then(|action| action["url"].as_str())
        .map(str::to_string)
        .ok_or_else(|| BusinessError::exception("Failed to connect Google", "systemError", 400))
}

/// A callback the session does not accept (another user, not a member, a
/// used session): `connectionStateMismatch`. The session is left as is, so
/// a forged callback cannot end the real user's session.
fn mismatch() -> BusinessError {
    crate::connection::oauth::state_mismatch()
}

fn is_member(conn: &Connection, workspace_id: i64, user_id: i64) -> Result<bool> {
    conn.query_row(
        r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
        params![workspace_id, user_id],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(|e| BusinessError::Repo(e.into()))
}

/// The callback: checks the session binding, exchanges `code`, stores the
/// integration and completes the session. The integration id.
pub async fn complete(
    db: &Db,
    google: &ExternalCalendars,
    app_url: &str,
    session: &SessionRow,
    user_id: Option<i64>,
    code: &str,
) -> Result<i64> {
    let kind = Kind::from_provider(&session.provider).ok_or_else(mismatch)?;
    let Some(user_id) = user_id.filter(|id| session.actor_user_id == Some(*id)) else {
        return Err(mismatch());
    };
    let workspace_id = session.workspace_id;
    if !tx::read(db, move |conn| is_member(conn, workspace_id, user_id)).await? {
        return Err(mismatch());
    }
    if session.status != "pending" {
        return Err(connect_session::expired(
            "This connect session is no longer active.",
        ));
    }
    connect_session::claim_authorization(db, session.id, workspace_id).await?;
    let outcome = exchange_and_store(db, google, app_url, kind, session, user_id, code).await;
    if outcome.is_err() {
        connect_session::fail(db, session, "exchange_failed", Some(AUTHORIZED)).await;
    }
    outcome
}

async fn exchange_and_store(
    db: &Db,
    google: &ExternalCalendars,
    app_url: &str,
    kind: Kind,
    session: &SessionRow,
    user_id: i64,
    code: &str,
) -> Result<i64> {
    if code.is_empty() {
        return Err(sdk_error("Code is required"));
    }
    let (client_id, client_secret) = google.client_credential(db, session.workspace_id).await?;
    let redirect = callback_url(app_url, kind);
    let response = google
        .http
        .post(&google.endpoints.token_url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form(&[
            ("code", code),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
        ]))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(sdk_error)?;
    let status = response.status();
    let tokens: Value = read_json(response).await.unwrap_or(Value::Null);
    let Some(access) = tokens["access_token"].as_str().filter(|t| !t.is_empty()) else {
        return Err(sdk_error(google_message(&tokens, status.as_u16())));
    };
    let refresh = tokens["refresh_token"].as_str();
    if kind == Kind::Sheets && refresh.is_none() {
        return Err(sdk_error("Google OAuth response has no refresh token"));
    }
    let mut auth = json!({
        "authType": "oauth2",
        "clientId": client_id,
        "clientSecret": client_secret,
        "redirectUrl": redirect,
        "tokens": {
            "accessToken": access,
            "expiresAt": tokens["expires_in"].as_i64().map(|s| iso(now_ms() + s * 1000)),
            "refreshToken": refresh,
        },
        "metadata": { "scope": tokens["scope"] },
    });
    let session = session.clone();
    match kind {
        Kind::Calendar => {
            let (calendar_id, email) = google.verify_calendar(access, "primary").await?;
            auth["metadata"]["providerCalendarId"] = json!(calendar_id);
            auth["metadata"]["email"] = json!(email);
            let encryption = Arc::clone(
                google
                    .encryption
                    .as_ref()
                    .ok_or_else(|| sdk_error("encryption is not configured"))?,
            );
            tx::write(db, move |conn| {
                store_calendar(
                    conn,
                    &encryption,
                    &session,
                    user_id,
                    &auth,
                    &calendar_id,
                    email,
                )
            })
            .await
        }
        Kind::Sheets => {
            let info = google
                .http
                .get(format!(
                    "{}?access_token={}",
                    google.endpoints.tokeninfo_url,
                    urlencode(access)
                ))
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(sdk_error)?;
            let info: Value = read_json(info).await.unwrap_or(Value::Null);
            let account = info["sub"]
                .as_str()
                .or_else(|| info["user_id"].as_str())
                .ok_or_else(|| sdk_error("Google Sheets token info has no stable account id"))?;
            auth["metadata"] = json!({
                "scope": tokens["scope"],
                "accountId": account,
                "email": info["email"],
            });
            let sealed = crate::email_marketing::seal_auth(&auth)?;
            tx::write(db, move |conn| {
                let integration_id = new_id();
                conn.execute(
                    r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'googleSheets')"#,
                    params![integration_id, session.workspace_id],
                )
                .map_err(repo::Error::from)?;
                conn.execute(
                    r#"INSERT INTO "IntegrationGoogleSheet" ("id", "workspaceId", "integrationId", "auth") VALUES (?1, ?2, ?3, ?4)"#,
                    params![new_id(), session.workspace_id, integration_id, sealed],
                )
                .map_err(repo::Error::from)?;
                let result = json!({ "targetId": integration_id.to_string(), "status": "connected" });
                connect_session::complete_authorized(conn, &session, result, None, now_ms())?;
                Ok(integration_id)
            })
            .await
        }
    }
}

/// `createGoogleFromOAuthCallback` + `attachIntegrationConnectionRow` +
/// the session completion, one transaction.
// ponytail: the original attaches the Connection row best-effort after the
// integration commit; here a failed attach rolls the connect back.
fn store_calendar(
    conn: &Connection,
    encryption: &Encryption,
    session: &SessionRow,
    user_id: i64,
    auth: &Value,
    calendar_id: &str,
    email: Option<String>,
) -> Result<i64> {
    let ws = session.workspace_id;
    let integration_id =
        create_google_from_oauth(conn, encryption, ws, auth, calendar_id, email.as_deref())?;
    let name = email.unwrap_or_else(|| "Google Calendar".to_string());
    let now = now_ms();
    let existing: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "Connection" WHERE "workspaceId" = ?1 AND "provider" = 'googleCalendar' AND "sourceId" = ?2"#,
            params![ws, calendar_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo::Error::from)?;
    let connection_id = match existing {
        Some(id) => {
            conn.execute(
                r#"UPDATE "Connection" SET "integrationId" = ?2, "displayName" = ?3, "lastError" = NULL,
                       "status" = 'connected', "statusReason" = NULL, "disconnectedAt" = NULL,
                       "connectedAt" = ?4, "updatedAt" = ?4
                   WHERE "id" = ?1"#,
                params![id, integration_id, name, now],
            )
            .map_err(repo::Error::from)?;
            id
        }
        None => {
            let id = new_id();
            conn.execute(
                r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                       "kind", "integrationId", "sourceId", "displayName", "status", "createdBy", "connectedAt")
                   VALUES (?1, ?2, ?2, ?3, 'googleCalendar', 'integration', ?4, ?5, ?6, 'connected', ?7, ?2)"#,
                params![id, now, ws, integration_id, calendar_id, name, user_id],
            )
            .map_err(repo::Error::from)?;
            id
        }
    };
    let result = json!({
        "targetId": calendar_id,
        "status": "connected",
        "connectionId": connection_id.to_string(),
    });
    connect_session::complete_authorized(conn, session, result, Some(connection_id), now)?;
    Ok(integration_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_path_keeps_only_app_relative_targets() {
        let app = "https://app.example.com";
        assert_eq!(
            return_path(app, Some("https://app.example.com/space/1/x?a=1")).as_deref(),
            Some("/space/1/x?a=1")
        );
        assert_eq!(return_path(app, Some("https://evil.example/x")), None);
        assert_eq!(return_path(app, Some("//evil.example/x")), None);
        assert_eq!(return_path(app, Some("/\\evil.example")), None);
        assert_eq!(return_path(app, Some("javascript:alert(1)")), None);
        assert_eq!(
            return_path(app, Some("/space/2")).as_deref(),
            Some("/space/2")
        );
        assert_eq!(return_path(app, None), None);
    }

    #[test]
    fn authorize_url_carries_the_session_state() {
        let url = authorize_url(
            "https://accounts.google.com/o/oauth2/v2/auth",
            "cid",
            "https://app/integrations/google-calendar/callback",
            Kind::Calendar,
            "12.abc",
        );
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs["state"], "12.abc");
        assert_eq!(pairs["access_type"], "offline");
        assert_eq!(pairs["prompt"], "consent");
        assert!(pairs["scope"].contains("calendar.events"));
        assert_eq!(Kind::from_slug("google-sheets"), Some(Kind::Sheets));
        assert_eq!(Kind::from_provider("googleCalendar"), Some(Kind::Calendar));
    }
}
