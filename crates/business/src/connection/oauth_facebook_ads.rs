//! Q.F2-settings: the workspace Facebook Ads integration
//! (`features/integration-facebook-ads`): `connectFacebookAds`
//! (`buildFacebookAdsAuthRedirect`), the `flow: "facebookAds"` branch of
//! the Messenger OAuth callback (`storeFacebookAdsConnection` +
//! `integrationFacebookAdsService.upsert`), `disconnectFacebookAdsAction`
//! (`revokeToken` + `disconnect`) and the settings page row.
//!
//! The original state is unsigned base64 JSON (`workspaceId`, `referer`,
//! `flow`). Here the connect action starts a connect session
//! ([`connect_session`], provider `facebookAds`): the state is
//! `{sessionId}.{nonce}`, and the callback accepts it only for the signed-in
//! user who started it while they are still a member (as
//! [`crate::google_oauth`]). The return URL is the fixed settings path. The
//! Messenger app credential is used and the Messenger callback is the
//! `redirect_uri` (the only one registered with the Facebook app). The token
//! is stored with `encryptObject` (no AAD), as the ads reads expect.

use chatbotx_db::Db;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use super::credentials::Upstream;
use super::oauth::{self, OAuthCredential, ProviderFailure, call, get, query_url};
use super::repo_err;
use crate::connect_session::{self, NewSession, SessionRow};
use crate::errors::{BusinessError, Result};
use crate::tx;

pub const PROVIDER: &str = "facebookAds";
const GRAPH: &str = "https://graph.facebook.com";
const FACEBOOK: &str = "https://www.facebook.com";
/// `DEFAULT_API_VERSION` of `integrations/facebook-ads`.
const DEFAULT_VERSION: &str = "v23.0";
/// `FACEBOOK_ADS_SCOPES`.
const SCOPES: &[&str] = &[
    "ads_read",
    "ads_management",
    "pages_manage_ads",
    "pages_read_engagement",
    "pages_show_list",
];
const AUTHORIZED: &[&str] = &["authorized"];

/// `facebookAds.errors.invalidAppSettings` (en).
fn invalid_app_settings() -> BusinessError {
    BusinessError::exception("Facebook App settings are not valid", "systemError", 400)
}

fn version(config: &Value) -> String {
    config["version"]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_VERSION)
        .to_string()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// The settings return path (`refererPath` of `connectFacebookAds`).
pub fn settings_path(workspace_id: i64) -> String {
    format!("/space/{workspace_id}/settings/integrations")
}

/// The Messenger platform credential of the workspace's owner.
async fn credential(
    db: &Db,
    encryption: &chatbotx_core::encryption::Encryption,
    app_url: &str,
    owner: i64,
) -> Result<OAuthCredential> {
    oauth::resolve_credential(db, encryption, app_url, "messenger", Some(owner))
        .await?
        .filter(|c| !text(&c.config, "clientId").is_empty())
        .ok_or_else(invalid_app_settings)
}

/// `connectFacebookAds`: the Facebook dialog URL of a new connect session.
pub async fn start(
    db: &Db,
    encryption: &chatbotx_core::encryption::Encryption,
    app_url: &str,
    workspace_id: i64,
    user_id: i64,
) -> Result<String> {
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    let credential = credential(db, encryption, app_url, owner).await?;
    let base = format!("{FACEBOOK}/{}/dialog/oauth", version(&credential.config));
    let client_id = text(&credential.config, "clientId").to_string();
    let redirect = credential.callback_url.clone();
    let session = connect_session::create(
        db,
        NewSession {
            workspace_id,
            provider: PROVIDER.to_string(),
            purpose: "connect".to_string(),
            actor_user_id: Some(user_id),
            platform_owner_id: Some(owner.to_string()),
            return_url: Some(settings_path(workspace_id)),
            ..NewSession::default()
        },
        move |id, nonce| {
            let state = format!("{id}.{nonce}");
            let scope = SCOPES.join(",");
            json!({
                "type": "open_url",
                "url": query_url(&base, &[
                    ("client_id", &client_id),
                    ("redirect_uri", &redirect),
                    ("scope", &scope),
                    ("response_type", "code"),
                    ("state", &state),
                ]),
            })
        },
    )
    .await?;
    session
        .next_action
        .as_ref()
        .and_then(|action| action["url"].as_str())
        .map(str::to_string)
        .ok_or_else(invalid_app_settings)
}

/// How the callback stored the token (the audit wording differs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Created,
    Updated,
}

