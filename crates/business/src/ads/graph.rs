//! Meta Marketing API reads of the ads reports (`integration-facebook-ads`
//! `getAdAccounts` / `getAdInsights` / `getCustomAudiences`), the two token
//! sources (`buildFacebookAdsContext`: the workspace-wide Facebook Ads
//! connection; `buildMessagingAdsContext`: a channel integration's
//! MessagingAdsConnection) and `resolveChannelAdAccountSources`.
//! ponytail: one in-process TTL cache stands in for the Redis `withCache` /
//! `getOrRevalidate` layers (one server process).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chatbotx_core::encryption::EncryptedData;
use chatbotx_db::Db;
use chatbotx_db::repo::messaging_ads_connection::{self as connections, IntegrationRef};
use chatbotx_db::rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use crate::errors::{BusinessError, Result};
use crate::meta_catalog::process_encryption;
use crate::tx;

const GRAPH: &str = "https://graph.facebook.com";
/// `DEFAULT_API_VERSION`.
const DEFAULT_VERSION: &str = "v23.0";
/// `ADS_PAGE_LIMIT` / `MAX_GRAPH_PAGES`.
const PAGE_LIMIT: &str = "499";
const MAX_PAGES: usize = 20;
/// `FB_LIST_CACHE_TTL_SECONDS` (also the messaging `staleAfter` of 60 s).
pub const LIST_TTL: Duration = Duration::from_secs(60);
/// `FB_INSIGHTS_CACHE_TTL_SECONDS`.
pub const INSIGHTS_TTL: Duration = Duration::from_secs(60 * 60);
const TOKEN_EXPIRED: i64 = 190;

/// A Graph token and API version.
#[derive(Debug, Clone)]
pub struct AdsAuth {
    pub access_token: String,
    pub version: String,
}

/// A failed Graph call (`FacebookAdsException`): the Graph error code, if any.
#[derive(Debug, Clone)]
pub struct GraphFailure {
    pub code: Option<i64>,
    pub message: String,
}

impl From<GraphFailure> for BusinessError {
    fn from(failure: GraphFailure) -> BusinessError {
        BusinessError::exception(failure.message, "facebookAdsError", 400)
    }
}

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Value)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The cached value of `key`, else `fetch()` (stored on success only).
pub async fn cached<F, Fut, E>(
    key: String,
    ttl: Duration,
    fetch: F,
) -> std::result::Result<Value, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<Value, E>>,
{
    if let Ok(cache) = CACHE.lock()
        && let Some((at, value)) = cache.get(&key)
        && at.elapsed() < ttl
    {
        return Ok(value.clone());
    }
    let value = fetch().await?;
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, (at, _)| at.elapsed() < INSIGHTS_TTL);
        cache.insert(key, (Instant::now(), value.clone()));
    }
    Ok(value)
}

/// Drops one cached key (a `forceRefresh` read).
pub(crate) fn forget(key: &str) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.remove(key);
    }
}

fn decrypt(raw: &Value) -> Option<AdsAuth> {
    let data: EncryptedData = serde_json::from_value(raw.clone()).ok()?;
    let auth: Value = process_encryption().decrypt_object(&data, None).ok()?;
    let token = auth["accessToken"].as_str()?.trim();
    (!token.is_empty()).then(|| AdsAuth {
        access_token: token.to_owned(),
        version: auth["version"]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_VERSION)
            .to_owned(),
    })
}

/// `buildFacebookAdsContext`: `None` without a workspace connection
/// (`findByWorkspaceIdOrFail` throws a plain `Error`).
pub async fn workspace_auth(db: &Db, workspace_id: i64) -> Result<Option<AdsAuth>> {
    let raw: Option<String> = tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                "SELECT \"auth\" FROM \"IntegrationFacebookAds\" WHERE \"workspaceId\" = ?1 LIMIT 1",
                [workspace_id],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    decrypt(&value)
        .map(Some)
        .ok_or_else(|| internal("Facebook Ads credentials could not be read"))
}

