//! The channel token refresh: `refreshAllChannelTokensAction` (the user's
//! workspaces, `refreshed` / `failed` counts) and the `refreshChannelTokens`
//! schedule (every row; `schedule:refreshChannelTokens` audit source; the
//! WhatsApp Business Account credentials and the Threads tokens too).
//! Per row: the `auth:refresh:<channel>:<id>` lock → read → refresh or skip
//! → `updateAuth` + `recordRefreshedAuth` → audit; a failure stores
//! `tokenRefreshError` and marks the connection `needs_reauth` (revoked) or
//! `degraded` (`refresh_failed`).

use std::sync::OnceLock;

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::{OptionalExtension as _, params, params_from_iter};
use chatbotx_jobs::events::emit::EventBuses;
use chatbotx_kv::{LockMap, LockOpts};
use futures::future::join_all;
use serde_json::{Value, json};

use crate::audit::{AuditActor, AuditRecordInput, AuditService, SYSTEM_ACTOR, with_audit_context};
use crate::channel_api::{self as api, ApiFailure, ChannelHttp};
use crate::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use crate::{BusinessError, Result, tx};

const BATCH_SIZE: usize = 50;
/// `REFRESH_LOCK_TIMEOUT_SECONDS` of the action (outlives Zalo's 30 s OAuth
/// timeout: its refresh token is single-use).
const ACTION_LOCK_SECS: u64 = 60;
/// `REFRESH_SOURCE`.
pub const REFRESH_SOURCE: &str = "schedule:refreshChannelTokens";
/// `THREADS_TOKEN_REFRESH_THRESHOLD_DAYS`.
pub const THREADS_TOKEN_REFRESH_THRESHOLD_DAYS: i64 = 14;
const THREADS_LOCK_SECS: u64 = 30 * 60;

/// The `auth:refresh:*` locks (one map for the process: the action, the
/// schedule and the connection refresh share the keys).
pub fn refresh_locks() -> &'static LockMap {
    static LOCKS: OnceLock<LockMap> = OnceLock::new();
    LOCKS.get_or_init(LockMap::new)
}

/// What the refresh needs.
#[derive(Clone, Copy)]
pub struct RefreshDeps<'a> {
    pub db: &'a Db,
    pub http: &'a dyn ChannelHttp,
    pub audit: &'a AuditService,
    /// `logProviderError` of the schedule (`None` = not logged).
    pub buses: Option<&'a EventBuses>,
}

/// `RefreshSummary`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshSummary {
    pub refreshed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Refreshed,
    Skipped,
    Failed,
}

/// A channel with a per-row token refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshChannel {
    Zalo,
    Tiktok,
    Instagram,
    InstagramFacebook,
    Messenger,
    Whatsapp,
}

impl RefreshChannel {
    const ALL: [RefreshChannel; 6] = [
        RefreshChannel::Zalo,
        RefreshChannel::Tiktok,
        RefreshChannel::Instagram,
        RefreshChannel::InstagramFacebook,
        RefreshChannel::Messenger,
        RefreshChannel::Whatsapp,
    ];

    fn table(self) -> &'static str {
        match self {
            RefreshChannel::Zalo => "IntegrationZalo",
            RefreshChannel::Tiktok => "IntegrationTiktok",
            RefreshChannel::Instagram | RefreshChannel::InstagramFacebook => "IntegrationInstagram",
            RefreshChannel::Messenger => "IntegrationMessenger",
            RefreshChannel::Whatsapp => "IntegrationWhatsapp",
        }
    }

    /// The `Connection.sourceId` column.
    fn identifier(self) -> &'static str {
        match self {
            RefreshChannel::Zalo => "oaId",
            RefreshChannel::Tiktok => "openId",
            RefreshChannel::Instagram | RefreshChannel::InstagramFacebook => "igId",
            RefreshChannel::Messenger => "pageId",
            RefreshChannel::Whatsapp => "phoneNumberId",
        }
    }

    /// The `Connection.provider` of a row (SQL).
    fn provider_sql(self) -> &'static str {
        match self {
            RefreshChannel::Zalo => "'zalo'",
            RefreshChannel::Tiktok => "'tiktok'",
            RefreshChannel::Instagram | RefreshChannel::InstagramFacebook => {
                "CASE \"type\" WHEN 'facebook' THEN 'instagramFacebook' ELSE 'instagram' END"
            }
            RefreshChannel::Messenger => "'messenger'",
            RefreshChannel::Whatsapp => "'whatsapp'",
        }
    }

    /// The list filter (`findForTokenRefresh` / `findFacebookForTokenRefresh`).
    fn list_filter(self) -> &'static str {
        match self {
            RefreshChannel::Instagram => " AND \"type\" = 'instagram'",
            RefreshChannel::InstagramFacebook => " AND \"type\" = 'facebook'",
            _ => "",
        }
    }

    /// The lock key segment.
    fn lock(self) -> &'static str {
        match self {
            RefreshChannel::Zalo => "zalo",
            RefreshChannel::Tiktok => "tiktok",
            RefreshChannel::Instagram => "instagram",
            RefreshChannel::InstagramFacebook => "instagramFacebook",
            RefreshChannel::Messenger => "messenger",
            RefreshChannel::Whatsapp => "whatsapp",
        }
    }

    /// The `logProviderError` provider.
    fn error_provider(self) -> &'static str {
        match self {
            RefreshChannel::InstagramFacebook => "instagram",
            other => other.lock(),
        }
    }

    /// The audit detail object.
    fn noun(self) -> &'static str {
        match self {
            RefreshChannel::Zalo => "Zalo channel permissions",
            RefreshChannel::Tiktok => "TikTok channel token",
            RefreshChannel::Instagram | RefreshChannel::InstagramFacebook => {
                "Instagram channel token"
            }
            RefreshChannel::Messenger => "Messenger channel token",
            RefreshChannel::Whatsapp => "WhatsApp channel token",
        }
    }

    /// The schedule's lock timeout (Zalo: single-use refresh token).
    fn schedule_lock_secs(self) -> u64 {
        if self == RefreshChannel::Zalo { 60 } else { 10 }
    }
}

