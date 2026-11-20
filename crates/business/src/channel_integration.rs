//! `channel-integration/service.ts` (the `tiktok` and `zalo` fetchers),
//! `tiktokIntegrationService.{set,refresh}CommentToMessage` with
//! `apis/direct-reply.ts`, and `zaloIntegrationService.updateTagSync`
//! (Q.I2: `tiktokChannels.*`, `zaloChannels.*`).

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use crate::audit::AuditService;
use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::{BusinessError, Result, tx};

/// `BUSINESS_API_URL`.
pub const TIKTOK_BUSINESS_API_URL: &str = "https://business-api.tiktok.com/open_api/v1.3/";

/// `ChannelIntegrationSummary` of a TikTok or Zalo row (no credentials).
pub fn summary(
    channel: &str,
    id: i64,
    inbox_id: i64,
    name: &str,
    external_id: &str,
    sync_tag_enabled_at: Option<i64>,
    token_refresh_error: Option<String>,
) -> Value {
    json!({
        "id": id.to_string(),
        "channel": channel,
        "inboxId": inbox_id.to_string(),
        "name": name,
        "externalId": external_id,
        "wabaId": null,
        "displayPhoneNumber": null,
        "username": null,
        "coexistEnabled": false,
        "isCoexist": null,
        "hasCapiScope": false,
        "capiConnected": false,
        "datasetId": null,
        "capiTestEventCode": null,
        "capiDisconnected": false,
        "syncTagEnabledAt": sync_tag_enabled_at.map(|ms| chatbotx_core::js::to_iso_string(ms).unwrap_or_default()),
        "handoverResumeFlowId": null,
        "tokenRefreshError": token_refresh_error,
        // `adsEligibleChannelTypes` is WhatsApp / Messenger / Instagram.
        "adsEligible": false,
    })
}

/// `channelIntegrationService.list` / `get` for `tiktok` or `zalo`
/// (`None` = another channel).
pub async fn list(
    db: &Db,
    channel: &str,
    workspace_id: i64,
    id: Option<i64>,
) -> Result<Option<Vec<Value>>> {
    let (sql, zalo) = match channel {
        "tiktok" => (
            r#"SELECT "id", "inboxId", "name", "openId", NULL, "tokenRefreshError" FROM "IntegrationTiktok""#,
            false,
        ),
        "zalo" => (
            r#"SELECT "id", "inboxId", "name", "oaId", "syncTagEnabledAt", "tokenRefreshError" FROM "IntegrationZalo""#,
            true,
        ),
        _ => return Ok(None),
    };
    let channel = channel.to_owned();
    let rows = tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                r#"{sql} WHERE "workspaceId" = ?1 AND (?2 IS NULL OR "id" = ?2)"#
            ))
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params![workspace_id, id], |r| {
                Ok(summary(
                    &channel,
                    r.get(0)?,
                    r.get(1)?,
                    &r.get::<_, String>(2)?,
                    &r.get::<_, String>(3)?,
                    if zalo { r.get(4)? } else { None },
                    r.get(5)?,
                ))
            })
            .map_err(|e| BusinessError::Repo(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await?;
    Ok(Some(rows))
}

/// `zaloIntegrationService.updateTagSync`: the new `syncTagEnabledAt`.
pub async fn update_zalo_tag_sync(
    db: &Db,
    workspace_id: i64,
    id: i64,
    enabled: bool,
) -> Result<Option<i64>> {
    let at = enabled.then(repo::now_ms);
    tx::write(db, move |tx| {
        let changed = tx
            .execute(
                r#"UPDATE "IntegrationZalo" SET "syncTagEnabledAt" = ?3, "updatedAt" = ?4
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id, at, repo::now_ms()],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        if changed == 0 {
            return Err(BusinessError::not_found("Zalo channel not found"));
        }
        Ok(at)
    })
    .await
}

async fn tiktok_auth(db: &Db, workspace_id: i64, id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let auth: Option<String> = conn
            .query_row(
                r#"SELECT "auth" FROM "IntegrationTiktok" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                chatbotx_db::rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(BusinessError::Repo(e.into())),
            })?;
        let auth = auth.ok_or_else(|| BusinessError::not_found("Integration not found"))?;
        Ok(serde_json::from_str(&auth).unwrap_or(Value::Null))
    })
    .await
}