/// A plain thrown `Error` (500).
pub fn internal(message: &str) -> BusinessError {
    BusinessError::exception(message, "INTERNAL_SERVER_ERROR", 500)
}

/// `MessagingAdsReconnectRequiredException`.
pub fn reconnect_required() -> BusinessError {
    BusinessError::exception(
        "This connection needs to be reconnected.",
        "messagingAdsReconnectRequired",
        409,
    )
}

/// The integration FK of one channel.
pub fn integration_ref(workspace_id: i64, channel: &str, integration_id: i64) -> IntegrationRef {
    let mut out = IntegrationRef {
        workspace_id,
        ..IntegrationRef::default()
    };
    match channel {
        "whatsapp" => out.integration_whatsapp_id = Some(integration_id),
        "messenger" => out.integration_messenger_id = Some(integration_id),
        _ => out.integration_instagram_id = Some(integration_id),
    }
    out
}

pub(crate) async fn mark_invalid(db: &Db, reference: IntegrationRef) {
    let result = tx::write(db, move |tx| {
        connections::update_status(tx, reference, "invalid")?;
        Ok(())
    })
    .await;
    if let Err(error) = result {
        tracing::warn!(?error, "failed to mark a messaging-ads connection invalid");
    }
}

/// `buildMessagingAdsContext`: an inactive connection, or one whose auth does
/// not decrypt (then marked invalid), needs a reconnect.
pub async fn messaging_auth(db: &Db, reference: IntegrationRef) -> Result<AdsAuth> {
    let row = tx::read(db, move |conn| {
        Ok(connections::find_for_integration(conn, reference)?)
    })
    .await?;
    let Some(row) = row.filter(|r| r.status == "active") else {
        return Err(reconnect_required());
    };
    match decrypt(&row.auth.0) {
        Some(auth) => Ok(auth),
        None => {
            mark_invalid(db, reference).await;
            Err(reconnect_required())
        }
    }
}

fn graph_url(path: &str, query: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    format!("{GRAPH}/{path}?{query}")
}