fn now_iso_plus(secs: i64) -> Value {
    json!(chatbotx_core::js::to_iso_string(repo::now_ms() + secs * 1000).unwrap_or_default())
}

fn secs(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f as i64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0)
}

fn str_of<'v>(value: &'v Value, key: &str) -> &'v str {
    value[key].as_str().unwrap_or("")
}

fn version_of(auth: &Value) -> String {
    auth["metadata"]["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(api::META_DEFAULT_VERSION)
        .to_owned()
}

fn set_token(auth: &mut Value, key: &str, value: Value) {
    if !auth["tokens"].is_object() {
        auth["tokens"] = json!({});
    }
    auth["tokens"][key] = value;
}

/// `refreshAuth` of the channel: the new auth, or `None` = skipped (no
/// refresh token, a manual WhatsApp token).
async fn refreshed_auth(
    http: &dyn ChannelHttp,
    channel: RefreshChannel,
    mut auth: Value,
) -> std::result::Result<Option<Value>, ApiFailure> {
    let refresh_token = auth["tokens"]["refreshToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    let access_token = str_of(&auth["tokens"], "accessToken").to_owned();
    match channel {
        RefreshChannel::Zalo => {
            let Some(refresh_token) = refresh_token else {
                return Ok(None);
            };
            let tokens = api::refresh_zalo_token(
                http,
                str_of(&auth, "clientId"),
                str_of(&auth, "clientSecret"),
                &refresh_token,
            )
            .await?;
            set_token(&mut auth, "accessToken", tokens["access_token"].clone());
            set_token(&mut auth, "refreshToken", tokens["refresh_token"].clone());
            set_token(
                &mut auth,
                "expiresAt",
                now_iso_plus(secs(&tokens["expires_in"])),
            );
        }
        RefreshChannel::Tiktok => {
            let Some(refresh_token) = refresh_token else {
                return Ok(None);
            };
            let data = api::refresh_tiktok_token(
                http,
                &auth["clientId"],
                &auth["clientSecret"],
                &refresh_token,
            )
            .await?;
            set_token(&mut auth, "accessToken", data["access_token"].clone());
            set_token(&mut auth, "refreshToken", data["refresh_token"].clone());
            set_token(
                &mut auth,
                "expiresAt",
                now_iso_plus(secs(&data["expires_in"])),
            );
            set_token(
                &mut auth,
                "refreshTokenExpiresAt",
                now_iso_plus(secs(&data["refresh_token_expires_in"])),
            );
            if !auth["metadata"].is_object() {
                auth["metadata"] = json!({});
            }
            auth["metadata"]["scopes"] = json!(api::parse_tiktok_scopes(&data["scope"]));
        }
        RefreshChannel::Instagram => {
            let (token, expires_in) = api::refresh_instagram_token(http, &access_token).await?;
            set_token(&mut auth, "accessToken", json!(token));
            set_token(&mut auth, "expiresAt", now_iso_plus(expires_in));
        }
        RefreshChannel::InstagramFacebook
        | RefreshChannel::Messenger
        | RefreshChannel::Whatsapp => {
            if channel == RefreshChannel::Whatsapp
                && auth["metadata"]["isManual"].as_bool() == Some(true)
            {
                return Ok(None);
            }
            // WhatsApp's `exchangeLongLivedToken` takes no version.
            let version = if channel == RefreshChannel::Whatsapp {
                api::META_DEFAULT_VERSION.to_owned()
            } else {
                version_of(&auth)
            };
            let token = api::exchange_long_lived_token(
                http,
                &version,
                str_of(&auth, "clientId"),
                str_of(&auth, "clientSecret"),
                &access_token,
            )
            .await?;
            set_token(&mut auth, "accessToken", json!(token));
        }
    }
    Ok(Some(auth))
}

/// `isRevokedTokenError` of the channel.
fn is_revoked(channel: RefreshChannel, failure: &ApiFailure) -> bool {
    match channel {
        RefreshChannel::Zalo => api::is_zalo_revoked(failure),
        RefreshChannel::Tiktok => api::is_tiktok_revoked(failure),
        RefreshChannel::Whatsapp => failure.is_whatsapp_revoked(),
        _ => failure.is_graph_revoked(),
    }
}

struct Stored {
    auth: Value,
    identifier: String,
    provider: String,
}

async fn load(
    db: &Db,
    channel: RefreshChannel,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Stored>> {
    let sql = format!(
        r#"SELECT "auth", "{}", {} FROM "{}" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        channel.identifier(),
        channel.provider_sql(),
        channel.table()
    );
    tx::read(db, move |conn| {
        conn.query_row(&sql, params![id, workspace_id], |r| {
            let auth: Option<String> = r.get(0)?;
            Ok(Stored {
                auth: auth
                    .and_then(|a| serde_json::from_str(&a).ok())
                    .unwrap_or(Value::Null),
                identifier: r.get(1)?,
                provider: r.get(2)?,
            })
        })
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

/// `recordRefreshedAuth`: an active `Connection` of `(provider, sourceId)`
/// back to `connected` with the new `authExpiresAt` (`oauth2` only).
pub fn record_refreshed_auth(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    provider: &str,
    source_id: &str,
    auth: &Value,
) -> chatbotx_db::rusqlite::Result<usize> {
    let expires_at = (auth["authType"] == "oauth2")
        .then(|| auth["tokens"]["expiresAt"].as_str())
        .flatten()
        .and_then(chatbotx_core::js::parse_date);
    conn.execute(
        r#"UPDATE "Connection" SET "status" = 'connected', "statusReason" = NULL,
               "authExpiresAt" = ?4, "lastError" = NULL, "updatedAt" = ?5
           WHERE "workspaceId" = ?1 AND "provider" = ?2 AND "sourceId" = ?3
             AND "status" IN ('connected', 'degraded')"#,
        params![
            workspace_id,
            provider,
            source_id,
            expires_at,
            repo::now_ms()
        ],
    )
}

/// `updateAuth` (clears `tokenRefreshError`) + `recordRefreshedAuth`.
async fn save(
    db: &Db,
    table: &'static str,
    workspace_id: i64,
    id: i64,
    stored: &Stored,
    auth: Value,
) -> Result<()> {
    let (provider, identifier) = (stored.provider.clone(), stored.identifier.clone());
    tx::write(db, move |tx| {
        tx.execute(
            &format!(
                r#"UPDATE "{table}" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?4
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#
            ),
            params![id, workspace_id, auth.to_string(), repo::now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        record_refreshed_auth(tx, workspace_id, &provider, &identifier, &auth)
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// `markTokenRefreshError`: the message on the row; revoked →
/// `markUnhealthyByIdentifier`, else `markDegradedByIdentifier`
/// (`refresh_failed`). A missing row is a warning.
/// A satellite table: its name, its `Connection.sourceId` column and its
/// `Connection.provider` (SQL).
#[derive(Debug, Clone, Copy)]
pub struct SatelliteTable {
    pub table: &'static str,
    pub identifier_column: &'static str,
    pub provider_sql: &'static str,
}

pub async fn mark_token_refresh_error(
    db: &Db,
    satellite: SatelliteTable,
    workspace_id: i64,
    id: i64,
    message: &str,
    revoked: bool,
) -> Result<()> {
    let SatelliteTable {
        table,
        identifier_column,
        provider_sql,
    } = satellite;
    let text = message.to_owned();
    let row = tx::write(db, move |tx| {
        let row: Option<(String, i64, String)> = tx
            .query_row(
                &format!(
                    r#"UPDATE "{table}" SET "tokenRefreshError" = ?3, "updatedAt" = ?4
                       WHERE "id" = ?1 AND "workspaceId" = ?2
                       RETURNING "{identifier_column}", "inboxId", {provider_sql}"#
                ),
                params![id, workspace_id, text, repo::now_ms()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        if let (Some((identifier, _, provider)), false) = (&row, revoked) {
            tx.execute(
                r#"UPDATE "Connection" SET "status" = 'degraded', "statusReason" = 'refresh_failed',
                       "updatedAt" = ?4
                   WHERE "workspaceId" = ?1 AND "provider" = ?2 AND "sourceId" = ?3
                     AND "status" IN ('connected', 'degraded')"#,
                params![workspace_id, provider, identifier, repo::now_ms()],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        }
        Ok(row)
    })
    .await?;
    let Some((identifier, inbox_id, provider)) = row else {
        tracing::warn!(
            integration_id = id,
            workspace_id,
            table,
            "Unable to mark token refresh error: integration not found"
        );
        return Ok(());
    };
    if revoked {
        crate::connection::mark_unhealthy_by_identifier(
            db,
            &provider,
            &identifier,
            workspace_id,
            inbox_id,
        )
        .await?;
    }
    Ok(())
}

/// One row: `refreshGuarded` (the action) / `refreshWithErrorHandling` (the
/// schedule, `scheduled = true`).
async fn refresh_row(
    deps: RefreshDeps<'_>,
    channel: RefreshChannel,
    workspace_id: i64,
    id: i64,
    scheduled: bool,
) -> Outcome {
    let ttl_secs = if scheduled {
        channel.schedule_lock_secs()
    } else {
        ACTION_LOCK_SECS
    };
    let key = format!("auth:refresh:{}:{id}", channel.lock());
    let _guard = match refresh_locks()
        .acquire(
            &key,
            LockOpts {
                ttl_secs,
                retry_secs: None,
            },
        )
        .await
    {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!(%error, id, workspace_id, "[{}] lock acquisition failed", channel.lock());
            return Outcome::Failed;
        }
    };
    let failure = match load(deps.db, channel, workspace_id, id).await {
        // Zalo / TikTok `findById` throws on a missing row.
        Ok(None) if matches!(channel, RefreshChannel::Zalo | RefreshChannel::Tiktok) => {
            ApiFailure::new("Integration not found")
        }
        Ok(None) => return Outcome::Skipped,
        Err(error) => ApiFailure::new(error.to_string()),
        Ok(Some(stored)) => match refreshed_auth(deps.http, channel, stored.auth.clone()).await {
            Ok(None) => return Outcome::Skipped,
            Err(failure) => failure,
            Ok(Some(auth)) => {
                match save(deps.db, channel.table(), workspace_id, id, &stored, auth).await {
                    Err(error) => ApiFailure::new(error.to_string()),
                    Ok(()) => {
                        let detail = if scheduled {
                            format!("auto-refreshed the {}", channel.noun())
                        } else {
                            format!("refreshed the {}", channel.noun())
                        };
                        let mut input = AuditRecordInput::new("refresh", &detail);
                        input.workspace_id = Some(workspace_id.to_string());
                        input.source = scheduled.then(|| REFRESH_SOURCE.to_owned());
                        deps.audit.record(input).await;
                        return Outcome::Refreshed;
                    }
                }
            }
        },
    };
    tracing::error!(
        err = %failure,
        integration_id = id,
        workspace_id,
        "Failed to refresh {} channel token",
        channel.lock()
    );
    if let Err(error) = mark_token_refresh_error(
        deps.db,
        SatelliteTable {
            table: channel.table(),
            identifier_column: channel.identifier(),
            provider_sql: channel.provider_sql(),
        },
        workspace_id,
        id,
        &failure.message,
        is_revoked(channel, &failure),
    )
    .await
    {
        tracing::error!(%error, integration_id = id, workspace_id, "Failed to record token refresh error");
    }
    if scheduled {
        log_failure(deps.buses, channel.error_provider(), workspace_id, &failure).await;
    }
    Outcome::Failed
}

async fn log_failure(
    buses: Option<&EventBuses>,
    provider: &str,
    workspace_id: i64,
    failure: &ApiFailure,
) {
    let error = Thrown::Error {
        message: failure.message.clone(),
        stack: None,
        status: failure.status.map(i64::from),
    };
    log_provider_error(
        buses,
        ProviderErrorInput::new(provider, &workspace_id.to_string(), error),
    )
    .await;
}

/// The `(id, workspaceId)` rows of a channel (`workspaces = None`: all).
async fn list_rows(
    db: &Db,
    channel: RefreshChannel,
    workspaces: Option<Vec<i64>>,
) -> Result<Vec<(i64, i64)>> {
    let scope = match &workspaces {
        None => String::new(),
        Some(ids) => format!(
            r#" AND "workspaceId" IN ({})"#,
            vec!["?"; ids.len()].join(", ")
        ),
    };
    let sql = format!(
        r#"SELECT "id", "workspaceId" FROM "{}" WHERE 1 = 1{}{scope} ORDER BY "id""#,
        channel.table(),
        channel.list_filter()
    );
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params_from_iter(workspaces.unwrap_or_default()), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .and_then(Iterator::collect)
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await
}

async fn refresh_channel(
    deps: RefreshDeps<'_>,
    channel: RefreshChannel,
    workspaces: Option<Vec<i64>>,
) -> Result<RefreshSummary> {
    let scheduled = workspaces.is_none();
    let rows = list_rows(deps.db, channel, workspaces).await?;
    let mut summary = RefreshSummary::default();
    for batch in rows.chunks(BATCH_SIZE) {
        let outcomes = join_all(batch.iter().map(|&(id, workspace_id)| {
            let run = refresh_row(deps, channel, workspace_id, id, scheduled);
            async move {
                if scheduled {
                    with_audit_context(system_actor(workspace_id), run).await
                } else {
                    run.await
                }
            }
        }))
        .await;
        for outcome in outcomes {
            match outcome {
                Outcome::Refreshed => summary.refreshed += 1,
                Outcome::Failed => summary.failed += 1,
                Outcome::Skipped => {}
            }
        }
    }
    Ok(summary)
}

fn system_actor(workspace_id: i64) -> AuditActor {
    AuditActor {
        user_id: Some(SYSTEM_ACTOR.into()),
        workspace_id: Some(workspace_id.to_string()),
        source: Some(REFRESH_SOURCE.into()),
        ..AuditActor::default()
    }
}

/// `refreshAllChannelTokensAction` over `workspace_ids` (already filtered:
/// member, not scheduled for deletion).
pub async fn refresh_for_workspaces(
    deps: RefreshDeps<'_>,
    workspace_ids: &[i64],
) -> Result<RefreshSummary> {
    if workspace_ids.is_empty() {
        return Ok(RefreshSummary::default());
    }
    let runs = RefreshChannel::ALL
        .map(|channel| refresh_channel(deps, channel, Some(workspace_ids.to_vec())));
    let mut total = RefreshSummary::default();
    for summary in join_all(runs).await {
        let summary = summary?;
        total.refreshed += summary.refreshed;
        total.failed += summary.failed;
    }
    Ok(total)
}

/// The `refreshChannelTokens` schedule. `channels` = `data.channels`
/// (`None` = all): `zalo`, `tiktok`, `threads`, `instagram` (both Instagram
/// logins), `messenger`, `whatsapp`; the others have no refresh.
pub async fn refresh_scheduled(deps: RefreshDeps<'_>, channels: Option<&[String]>) {
    let names = [
        "zalo",
        "tiktok",
        "threads",
        "instagram",
        "messenger",
        "whatsapp",
    ];
    let runs = names
        .into_iter()
        .filter(|name| channels.is_none_or(|list| list.iter().any(|c| c == name)))
        .map(|name| async move { (name, run_scheduled(deps, name).await) });
    let mut failed = 0;
    for (channel, result) in join_all(runs).await {
        if let Err(error) = result {
            failed += 1;
            tracing::error!(%error, "[refreshChannelTokens] channel={channel} failed");
        }
    }
    if failed > 0 {
        tracing::warn!("[refreshChannelTokens] {failed} channel(s) failed to run");
    }
}

async fn run_scheduled(deps: RefreshDeps<'_>, name: &str) -> Result<()> {
    let one = |channel| async move { refresh_channel(deps, channel, None).await.map(drop) };
    match name {
        "zalo" => one(RefreshChannel::Zalo).await,
        "tiktok" => one(RefreshChannel::Tiktok).await,
        "threads" => refresh_threads(deps).await,
        "instagram" => {
            let (direct, facebook) = futures::future::join(
                one(RefreshChannel::Instagram),
                one(RefreshChannel::InstagramFacebook),
            )
            .await;
            direct.and(facebook)
        }
        "messenger" => one(RefreshChannel::Messenger).await,
        _ => {
            // The WABA list is read before the rows are refreshed.
            let wabas = waba_rows(deps.db).await?;
            one(RefreshChannel::Whatsapp).await?;
            for batch in wabas.chunks(BATCH_SIZE) {
                join_all(batch.iter().map(|(id, workspace_id, waba_id)| {
                    with_audit_context(
                        system_actor(*workspace_id),
                        refresh_waba(deps, *id, *workspace_id, waba_id),
                    )
                }))
                .await;
            }
            Ok(())
        }
    }
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// One `(id, workspaceId, wabaId)` per `(workspaceId, wabaId)` of the
/// non-manual WhatsApp rows (the first row wins).
async fn waba_rows(db: &Db) -> Result<Vec<(i64, i64, String)>> {
    let rows: Vec<(i64, i64, Option<String>, Option<String>)> = tx::read(db, |conn| {
        let mut stmt = conn
            .prepare(r#"SELECT "id", "workspaceId", "wabaId", "auth" FROM "IntegrationWhatsapp" ORDER BY "id""#)
            .map_err(repo_err)?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .and_then(Iterator::collect)
            .map_err(repo_err)
    })
    .await?;
    let mut seen = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter_map(|(id, workspace_id, waba_id, auth)| {
            let auth: Value = auth
                .and_then(|a| serde_json::from_str(&a).ok())
                .unwrap_or_default();
            let waba_id = waba_id?;
            let manual = auth["metadata"]["isManual"].as_bool() == Some(true);
            (!manual && seen.insert((workspace_id, waba_id.clone()))).then_some((
                id,
                workspace_id,
                waba_id,
            ))
        })
        .collect())
}

/// `refreshWabaCredential`: the encrypted WABA credential re-exchanged and
/// stored with a revision CAS (a refresh losing to a reconnect is a no-op).
async fn refresh_waba(deps: RefreshDeps<'_>, id: i64, workspace_id: i64, waba_id: &str) {
    let key = format!("auth:refresh:whatsapp:waba:{workspace_id}:{waba_id}");
    let outcome = match refresh_locks()
        .acquire(
            &key,
            LockOpts {
                ttl_secs: 10,
                retry_secs: None,
            },
        )
        .await
    {
        Err(error) => Err(ApiFailure::new(error.to_string())),
        Ok(_guard) => refresh_waba_locked(deps, id, workspace_id, waba_id).await,
    };
    if let Err(failure) = outcome {
        tracing::error!(err = %failure, "[refreshWhatsappTokens] wabaId={waba_id} failed");
        log_failure(deps.buses, "whatsapp", workspace_id, &failure).await;
    }
}

async fn refresh_waba_locked(
    deps: RefreshDeps<'_>,
    id: i64,
    workspace_id: i64,
    waba_id: &str,
) -> std::result::Result<(), ApiFailure> {
    use chatbotx_db::repo::whatsapp_business_account as waba;
    let fail = |e: &dyn std::fmt::Display| ApiFailure::new(e.to_string());
    let owned = waba_id.to_owned();
    let account = tx::read(deps.db, move |conn| {
        waba::find_by_waba(conn, workspace_id, &owned).map_err(BusinessError::Repo)
    })
    .await
    .map_err(|e| fail(&e))?;
    // Legacy phone rows are not backfilled by this cron.
    let Some(account) = account else {
        return Ok(());
    };
    let aad = format!("whatsapp-waba:{workspace_id}:{waba_id}");
    let encryption = crate::meta_catalog::process_encryption();
    let sealed: chatbotx_core::encryption::EncryptedData =
        serde_json::from_value(account.credential.0.clone()).map_err(|e| fail(&e))?;
    let mut credential: Value = encryption
        .decrypt_object(&sealed, Some(&aad))
        .map_err(|e| fail(&e))?;
    let current: Option<(Option<String>, Option<String>)> = tx::read(deps.db, move |conn| {
        conn.query_row(
            r#"SELECT "auth", "wabaId" FROM "IntegrationWhatsapp" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo_err)
    })
    .await
    .map_err(|e| fail(&e))?;
    let Some((auth, Some(row_waba))) = current else {
        return Ok(());
    };
    let auth: Value = auth
        .and_then(|a| serde_json::from_str(&a).ok())
        .unwrap_or_default();
    if row_waba != waba_id || auth["metadata"]["isManual"].as_bool() == Some(true) {
        return Ok(());
    }
    let token = api::exchange_long_lived_token(
        deps.http,
        api::META_DEFAULT_VERSION,
        str_of(&auth, "clientId"),
        str_of(&auth, "clientSecret"),
        str_of(&credential, "accessToken"),
    )
    .await?;
    credential["accessToken"] = json!(token);
    let sealed = encryption
        .encrypt_object(&credential, Some(&aad))
        .map_err(|e| fail(&e))?;
    let input = waba::UpsertCredentialInput {
        workspace_id,
        waba_id: waba_id.to_owned(),
        business_id: account.business_id.clone(),
        credential: repo::Json(serde_json::to_value(sealed).map_err(|e| fail(&e))?),
        granted_scopes: serde_json::from_value(account.granted_scopes.0.clone())
            .unwrap_or_default(),
        scope_checked_at: account.scope_checked_at.unwrap_or_else(repo::now_ms),
        expected_revision: account.revision,
    };
    let updated = tx::write(deps.db, move |tx| {
        waba::upsert_credential(tx, &input).map_err(BusinessError::Repo)
    })
    .await
    .map_err(|e| fail(&e))?;
    if updated.is_some() {
        let mut input = AuditRecordInput::new(
            "refresh",
            "auto-refreshed the WhatsApp Business Account credential",
        );
        input.workspace_id = Some(workspace_id.to_string());
        input.source = Some(REFRESH_SOURCE.to_owned());
        deps.audit.record(input).await;
    }
    Ok(())
}

/// `^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}` (the SQL guard of
/// `listDueForTokenRefresh`).
fn looks_like_timestamp(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        range
            .into_iter()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
    digits(0..4)
        && b.get(4) == Some(&b'-')
        && digits(5..7)
        && b.get(7) == Some(&b'-')
        && digits(8..10)
        && matches!(b.get(10), Some(b'T' | b' '))
        && digits(11..13)
        && b.get(13) == Some(&b':')
        && digits(14..16)
}

/// `refreshThreadsTokens`: under `schedule:refresh-threads-tokens` (30 min),
/// the rows whose token expires within 14 days (or has no `expiresAt`),
/// one by one; the save is a CAS on the old access token.
async fn refresh_threads(deps: RefreshDeps<'_>) -> Result<()> {
    let _guard = refresh_locks()
        .acquire(
            "schedule:refresh-threads-tokens",
            LockOpts {
                ttl_secs: THREADS_LOCK_SECS,
                retry_secs: None,
            },
        )
        .await
        .map_err(|e| BusinessError::exception(e.to_string(), "lockFailed", 500))?;
    let refresh_before =
        repo::now_ms() + THREADS_TOKEN_REFRESH_THRESHOLD_DAYS * 24 * 60 * 60 * 1000;
    let rows: Vec<(i64, i64, Option<String>)> = tx::read(deps.db, |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "workspaceId", "auth" FROM "IntegrationThreads"
                   WHERE json_extract("auth", '$.tokens.accessToken') IS NOT NULL ORDER BY "id""#,
            )
            .map_err(repo_err)?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .and_then(Iterator::collect)
            .map_err(repo_err)
    })
    .await?;
    let due = rows.into_iter().filter_map(|(id, workspace_id, auth)| {
        let auth: Value = serde_json::from_str(auth.as_deref()?).ok()?;
        let token = auth["tokens"]["accessToken"]
            .as_str()
            .filter(|t| !t.is_empty())?
            .to_owned();
        let is_due = match auth["tokens"]["expiresAt"].as_str() {
            None => true,
            Some(at) if looks_like_timestamp(at) => {
                chatbotx_core::js::parse_date(at).is_some_and(|ms| ms <= refresh_before)
            }
            Some(_) => false,
        };
        is_due.then_some((id, workspace_id, auth, token))
    });
    for (id, workspace_id, mut auth, token) in due {
        match api::refresh_threads_token(deps.http, &token).await {
            Ok((new_token, expires_in)) => {
                set_token(&mut auth, "accessToken", json!(new_token));
                if let Some(expires_in) = expires_in {
                    set_token(&mut auth, "expiresAt", now_iso_plus(expires_in));
                }
                let updated = tx::write(deps.db, move |tx| {
                    tx.execute(
                        r#"UPDATE "IntegrationThreads" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?5
                           WHERE "id" = ?1 AND "workspaceId" = ?2
                             AND json_extract("auth", '$.tokens.accessToken') = ?4"#,
                        params![id, workspace_id, auth.to_string(), token, repo::now_ms()],
                    )
                    .map_err(repo_err)
                })
                .await?;
                tracing::info!(
                    integration_id = id,
                    workspace_id,
                    refreshed = updated > 0,
                    "refreshThreadsTokens: integration done"
                );
            }
            Err(failure) => {
                tracing::error!(status = ?failure.status, code = ?failure.code, integration_id = id, workspace_id, "refreshThreadsTokens: integration refresh failed");
                mark_token_refresh_error(
                    deps.db,
                    SatelliteTable {
                        table: "IntegrationThreads",
                        identifier_column: "threadsUserId",
                        provider_sql: "'threads'",
                    },
                    workspace_id,
                    id,
                    &failure.message,
                    failure.is_graph_revoked(),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// `connectionService.refresh` for the OAuth channel providers (Zalo,
/// TikTok, Messenger, both Instagram logins, Threads): under
/// `auth:refresh:connection:<id>` (10 s), a forced `refreshAuth` of the
/// satellite row's auth (3 attempts, 250 ms backoff; a revoked token marks
/// the connection `needs_reauth`), then the save + `recordAuthSaved`.
/// `Ok(false)` = the provider has no channel refresh here.
pub async fn refresh_connection(
    db: &Db,
    http: &dyn ChannelHttp,
    connection: &crate::connection::ConnectionRow,
) -> Result<bool> {
    let channel = match connection.provider.as_str() {
        "zalo" => Some(RefreshChannel::Zalo),
        "tiktok" => Some(RefreshChannel::Tiktok),
        "instagram" => Some(RefreshChannel::Instagram),
        "instagramFacebook" => Some(RefreshChannel::InstagramFacebook),
        "messenger" => Some(RefreshChannel::Messenger),
        "threads" => None,
        _ => return Ok(false),
    };
    let table = channel.map_or("IntegrationThreads", RefreshChannel::table);
    // A channel connection points at its satellite row through `inboxId`.
    let Some(inbox_id) = connection.inbox_id else {
        return Err(crate::connection::not_configured(&connection.provider));
    };
    let _guard = refresh_locks()
        .acquire(
            &format!("auth:refresh:connection:{}", connection.id),
            LockOpts {
                ttl_secs: 10,
                retry_secs: None,
            },
        )
        .await
        .map_err(|e| BusinessError::exception(e.to_string(), "lockFailed", 500))?;
    let workspace_id = connection.workspace_id;
    let sql = format!(
        r#"SELECT "id", "auth" FROM "{table}" WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#
    );
    let row: Option<(i64, Option<String>)> = tx::read(db, move |conn| {
        conn.query_row(&sql, params![inbox_id, workspace_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let Some((integration_id, Some(auth))) = row else {
        return Err(crate::connection::not_configured(&connection.provider));
    };
    let Ok(auth) = serde_json::from_str::<Value>(&auth) else {
        return Err(crate::connection::not_configured(&connection.provider));
    };
    if auth["authType"] != "oauth2" {
        return Ok(true);
    }
    let mut attempts = 0;
    let outcome = loop {
        attempts += 1;
        let result = match channel {
            Some(channel) => refreshed_auth(http, channel, auth.clone()).await,
            None => refresh_threads_auth(http, auth.clone()).await,
        };
        let failure = match result {
            Ok(done) => break Ok(done),
            Err(failure) => failure,
        };
        let revoked =
            channel.map_or_else(|| failure.is_graph_revoked(), |c| is_revoked(c, &failure));
        if revoked || attempts == 3 {
            break Err((failure, revoked));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250 << (attempts - 1))).await;
    };
    let new_auth = match outcome {
        Ok(Some(auth)) => auth,
        Ok(None) => return Ok(true),
        Err((failure, revoked)) => {
            if revoked {
                if let Err(error) = crate::connection::mark_unhealthy_by_identifier(
                    db,
                    &connection.provider,
                    &connection.source_id,
                    workspace_id,
                    inbox_id,
                )
                .await
                {
                    tracing::error!(%error, connection_id = connection.id, "Failed to mark integration auth offline");
                }
            }
            tracing::warn!(err = %failure, connection_id = connection.id, "connection refresh failed");
            return Err(BusinessError::exception(
                format!(
                    "Integration \"{}\" auth refresh failed after {attempts} attempt(s).",
                    connection.provider
                ),
                "unknownError",
                401,
            ));
        }
    };
    let (id, provider, source_id) = (
        integration_id,
        connection.provider.clone(),
        connection.source_id.clone(),
    );
    tx::write(db, move |tx| {
        let saved = tx
            .execute(
                &format!(
                    r#"UPDATE "{table}" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?4
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#
                ),
                params![id, workspace_id, new_auth.to_string(), repo::now_ms()],
            )
            .map_err(repo_err)?;
        if saved == 0 {
            return Err(BusinessError::exception(
                "Connection auth persistence did not match a satellite row",
                "systemError",
                500,
            ));
        }
        record_refreshed_auth(tx, workspace_id, &provider, &source_id, &new_auth).map_err(repo_err)?;
        Ok(())
    })
    .await?;
    Ok(true)
}

/// Threads `refreshAuth`.
async fn refresh_threads_auth(
    http: &dyn ChannelHttp,
    mut auth: Value,
) -> std::result::Result<Option<Value>, ApiFailure> {
    let token = str_of(&auth["tokens"], "accessToken").to_owned();
    if token.is_empty() {
        return Err(ApiFailure::new("Threads access token not available"));
    }
    let (new_token, expires_in) = api::refresh_threads_token(http, &token).await?;
    set_token(&mut auth, "accessToken", json!(new_token));
    if let Some(expires_in) = expires_in {
        set_token(&mut auth, "expiresAt", now_iso_plus(expires_in));
    }
    Ok(Some(auth))
}