/// One `business/message/direct_reply/*` call: TikTok's own `message` on a
/// non-zero `code` (`rescue` + the `code !== 0` check).
async fn direct_reply_call(
    upstream: &impl Upstream,
    base: &str,
    token: &str,
    request: (&'static str, String, Option<Value>),
    fallback: &str,
) -> std::result::Result<Value, String> {
    let (method, path, body) = request;
    let reply = upstream
        .send(UpstreamRequest {
            method,
            url: format!("{base}{path}"),
            headers: vec![
                ("Access-Token".into(), token.to_owned()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: body.map(|b| b.to_string()),
        })
        .await?;
    let parsed: Value = serde_json::from_str(&reply.1).unwrap_or(Value::Null);
    if !(200..300).contains(&reply.0) {
        return Err(parsed["error"]["message"]
            .as_str()
            .unwrap_or(path.split('?').next().unwrap_or(""))
            .to_owned());
    }
    if parsed["code"].as_i64() != Some(0) {
        return Err(parsed["message"].as_str().unwrap_or(fallback).to_owned());
    }
    Ok(parsed["data"].clone())
}

/// `cacheCommentToMessageStatus` + `updateAuth` (clears `tokenRefreshError`).
// ponytail: `recordRefreshedAuth` (Connection `authExpiresAt`) is not called;
// the tokens are unchanged by this write.
async fn cache_status(
    db: &Db,
    workspace_id: i64,
    id: i64,
    mut auth: Value,
    status: &str,
) -> Result<()> {
    if !auth["metadata"].is_object() {
        auth["metadata"] = json!({});
    }
    auth["metadata"]["commentToMessage"] = json!({
        "status": status,
        "checkedAt": chatbotx_core::js::to_iso_string(repo::now_ms()).unwrap_or_default(),
    });
    tx::write(db, move |tx| {
        let changed = tx
            .execute(
                r#"UPDATE "IntegrationTiktok" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?4
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id, auth.to_string(), repo::now_ms()],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        if changed == 0 {
            return Err(BusinessError::not_found("TikTok integration not found"));
        }
        Ok(())
    })
    .await
}

fn system_error(message: String) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

/// `tiktokIntegrationService.setCommentToMessage`.
pub async fn set_comment_to_message(
    db: &Db,
    upstream: &impl Upstream,
    audit: &AuditService,
    base: &str,
    workspace_id: i64,
    id: i64,
    enabled: bool,
) -> Result<String> {
    let auth = tiktok_auth(db, workspace_id, id).await?;
    let status = if enabled { "ENABLE" } else { "DISABLE" };
    // `tiktokCanMessage`.
    if auth["metadata"]["features"]["messaging"] == false {
        return Err(system_error("This TikTok connection has no messaging permission, so Comment-to-Message cannot be used. TikTok Business Messaging is not available to accounts registered in the EEA, Switzerland or the UK.".into()));
    }
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or("");
    let business_id = auth["metadata"]["openId"].as_str().unwrap_or("");
    let body = json!({
        "business_id": business_id,
        "direct_reply_type": "COMMENT_TO_MESSAGE",
        "operation_status": status,
    });
    if let Err(message) = direct_reply_call(
        upstream,
        base,
        token,
        (
            "POST",
            "business/message/direct_reply/update/".into(),
            Some(body),
        ),
        "Failed to update the TikTok Comment-to-Message setting",
    )
    .await
    {
        tracing::error!(
            err = message,
            id,
            workspace_id,
            "Failed to update TikTok Comment-to-Message"
        );
        return Err(system_error(message));
    }
    cache_status(db, workspace_id, id, auth, status).await?;
    let verb = if enabled { "enabled" } else { "disabled" };
    audit
        .audit("update", &format!("{verb} TikTok Comment-to-Message"))
        .await;
    Ok(status.to_owned())
}

/// `tiktokIntegrationService.refreshCommentToMessage`: `None` when TikTok
/// answered without a status (the cache is left alone).
pub async fn refresh_comment_to_message(
    db: &Db,
    upstream: &impl Upstream,
    base: &str,
    workspace_id: i64,
    id: i64,
) -> Result<Option<String>> {
    let auth = tiktok_auth(db, workspace_id, id).await?;
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or("");
    let business_id = auth["metadata"]["openId"].as_str().unwrap_or("");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("business_id", business_id)
        .append_pair("direct_reply_type", "COMMENT_TO_MESSAGE")
        .finish();
    let data = direct_reply_call(
        upstream,
        base,
        token,
        (
            "GET",
            format!("business/message/direct_reply/get/?{query}"),
            None,
        ),
        "Failed to read the TikTok Comment-to-Message setting",
    )
    .await
    .map_err(|message| {
        tracing::error!(
            err = message,
            id,
            workspace_id,
            "Failed to read the TikTok Comment-to-Message setting"
        );
        system_error(message)
    })?;
    let Some(status) = data["operation_status"].as_str().map(str::to_owned) else {
        return Ok(None);
    };
    cache_status(db, workspace_id, id, auth, &status).await?;
    Ok(Some(status))
}

/// The settings-table rows of `listIntegrationTiktoks` (`toResource`) /
/// `listIntegrationZalo` / `listIntegrationThreads`, oldest first. Only
/// fields derived from `auth` leave this side; the tokens and app secret
/// never do.
pub async fn settings_list(db: &Db, channel: &str, workspace_id: i64) -> Result<Vec<Value>> {
    if channel == "threads" {
        return threads_list(db, workspace_id).await;
    }
    let tiktok = match channel {
        "tiktok" => true,
        "zalo" => false,
        _ => return Err(BusinessError::not_found("Channel not found")),
    };
    tx::read(db, move |conn| {
        let sql = if tiktok {
            r#"SELECT "id", "inboxId", "name", "openId", "tokenRefreshError", "auth"
               FROM "IntegrationTiktok" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#
        } else {
            r#"SELECT "id", "inboxId", "name", "oaId", "tokenRefreshError", "syncTagEnabledAt"
               FROM "IntegrationZalo" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params![workspace_id], |r| {
                let mut out = json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "inboxId": r.get::<_, i64>(1)?.to_string(),
                    "name": r.get::<_, String>(2)?,
                    "tokenRefreshError": r.get::<_, Option<String>>(4)?,
                });
                if tiktok {
                    let auth: Value =
                        serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null);
                    out["openId"] = json!(r.get::<_, String>(3)?);
                    out["needsReauthorization"] = json!(
                        crate::connection::oauth_channels::tiktok_needs_reauthorization(&auth)
                    );
                    out["commentToMessageStatus"] =
                        auth["metadata"]["commentToMessage"]["status"].clone();
                } else {
                    out["oaId"] = json!(r.get::<_, String>(3)?);
                    out["syncTagEnabledAt"] = json!(
                        r.get::<_, Option<i64>>(5)?
                            .and_then(chatbotx_core::js::to_iso_string)
                    );
                }
                Ok(out)
            })
            .map_err(|e| BusinessError::Repo(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await
}

async fn threads_list(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "inboxId", "name", "threadsUserId", "username", "tokenRefreshError"
                   FROM "IntegrationThreads" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params![workspace_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "inboxId": r.get::<_, i64>(1)?.to_string(),
                    "name": r.get::<_, String>(2)?,
                    "threadsUserId": r.get::<_, String>(3)?,
                    "username": r.get::<_, String>(4)?,
                    "tokenRefreshError": r.get::<_, Option<String>>(5)?,
                }))
            })
            .map_err(|e| BusinessError::Repo(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await
}

/// `tiktokIntegrationService.disconnect` / `zaloIntegrationService.disconnect`
/// / `integrationThreadsService.disconnect`
/// (the Zalo OA API has no disconnect call): the `TagChannel` rows of a Zalo
/// OA, the satellite row, then `connectionStateService.disconnectInbox`, in
/// one transaction. A row of another workspace is not found.
pub async fn disconnect(db: &Db, channel: &str, workspace_id: i64, id: i64) -> Result<()> {
    let (table, label) = match channel {
        "tiktok" => ("IntegrationTiktok", "TikTok"),
        "zalo" => ("IntegrationZalo", "Zalo"),
        "threads" => ("IntegrationThreads", "Threads"),
        _ => return Err(BusinessError::not_found("Channel not found")),
    };
    tx::write(db, move |tx| {
        let inbox_id: Option<i64> = tx
            .query_row(
                &format!(
                    r#"SELECT "inboxId" FROM "{table}" WHERE "id" = ?1 AND "workspaceId" = ?2"#
                ),
                params![id, workspace_id],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                chatbotx_db::rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(BusinessError::Repo(e.into())),
            })?;
        let inbox_id = inbox_id
            .ok_or_else(|| BusinessError::not_found(format!("{label} channel not found")))?;
        if table == "IntegrationZalo" {
            // Polymorphic FK: no cascade for `TagChannel.integrationId`.
            tx.execute(
                r#"DELETE FROM "TagChannel" WHERE "channelType" = 'zalo' AND "integrationId" = ?1"#,
                params![id],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        }
        tx.execute(
            &format!(r#"DELETE FROM "{table}" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
            params![id, workspace_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        crate::connection::disconnect_inbox(tx, inbox_id, workspace_id, repo::now_ms())
    })
    .await
}

/// `AUTH_FAILED_PATTERNS` of the TikTok error mapper.
const TIKTOK_AUTH_FAILED: &[&str] = &[
    "access_token_invalid",
    "access_token_expired",
    "token_not_valid",
    "unauthorized",
    "invalid_token",
    "token revoked",
    "auth_failed",
];

/// `refreshTiktokTokenAction` (`refreshTiktokToken`): under the
/// `auth:refresh:tiktok:<id>` lock (10 s), `tt_user/oauth2/refresh_token`,
/// then `updateAuth` + `recordRefreshedAuth` and the audit. A failure stores
/// `tokenRefreshError` and marks the connection `needs_reauth` (revoked) or
/// `degraded` (`refresh_failed`), then answers "Failed to refresh TikTok token".
pub async fn refresh_tiktok_token(
    db: &Db,
    upstream: &impl Upstream,
    audit: &AuditService,
    base: &str,
    workspace_id: i64,
    id: i64,
) -> Result<()> {
    let _guard = crate::channel_token_refresh::refresh_locks()
        .acquire(
            &format!("auth:refresh:tiktok:{id}"),
            chatbotx_kv::LockOpts {
                ttl_secs: 10,
                retry_secs: None,
            },
        )
        .await
        .map_err(|e| system_error(e.to_string()))?;
    let mut auth = tiktok_auth(db, workspace_id, id).await?;
    let Some(refresh_token) = auth["tokens"]["refreshToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
    else {
        return Err(system_error("TikTok refresh token not available".into()));
    };
    let body = json!({
        "client_id": auth["clientId"],
        "client_secret": auth["clientSecret"],
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });
    let reply = upstream
        .send(UpstreamRequest {
            method: "POST",
            url: format!("{base}tt_user/oauth2/refresh_token/"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body.to_string()),
        })
        .await;
    // `(message, revoked)` of a failed call (`isRevokedTokenError`).
    let answer = match reply {
        Err(message) => Err((message, false)),
        Ok((status, text)) => {
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if status == 401 {
                Err(("Unauthorized".to_owned(), true))
            } else if !(200..300).contains(&status) {
                Err((
                    format!("tt_user/oauth2/refresh_token: HTTP {status}"),
                    false,
                ))
            } else if parsed["code"].as_i64() != Some(0) {
                let message = parsed["message"]
                    .as_str()
                    .unwrap_or("Token refresh failed")
                    .to_owned();
                let lower = message.to_lowercase();
                let revoked = TIKTOK_AUTH_FAILED.iter().any(|p| lower.contains(p));
                Err((message, revoked))
            } else {
                Ok(parsed["data"].clone())
            }
        }
    };
    let (open_id, inbox_id) = tiktok_identity(db, workspace_id, id).await?;
    let data = match answer {
        Ok(data) => data,
        Err((message, revoked)) => {
            tracing::error!(
                err = message,
                id,
                workspace_id,
                "Failed to refresh TikTok token"
            );
            mark_tiktok_refresh_error(db, workspace_id, id, &open_id, inbox_id, &message, revoked)
                .await?;
            return Err(system_error("Failed to refresh TikTok token".into()));
        }
    };
    let now = repo::now_ms();
    let at = |secs: &Value| {
        chatbotx_core::js::to_iso_string(now + secs.as_i64().unwrap_or(0) * 1000)
            .unwrap_or_default()
    };
    let expires_at = now + data["expires_in"].as_i64().unwrap_or(0) * 1000;
    if !auth["tokens"].is_object() {
        auth["tokens"] = json!({});
    }
    auth["tokens"]["accessToken"] = data["access_token"].clone();
    auth["tokens"]["refreshToken"] = data["refresh_token"].clone();
    auth["tokens"]["expiresAt"] = json!(at(&data["expires_in"]));
    auth["tokens"]["refreshTokenExpiresAt"] = json!(at(&data["refresh_token_expires_in"]));
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "IntegrationTiktok" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, auth.to_string(), now],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        // `recordRefreshedAuth` → `auth.saved` on an active connection.
        tx.execute(
            r#"UPDATE "Connection" SET "status" = 'connected', "statusReason" = NULL,
                   "authExpiresAt" = ?3, "lastError" = NULL, "updatedAt" = ?4
               WHERE "workspaceId" = ?1 AND "provider" = 'tiktok' AND "sourceId" = ?2
                 AND "status" IN ('connected', 'degraded')"#,
            params![workspace_id, open_id, expires_at, now],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await?;
    audit
        .audit("refresh", "refreshed the TikTok channel token")
        .await;
    Ok(())
}

async fn tiktok_identity(db: &Db, workspace_id: i64, id: i64) -> Result<(String, i64)> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "openId", "inboxId" FROM "IntegrationTiktok" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| BusinessError::not_found("TikTok integration not found"))
    })
    .await
}

/// `tiktokIntegrationService.markTokenRefreshError`.
async fn mark_tiktok_refresh_error(
    db: &Db,
    workspace_id: i64,
    id: i64,
    open_id: &str,
    inbox_id: i64,
    message: &str,
    revoked: bool,
) -> Result<()> {
    let (open, text) = (open_id.to_owned(), message.to_owned());
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "IntegrationTiktok" SET "tokenRefreshError" = ?3, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, text, repo::now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        if !revoked {
            // `markDegradedByIdentifier` (`refresh.transient_failure`).
            tx.execute(
                r#"UPDATE "Connection" SET "status" = 'degraded', "statusReason" = 'refresh_failed',
                       "updatedAt" = ?3
                   WHERE "workspaceId" = ?1 AND "provider" = 'tiktok' AND "sourceId" = ?2
                     AND "status" IN ('connected', 'degraded')"#,
                params![workspace_id, open, repo::now_ms()],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        }
        Ok(())
    })
    .await?;
    if revoked {
        crate::connection::mark_unhealthy_by_identifier(
            db,
            "tiktok",
            open_id,
            workspace_id,
            inbox_id,
        )
        .await?;
    }
    Ok(())
}
