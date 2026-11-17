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