/// One GET; a non-2xx answer is a [`GraphFailure`] with the Graph code.
pub async fn graph_get(
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Value, GraphFailure> {
    graph_call("GET", path, query).await
}

async fn graph_call(
    method: &'static str,
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Value, GraphFailure> {
    let reply = HttpUpstream
        .send(UpstreamRequest {
            method,
            url: graph_url(path, query),
            headers: Vec::new(),
            body: None,
        })
        .await;
    match reply {
        Ok((status, body)) if (200..300).contains(&status) => {
            Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
        }
        Ok((status, body)) => {
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            Err(GraphFailure {
                code: body["error"]["code"].as_i64(),
                message: chatbotx_core::utils::graph_error::format_graph_error(body.get("error"))
                    .unwrap_or_else(|| format!("Facebook Ads request failed with status {status}")),
            })
        }
        Err(message) => Err(GraphFailure {
            code: None,
            message,
        }),
    }
}

/// `fetchAllPages`: follows `paging.cursors.after` while `paging.next` is set.
async fn all_pages(
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Vec<Value>, GraphFailure> {
    let mut rows = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut q = query.to_vec();
        if let Some(after) = &after {
            q.push(("after", after.as_str()));
        }
        let page = graph_get(path, &q).await?;
        rows.extend(page["data"].as_array().cloned().unwrap_or_default());
        after = page["paging"]["next"]
            .as_str()
            .and(page["paging"]["cursors"]["after"].as_str())
            .map(str::to_owned);
        if after.is_none() {
            break;
        }
    }
    Ok(rows)
}

/// `getAdAccounts` (`{ id, name? }`).
pub async fn ad_accounts(auth: &AdsAuth) -> std::result::Result<Value, GraphFailure> {
    let rows = all_pages(
        &format!("{}/me/adaccounts", auth.version),
        &[
            ("fields", "id,name"),
            ("limit", PAGE_LIMIT),
            ("access_token", &auth.access_token),
        ],
    )
    .await?;
    Ok(Value::Array(
        rows.iter()
            .filter_map(|row| {
                let mut account = json!({ "id": row["id"].as_str()? });
                if let Some(name) = row["name"].as_str() {
                    account["name"] = json!(name);
                }
                Some(account)
            })
            .collect(),
    ))
}

/// `getCustomAudiences` (`{ id, name?, subtype? }`).
pub async fn custom_audiences(
    auth: &AdsAuth,
    ad_account_id: &str,
) -> std::result::Result<Value, GraphFailure> {
    let rows = all_pages(
        &format!("{}/{ad_account_id}/customaudiences", auth.version),
        &[
            ("fields", "id,name,subtype"),
            ("limit", PAGE_LIMIT),
            ("access_token", &auth.access_token),
        ],
    )
    .await?;
    Ok(Value::Array(
        rows.iter()
            .filter_map(|row| {
                let mut audience = json!({ "id": row["id"].as_str()? });
                for key in ["name", "subtype"] {
                    if let Some(text) = row[key].as_str() {
                        audience[key] = json!(text);
                    }
                }
                Some(audience)
            })
            .collect(),
    ))
}

/// `numericInsightField`: absent / empty / not finite = 0.
fn numeric(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or_default(),
        Value::String(s) if !s.is_empty() => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .unwrap_or_default(),
        _ => 0.0,
    }
}

/// `getAdInsights` (`level=ad`; `daily` adds `time_increment=1` and
/// `date_start`). Rows without an `ad_id` are dropped.
pub async fn insights(
    auth: &AdsAuth,
    ad_account_id: &str,
    since: &str,
    until: &str,
    daily: bool,
) -> std::result::Result<Value, GraphFailure> {
    let fields = if daily {
        "ad_id,ad_name,account_currency,spend,impressions,clicks,date_start"
    } else {
        "ad_id,ad_name,account_currency,spend,impressions,clicks"
    };
    let range = json!({ "since": since, "until": until }).to_string();
    let mut query = vec![
        ("level", "ad"),
        ("fields", fields),
        ("time_range", range.as_str()),
        ("limit", PAGE_LIMIT),
        ("access_token", auth.access_token.as_str()),
    ];
    if daily {
        query.push(("time_increment", "1"));
    }
    let rows = all_pages(
        &format!("{}/{ad_account_id}/insights", auth.version),
        &query,
    )
    .await?;
    Ok(Value::Array(
        rows.iter()
            .filter_map(|row| {
                let ad_id = row["ad_id"].as_str()?.trim();
                if ad_id.is_empty() {
                    return None;
                }
                Some(json!({
                    "ad_id": ad_id,
                    "ad_name": row["ad_name"],
                    "account_currency": row["account_currency"],
                    "spend": numeric(&row["spend"]),
                    "impressions": numeric(&row["impressions"]),
                    "clicks": numeric(&row["clicks"]),
                    "date_start": row["date_start"],
                }))
            })
            .collect(),
    ))
}

/// Where an ad account was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Messaging(i64),
    Workspace,
}

/// A `ChannelAdAccount`: the account JSON and its sources.
#[derive(Debug, Clone)]
pub struct ChannelAdAccount {
    pub account: Value,
    pub sources: Vec<Source>,
}