impl Stored {
    /// `integrationFacebookAdsService.upsert`'s audit `(action, detail)`.
    pub fn audit(self) -> (&'static str, &'static str) {
        match self {
            Stored::Created => ("connect", "connected a new Facebook Ads integration"),
            Stored::Updated => (
                "update",
                "updated the Facebook Ads integration configuration",
            ),
        }
    }
}

fn is_member(conn: &Connection, workspace_id: i64, user_id: i64) -> Result<bool> {
    conn.query_row(
        r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
        params![workspace_id, user_id],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(repo_err)
}

/// The callback: checks the session binding (the signed-in starter, still a
/// member, a `pending` session), exchanges `code` for a long-lived token and
/// stores it. A forged or replayed state is `connectionStateMismatch` and
/// leaves the session as is.
pub async fn complete(
    db: &Db,
    encryption: &chatbotx_core::encryption::Encryption,
    upstream: &impl Upstream,
    app_url: &str,
    session: &SessionRow,
    user_id: Option<i64>,
    code: &str,
) -> Result<Stored> {
    if session.provider != PROVIDER {
        return Err(oauth::state_mismatch());
    }
    let Some(user_id) = user_id.filter(|id| session.actor_user_id == Some(*id)) else {
        return Err(oauth::state_mismatch());
    };
    let workspace_id = session.workspace_id;
    if !tx::read(db, move |conn| is_member(conn, workspace_id, user_id)).await? {
        return Err(oauth::state_mismatch());
    }
    if session.status != "pending" {
        return Err(connect_session::expired(
            "This connect session is no longer active.",
        ));
    }
    connect_session::claim_authorization(db, session.id, workspace_id).await?;
    let outcome = exchange_and_store(db, encryption, upstream, app_url, session, code).await;
    if outcome.is_err() {
        connect_session::fail(db, session, "exchange_failed", Some(AUTHORIZED)).await;
    }
    outcome
}

fn rejected(failure: ProviderFailure) -> BusinessError {
    match failure {
        ProviderFailure::Rejected(message) => {
            BusinessError::exception(message, "connectionProviderRejected", 400)
        }
        ProviderFailure::Unavailable(status) => BusinessError::exception(
            "The provider is unavailable.",
            "connectionProviderUnavailable",
            status,
        ),
    }
}

async fn exchange_and_store(
    db: &Db,
    encryption: &chatbotx_core::encryption::Encryption,
    upstream: &impl Upstream,
    app_url: &str,
    session: &SessionRow,
    code: &str,
) -> Result<Stored> {
    if code.is_empty() {
        return Err(BusinessError::validation("code", "Code is required"));
    }
    let owner = session
        .platform_owner_id
        .as_deref()
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(oauth::state_mismatch)?;
    let credential = credential(db, encryption, app_url, owner).await?;
    let config = &credential.config;
    let version = version(config);
    let endpoint = format!("{GRAPH}/{version}/oauth/access_token");
    let short = call(
        upstream,
        get(query_url(
            &endpoint,
            &[
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("redirect_uri", &credential.callback_url),
                ("code", code),
            ],
        )),
    )
    .await
    .map_err(rejected)?;
    let long = call(
        upstream,
        get(query_url(
            &endpoint,
            &[
                ("grant_type", "fb_exchange_token"),
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("fb_exchange_token", text(&short, "access_token")),
            ],
        )),
    )
    .await
    .map_err(rejected)?;
    let token = text(&long, "access_token");
    if token.is_empty() {
        return Err(rejected(ProviderFailure::Rejected(
            "Facebook returned no access token.".into(),
        )));
    }
    let expires_at = long["expires_in"].as_i64().map(|s| now_ms() + s * 1000);
    let mut auth = json!({ "authType": "custom", "accessToken": token, "version": version });
    if let Some(at) = expires_at {
        auth["expiresAt"] = json!(iso(at));
    }
    let sealed = crate::email_marketing::seal_auth(&auth)?;
    let session = session.clone();
    tx::write(db, move |conn| {
        let now = now_ms();
        let ws = session.workspace_id;
        let existing: Option<i64> = conn
            .query_row(
                r#"SELECT "integrationId" FROM "IntegrationFacebookAds" WHERE "workspaceId" = ?1"#,
                [ws],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        let (integration_id, stored) = match existing {
            Some(integration_id) => {
                conn.execute(
                    r#"UPDATE "IntegrationFacebookAds" SET "auth" = ?2, "tokenExpiresAt" = ?3,
                           "status" = 'active', "updatedAt" = ?4 WHERE "workspaceId" = ?1"#,
                    params![ws, sealed, expires_at, now],
                )
                .map_err(repo_err)?;
                (integration_id, Stored::Updated)
            }
            None => {
                let integration_id = new_id();
                conn.execute(
                    r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'facebookAds')"#,
                    params![integration_id, ws],
                )
                .map_err(repo_err)?;
                conn.execute(
                    r#"INSERT INTO "IntegrationFacebookAds" ("id", "createdAt", "updatedAt", "workspaceId",
                           "integrationId", "auth", "tokenExpiresAt", "status")
                       VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 'active')"#,
                    params![new_id(), now, ws, integration_id, sealed, expires_at],
                )
                .map_err(repo_err)?;
                (integration_id, Stored::Created)
            }
        };
        let connection_id = attach_connection(conn, ws, integration_id, session.actor_user_id, now)?;
        let result = json!({
            "targetId": "workspace",
            "status": "connected",
            "connectionId": connection_id.to_string(),
        });
        connect_session::complete_authorized(conn, &session, result, Some(connection_id), now)?;
        Ok(stored)
    })
    .await
}

/// `attachIntegrationConnectionRow` (`sourceId: "workspace"`, "Facebook Ads").
// ponytail: the original attaches best-effort after the token commit; here a
// failed attach rolls the connect back.
fn attach_connection(
    conn: &Connection,
    ws: i64,
    integration_id: i64,
    actor: Option<i64>,
    now: i64,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "Connection" WHERE "workspaceId" = ?1 AND "provider" = 'facebookAds' AND "sourceId" = 'workspace'"#,
            [ws],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)?;
    match existing {
        Some(id) => {
            conn.execute(
                r#"UPDATE "Connection" SET "integrationId" = ?2, "lastError" = NULL, "status" = 'connected',
                       "statusReason" = NULL, "disconnectedAt" = NULL, "connectedAt" = ?3, "updatedAt" = ?3
                   WHERE "id" = ?1"#,
                params![id, integration_id, now],
            )
            .map_err(repo_err)?;
            Ok(id)
        }
        None => {
            let id = new_id();
            conn.execute(
                r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                       "kind", "integrationId", "sourceId", "displayName", "status", "createdBy", "connectedAt")
                   VALUES (?1, ?2, ?2, ?3, 'facebookAds', 'integration', ?4, 'workspace', 'Facebook Ads', 'connected', ?5, ?2)"#,
                params![id, now, ws, integration_id, actor],
            )
            .map_err(repo_err)?;
            Ok(id)
        }
    }
}

/// The settings page row (`integrationFacebookAdsResource`: no `auth`).
pub async fn settings_row(db: &Db, workspace_id: i64) -> Result<Option<Value>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "id", "integrationId", "tokenExpiresAt", "status" FROM "IntegrationFacebookAds"
               WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "workspaceId": workspace_id.to_string(),
                    "integrationId": r.get::<_, i64>(1)?.to_string(),
                    "tokenExpiresAt": r.get::<_, Option<i64>>(2)?.map(iso),
                    "status": r.get::<_, String>(3)?,
                }))
            },
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// `disconnectFacebookAdsAction`: a best-effort `DELETE /me/permissions`,
/// then the satellite and its `Integration` (the `Connection` row follows by
/// cascade). `false` when nothing is connected (`findByWorkspaceIdOrFail`).
pub async fn disconnect(db: &Db, upstream: &impl Upstream, workspace_id: i64) -> Result<bool> {
    let row: Option<(i64, String)> = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "integrationId", "auth" FROM "IntegrationFacebookAds" WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let Some((integration_id, raw)) = row else {
        return Ok(false);
    };
    match crate::email_marketing::open_auth(&raw) {
        Some(auth) if !text(&auth, "accessToken").is_empty() => {
            let url = query_url(
                &format!("{GRAPH}/{}/me/permissions", version(&auth)),
                &[("access_token", text(&auth, "accessToken"))],
            );
            let request = super::credentials::UpstreamRequest {
                method: "DELETE",
                ..get(url)
            };
            if call(upstream, request).await.is_err() {
                tracing::error!(
                    workspace_id,
                    "Unable to revoke Facebook Ads token for workspace"
                );
            }
        }
        _ => tracing::error!(
            workspace_id,
            "Unable to revoke Facebook Ads token for workspace"
        ),
    }
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "IntegrationFacebookAds" WHERE "workspaceId" = ?1"#,
            [workspace_id],
        )
        .map_err(repo_err)?;
        conn.execute(
            r#"DELETE FROM "Integration" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![integration_id, workspace_id],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await?;
    Ok(true)
}