/// `listCachedMessagingAdAccounts` (a 190 marks the connection invalid);
/// failures are logged and give no accounts.
async fn messaging_accounts(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    integration_id: i64,
) -> Vec<Value> {
    let reference = integration_ref(workspace_id, channel, integration_id);
    let key = format!("msgads:ad-accounts:{workspace_id}:{channel}:{integration_id}");
    let result = cached(key, LIST_TTL, || async {
        let auth = messaging_auth(db, reference).await?;
        match ad_accounts(&auth).await {
            Ok(accounts) => Ok(accounts),
            Err(failure) => {
                if failure.code == Some(TOKEN_EXPIRED) {
                    mark_invalid(db, reference).await;
                }
                Err(BusinessError::from(failure))
            }
        }
    })
    .await;
    match result {
        Ok(accounts) => accounts.as_array().cloned().unwrap_or_default(),
        Err(error) => {
            tracing::warn!(
                ?error,
                workspace_id,
                channel,
                integration_id,
                "failed to load messaging-ads ad accounts"
            );
            Vec::new()
        }
    }
}

/// `getCachedAdAccounts` of the workspace connection (none without one).
async fn workspace_accounts(db: &Db, workspace_id: i64) -> Vec<Value> {
    let result = cached(
        format!("fb-ads:ad-accounts:{workspace_id}"),
        LIST_TTL,
        || async {
            let Some(auth) = workspace_auth(db, workspace_id).await? else {
                return Ok(Value::Array(Vec::new()));
            };
            ad_accounts(&auth).await.map_err(BusinessError::from)
        },
    )
    .await;
    match result {
        Ok(accounts) => accounts.as_array().cloned().unwrap_or_default(),
        Err(error) => {
            tracing::warn!(
                ?error,
                workspace_id,
                "failed to load the workspace-wide ad accounts"
            );
            Vec::new()
        }
    }
}

fn integration_of(row: &chatbotx_db::repo::models::MessagingAdsConnection) -> Option<i64> {
    row.integration_whatsapp_id
        .or(row.integration_messenger_id)
        .or(row.integration_instagram_id)
}

fn push_deduped(out: &mut Vec<ChannelAdAccount>, accounts: Vec<Value>, source: Source) {
    for account in accounts {
        match out.iter_mut().find(|a| a.account["id"] == account["id"]) {
            Some(existing) => existing.sources.push(source.clone()),
            None => out.push(ChannelAdAccount {
                account,
                sources: vec![source.clone()],
            }),
        }
    }
}

/// `resolveChannelAdAccountSources`: one integration's own connection, or
/// every connection of the channel plus the workspace-wide one, deduped by id.
pub async fn channel_ad_accounts(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    integration_id: Option<i64>,
) -> Result<Vec<ChannelAdAccount>> {
    let owned = channel.to_owned();
    let rows = tx::read(db, move |conn| {
        Ok(connections::list_for_channel(conn, workspace_id, &owned)?)
    })
    .await?;
    let mut out = Vec::new();
    if let Some(integration_id) = integration_id {
        if !rows
            .iter()
            .any(|r| integration_of(r) == Some(integration_id))
        {
            return Ok(out);
        }
        let accounts = messaging_accounts(db, workspace_id, channel, integration_id).await;
        push_deduped(&mut out, accounts, Source::Messaging(integration_id));
        return Ok(out);
    }
    for row in &rows {
        if let Some(id) = integration_of(row) {
            let accounts = messaging_accounts(db, workspace_id, channel, id).await;
            push_deduped(&mut out, accounts, Source::Messaging(id));
        }
    }
    let workspace = workspace_accounts(db, workspace_id).await;
    push_deduped(&mut out, workspace, Source::Workspace);
    Ok(out)
}

/// `getCachedCustomAudiences` (500 without a workspace connection).
pub async fn cached_custom_audiences(
    db: &Db,
    workspace_id: i64,
    ad_account_id: String,
) -> Result<Value> {
    let key = format!("fb-ads:custom-audiences:{workspace_id}:{ad_account_id}");
    cached(key, LIST_TTL, || async {
        let auth = workspace_auth(db, workspace_id)
            .await?
            .ok_or_else(|| internal("Facebook Ads integration not found"))?;
        custom_audiences(&auth, &ad_account_id)
            .await
            .map_err(BusinessError::from)
    })
    .await
}

/// The token of an account's first source (`buildContextResolverBySource`).
pub async fn source_auth(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    source: Option<&Source>,
) -> Result<AdsAuth> {
    match source {
        Some(Source::Messaging(id)) => {
            messaging_auth(db, integration_ref(workspace_id, channel, *id)).await
        }
        _ => workspace_auth(db, workspace_id)
            .await?
            .ok_or_else(|| internal("Facebook Ads integration not found")),
    }
}

/// `invalidateMessagingAdsCache`: drops the cached Graph reads of one
/// integration.
pub(crate) fn invalidate(workspace_id: i64, channel: &str, integration_id: i64) {
    let suffix = format!(":{workspace_id}:{channel}:{integration_id}");
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|key, _| !(key.starts_with("msgads:") && key.ends_with(&suffix)));
    }
}

/// `checkCampaignPrerequisites`.
pub async fn prerequisites(db: &Db, reference: IntegrationRef) -> Result<Value> {
    let row = tx::read(db, move |conn| {
        Ok(connections::find_for_integration(conn, reference)?)
    })
    .await?;
    let connected = row.as_ref().is_some_and(|r| r.status == "active");
    Ok(json!({ "connected": connected, "reconnectNeeded": row.is_some() && !connected }))
}

/// `listForChannel` (active only) as `{ data }`.
pub async fn list_connections(db: &Db, workspace_id: i64, channel: String) -> Result<Value> {
    let rows = tx::read(db, move |conn| {
        Ok(connections::list_for_channel(conn, workspace_id, &channel)?)
    })
    .await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "channel": r.channel,
                "integrationId": integration_of(r).map(|id| id.to_string()).unwrap_or_default(),
                "status": r.status,
                "createdAt": chatbotx_db::repo::iso(r.created_at),
                "updatedAt": chatbotx_db::repo::iso(r.updated_at),
            })
        })
        .collect();
    Ok(json!({ "data": data }))
}

/// `revokeAndDisconnect`: a best-effort `DELETE me/permissions` (skipped when
/// the auth does not decrypt), then the row is always removed.
pub async fn disconnect(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    integration_id: i64,
) -> Result<()> {
    let reference = integration_ref(workspace_id, channel, integration_id);
    let row = tx::read(db, move |conn| {
        Ok(connections::find_for_integration(conn, reference)?)
    })
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    if let Some(auth) = decrypt(&row.auth.0) {
        let path = format!("{}/me/permissions", auth.version);
        if let Err(failure) =
            graph_call("DELETE", &path, &[("access_token", &auth.access_token)]).await
        {
            tracing::warn!(message = %failure.message, "messaging-ads token revoke failed");
        }
    }
    let id = row.id;
    tx::write(db, move |tx| Ok(connections::remove(tx, id, workspace_id)?)).await?;
    invalidate(workspace_id, channel, integration_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_numeric_insight_fields() {
        assert_eq!(numeric(&json!("20")), 20.0);
        assert_eq!(numeric(&json!(1.5)), 1.5);
        assert_eq!(numeric(&json!("")), 0.0);
        assert_eq!(numeric(&json!("abc")), 0.0);
        assert_eq!(numeric(&Value::Null), 0.0);
    }

    #[tokio::test]
    async fn caches_successes_only() {
        let key = "test:cache".to_owned();
        let first: std::result::Result<Value, ()> =
            cached(key.clone(), LIST_TTL, || async { Err(()) }).await;
        assert!(first.is_err());
        let second: std::result::Result<Value, ()> =
            cached(key.clone(), LIST_TTL, || async { Ok(json!(1)) }).await;
        assert_eq!(second, Ok(json!(1)));
        let third: std::result::Result<Value, ()> =
            cached(key, LIST_TTL, || async { Ok(json!(2)) }).await;
        assert_eq!(third, Ok(json!(1)));
    }
}
