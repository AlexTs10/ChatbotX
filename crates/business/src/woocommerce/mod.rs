//! `packages/business/src/woocommerce/{auth,errors,integration.service}.ts`:
//! a workspace's WooCommerce store as its catalog and order source. The
//! REST API keys are kept encrypted (AAD `woocommerce:<ws>`); [`sync`] is
//! the worker side.

pub mod client;
pub mod mappers;
pub mod sync;

use chatbotx_core::encryption::EncryptedData;
use chatbotx_db::repo::models::IntegrationWoocommerce;
use chatbotx_db::repo::{iso, store_integration};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::{JobOptions, QueueName};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::client::{ApiError, AuthMode, Credentials};
use crate::errors::{BusinessError, Result};
use crate::external_store;
use crate::net::ssrf_guard::{HttpDoh, is_ssrf_unsafe_url_with};
use crate::shopify::Ctx;
use crate::tx;

pub const SOURCE: &str = "woocommerce";
/// `IntegrationJobAction.woocommerceSync`.
pub const SYNC_JOB: &str = "woocommerceSync";

/// The webhook topics registered on connect.
pub const WEBHOOK_TOPICS: &[&str] = &[
    "product.created",
    "product.updated",
    "product.deleted",
    "product.restored",
    "order.created",
    "order.updated",
    "order.deleted",
    "order.restored",
    "coupon.created",
    "coupon.updated",
    "coupon.deleted",
    "coupon.restored",
];

const STALE_SYNC_MS: i64 = 30 * 60 * 1000;
const CONSUMER_KEY_HINT_LENGTH: usize = 7;

// ---- errors.ts ---------------------------------------------------------------

pub fn not_connected() -> BusinessError {
    BusinessError::exception(
        "Connect a WooCommerce store in Ecommerce → Settings → WooCommerce first.",
        "woocommerceNotConnected",
        400,
    )
}

pub fn credentials_invalid(detail: Option<&str>) -> BusinessError {
    let suffix = detail.map_or_else(|| ".".to_string(), |d| format!(": {d}"));
    BusinessError::Exception {
        message: format!(
            "WooCommerce did not accept this connection{suffix} Check the store address and the REST API keys (Read/Write permission)."
        ),
        code: "woocommerceCredentialsInvalid".into(),
        status: 400,
        field: None,
        data: Some(json!({ "detail": detail.unwrap_or_default() })),
    }
}

pub fn sync_in_progress() -> BusinessError {
    BusinessError::exception(
        "A WooCommerce sync is already running. Wait for it to finish.",
        "woocommerceSyncInProgress",
        409,
    )
}

/// `toWooCommerceConnectException`.
fn connect_error(error: &ApiError) -> BusinessError {
    if error.status < 500 {
        return credentials_invalid(Some(&error.message));
    }
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.message.clone()))
}

// ---- auth.ts -----------------------------------------------------------------

/// `woocommerceAuthSecretSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSecret {
    pub consumer_key: String,
    pub consumer_secret: String,
    pub webhook_secret: String,
    /// `header` | `query`.
    pub auth_mode: String,
}

fn aad(workspace_id: i64) -> String {
    format!("woocommerce:{workspace_id}")
}

fn crypto_error(e: impl std::fmt::Display) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string()))
}

fn encrypt_auth(workspace_id: i64, secret: &AuthSecret) -> Result<Value> {
    let data = crate::meta_catalog::process_encryption()
        .encrypt_object(secret, Some(&aad(workspace_id)))
        .map_err(crypto_error)?;
    serde_json::to_value(data).map_err(crypto_error)
}

pub fn decrypt_auth(row: &IntegrationWoocommerce) -> Result<AuthSecret> {
    let data: EncryptedData =
        serde_json::from_value(row.encrypted_auth.0.clone()).map_err(crypto_error)?;
    crate::meta_catalog::process_encryption()
        .decrypt_object(&data, Some(&aad(row.workspace_id)))
        .map_err(crypto_error)
}

/// `toWooCommerceCredentials`.
pub fn credentials_of(site_url: &str, secret: &AuthSecret) -> Credentials {
    Credentials {
        site_url: site_url.to_string(),
        consumer_key: secret.consumer_key.clone(),
        consumer_secret: secret.consumer_secret.clone(),
        auth_mode: if secret.auth_mode == "query" {
            AuthMode::Query
        } else {
            AuthMode::Header
        },
    }
}

/// `resolveWooCommerceCredentials`. Every client call checks the address
/// with the SSRF guard again (`client::request`).
pub fn resolve_credentials(row: &IntegrationWoocommerce) -> Result<Credentials> {
    Ok(credentials_of(&row.site_url, &decrypt_auth(row)?))
}

// ---- integration.service.ts --------------------------------------------------

/// `woocommerceSiteUrlSchema`: `https://` origin + path, no trailing slash.
pub fn parse_site_url(input: &str) -> std::result::Result<String, String> {
    let trimmed = input.trim();
    let has_scheme = trimmed.split_once("://").is_some_and(|(scheme, _)| {
        scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    });
    let with_scheme = if has_scheme {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let invalid = || "Enter the store's https:// address, e.g. https://example.com.".to_string();
    let url = url::Url::parse(&with_scheme).map_err(|_| invalid())?;
    if url.scheme() != "https" || url.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    let origin = url.origin().ascii_serialization();
    Ok(format!("{origin}{}", url.path())
        .trim_end_matches('/')
        .to_string())
}

/// `woocommerceWebhookPath`.
pub fn webhook_path(workspace_id: i64) -> String {
    format!("/api/woocommerce/{workspace_id}/webhook")
}

fn webhook_url(app_url: &str, workspace_id: i64) -> String {
    chatbotx_core::utils::web_url::join_app_url(app_url, &webhook_path(workspace_id))
}

/// `toView`: never a secret, only the consumer key's first characters.
pub fn view(row: &IntegrationWoocommerce, webhook_url: &str) -> Result<Value> {
    let secret = decrypt_auth(row)?;
    let hint: String = secret
        .consumer_key
        .chars()
        .take(CONSUMER_KEY_HINT_LENGTH)
        .collect();
    Ok(json!({
        "connected": true,
        "siteUrl": row.site_url,
        "storeName": row.store_name,
        "currency": row.currency,
        "consumerKeyHint": format!("{hint}…"),
        "status": row.status,
        "statusReason": row.status_reason,
        "syncStatus": row.sync_status,
        "syncError": row.sync_error,
        "lastSyncedAt": row.last_synced_at.map(iso),
        "productCount": row.product_count,
        "orderCount": row.order_count,
        "webhookUrl": webhook_url,
        "adminUrl": client::admin_url(&row.site_url),
        "connectedAt": iso(row.created_at),
    }))
}

pub(crate) async fn find(
    db: &chatbotx_db::Db,
    workspace_id: i64,
) -> Result<Option<IntegrationWoocommerce>> {
    tx::read(db, move |conn| {
        Ok(store_integration::find_woocommerce(conn, workspace_id)?)
    })
    .await
}

/// `getStatus`.
pub async fn get_status(db: &chatbotx_db::Db, app_url: &str, workspace_id: i64) -> Result<Value> {
    let url = webhook_url(app_url, workspace_id);
    match find(db, workspace_id).await? {
        Some(row) => view(&row, &url),
        None => Ok(json!({ "connected": false, "webhookUrl": url })),
    }
}

/// `probeStore`: Basic auth first, then the keys in the query string.
async fn probe_store(
    site_url: &str,
    key: &str,
    secret: &str,
) -> std::result::Result<(&'static str, client::StoreInfo), ApiError> {
    let mut credentials = Credentials {
        site_url: site_url.to_string(),
        consumer_key: key.to_string(),
        consumer_secret: secret.to_string(),
        auth_mode: AuthMode::Header,
    };
    match client::get_store_info(&credentials).await {
        Ok(info) => return Ok(("header", info)),
        Err(e) if !e.unauthorized() => return Err(e),
        Err(_) => {}
    }
    credentials.auth_mode = AuthMode::Query;
    Ok(("query", client::get_store_info(&credentials).await?))
}

fn generate_webhook_secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| crypto_error("could not generate a webhook secret"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn remove_webhooks_quietly(credentials: &Credentials, ids: &[String]) {
    for id in ids {
        if let Err(error) = client::delete_webhook(credentials, id).await {
            tracing::warn!(%error, webhook_id = %id, "[woocommerce] could not remove a webhook");
        }
    }
}

async fn remove_previous_webhooks_quietly(row: &IntegrationWoocommerce) {
    match resolve_credentials(row) {
        Ok(credentials) => {
            let ids: Vec<String> =
                serde_json::from_value(row.webhook_ids.0.clone()).unwrap_or_default();
            remove_webhooks_quietly(&credentials, &ids).await;
        }
        Err(error) => tracing::warn!(
            %error,
            workspace_id = row.workspace_id,
            "[woocommerce] could not reach the store to remove its webhooks"
        ),
    }
}

/// `connect`: checks the keys, registers our webhooks (signed with a fresh
/// secret), saves the connection encrypted and queues a full sync.
pub async fn connect(
    ctx: Ctx<'_>,
    workspace_id: i64,
    site_url: &str,
    consumer_key: &str,
    consumer_secret: &str,
) -> Result<Value> {
    let site_url = parse_site_url(site_url).map_err(|m| credentials_invalid(Some(&m)))?;
    // The keys and every later call go to this address from our servers.
    if is_ssrf_unsafe_url_with(&site_url, &HttpDoh).await {
        return Err(credentials_invalid(Some(
            "the store address must be a public website",
        )));
    }
    let (key, secret_key) = (consumer_key.trim(), consumer_secret.trim());
    let (auth_mode, info) = probe_store(&site_url, key, secret_key)
        .await
        .map_err(|e| connect_error(&e))?;
    let secret = AuthSecret {
        consumer_key: key.to_string(),
        consumer_secret: secret_key.to_string(),
        webhook_secret: generate_webhook_secret()?,
        auth_mode: auth_mode.to_string(),
    };
    let credentials = credentials_of(&site_url, &secret);
    let previous = find(ctx.db, workspace_id).await?;
    let url = webhook_url(ctx.app_url, workspace_id);
    // Ours already on the store go only once the new ones are saved.
    let stale: Vec<String> = client::list_webhooks(&credentials, &url)
        .await
        .map_err(|e| connect_error(&e))?
        .iter()
        .filter_map(|w| w["id"].as_i64().map(|id| id.to_string()))
        .collect();
    let mut ids = Vec::new();
    for topic in WEBHOOK_TOPICS {
        match client::create_webhook(&credentials, topic, &url, &secret.webhook_secret).await {
            Ok(id) => ids.push(id),
            Err(error) => {
                remove_webhooks_quietly(&credentials, &ids).await;
                return Err(connect_error(&error));
            }
        }
    }
    let encrypted = encrypt_auth(workspace_id, &secret)?;
    let values: Vec<(&'static str, Sql)> = vec![
        ("siteUrl", Sql::Text(site_url.clone())),
        ("storeName", external_store::text(info.name.as_deref())),
        ("currency", Sql::Text(info.currency.clone())),
        ("encryptedAuth", Sql::Text(encrypted.to_string())),
        ("webhookIds", Sql::Text(json!(ids).to_string())),
        ("status", Sql::Text("active".into())),
        ("statusReason", Sql::Null),
        ("syncStatus", Sql::Text("idle".into())),
        ("syncError", Sql::Null),
        ("productCount", Sql::Integer(0)),
        ("orderCount", Sql::Integer(0)),
    ];
    let saved = tx::write(ctx.db, move |tx| {
        let conn: &Connection = tx;
        Ok(store_integration::upsert_woocommerce(
            conn,
            workspace_id,
            &values,
        )?)
    })
    .await;
    if let Err(error) = saved {
        remove_webhooks_quietly(&credentials, &ids).await;
        return Err(error);
    }
    remove_webhooks_quietly(&credentials, &stale).await;
    if let Some(previous) = previous.filter(|p| p.site_url != site_url) {
        remove_previous_webhooks_quietly(&previous).await;
    }
    ctx.audit
        .audit(
            "connect",
            &format!("connected WooCommerce store {site_url}"),
        )
        .await;
    request_sync(ctx, workspace_id, "all").await
}

async fn reactivate_webhooks_quietly(app_url: &str, row: &IntegrationWoocommerce) {
    let result: std::result::Result<(), String> = async {
        let credentials = resolve_credentials(row).map_err(|e| e.to_string())?;
        let ours = client::list_webhooks(&credentials, &webhook_url(app_url, row.workspace_id))
            .await
            .map_err(|e| e.to_string())?;
        for webhook in ours.iter().filter(|w| w["status"] != "active") {
            if let Some(id) = webhook["id"].as_i64() {
                client::activate_webhook(&credentials, &id.to_string())
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, workspace_id = row.workspace_id, "[woocommerce] could not turn the store's webhooks back on");
    }
}

/// `requestSync`: queues a sync unless a fresh one runs; turns our webhooks
/// back on (WooCommerce disables one after five failed deliveries).
pub async fn request_sync(ctx: Ctx<'_>, workspace_id: i64, scope: &str) -> Result<Value> {
    let row = find(ctx.db, workspace_id)
        .await?
        .ok_or_else(not_connected)?;
    let stale = chatbotx_db::repo::now_ms() - row.updated_at > STALE_SYNC_MS;
    if matches!(row.sync_status.as_str(), "queued" | "running") && !stale {
        return Err(sync_in_progress());
    }
    reactivate_webhooks_quietly(ctx.app_url, &row).await;
    let set = |status: &str, error: Option<&str>| {
        vec![
            ("syncStatus", Sql::Text(status.to_string())),
            ("syncError", external_store::text(error)),
        ]
    };
    external_store::update_connection(ctx.db, SOURCE, workspace_id, set("queued", None)).await?;
    let job_id = format!(
        "woocommerce-sync-{workspace_id}-{scope}-{}",
        chatbotx_db::repo::now_ms()
    );
    let queued = match ctx.jobs {
        Some(jobs) => jobs
            .add(
                QueueName::Integration,
                SYNC_JOB,
                json!({ "type": SYNC_JOB, "data": { "workspaceId": workspace_id.to_string(), "scope": scope } }),
                JobOptions {
                    job_id: Some(job_id),
                    ..JobOptions::default()
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        None => Err("the job queue is off".to_string()),
    };
    if let Err(error) = queued {
        external_store::update_connection(
            ctx.db,
            SOURCE,
            workspace_id,
            set("failed", Some("Could not queue the sync.")),
        )
        .await?;
        return Err(crypto_error(error));
    }
    let updated = find(ctx.db, workspace_id)
        .await?
        .ok_or_else(not_connected)?;
    view(&updated, &webhook_url(ctx.app_url, workspace_id))
}

/// `disconnect`.
pub async fn disconnect(ctx: Ctx<'_>, workspace_id: i64) -> Result<()> {
    let Some(row) = find(ctx.db, workspace_id).await? else {
        return Ok(());
    };
    remove_previous_webhooks_quietly(&row).await;
    tx::write(ctx.db, move |tx| {
        let conn: &Connection = tx;
        Ok(store_integration::delete_woocommerce(conn, workspace_id)?)
    })
    .await?;
    external_store::switch_store_back_from(ctx.db, workspace_id, SOURCE).await?;
    ctx.audit
        .audit(
            "disconnect",
            &format!("disconnected WooCommerce store {}", row.site_url),
        )
        .await;
    Ok(())
}

/// `markInvalid`.
pub async fn mark_invalid(db: &chatbotx_db::Db, workspace_id: i64, reason: &str) -> Result<()> {
    external_store::update_connection(
        db,
        SOURCE,
        workspace_id,
        vec![
            ("status", Sql::Text("invalid".into())),
            ("statusReason", Sql::Text(reason.to_string())),
        ],
    )
    .await
}

/// `refreshStoreCurrency`: the store's currency as WooCommerce shows prices
/// becomes ours while the store sells from WooCommerce.
pub async fn refresh_store_currency(
    db: &chatbotx_db::Db,
    workspace_id: i64,
) -> std::result::Result<(), sync::SyncError> {
    let Some(row) = find(db, workspace_id)
        .await?
        .filter(|r| r.status == "active")
    else {
        return Ok(());
    };
    let info = client::get_store_info(&resolve_credentials(&row)?).await?;
    if info.currency != row.currency || info.name != row.store_name {
        external_store::update_connection(
            db,
            SOURCE,
            workspace_id,
            vec![
                ("currency", Sql::Text(info.currency.clone())),
                ("storeName", external_store::text(info.name.as_deref())),
            ],
        )
        .await?;
    }
    external_store::adopt_external_store_currency(
        db,
        workspace_id,
        SOURCE,
        &info.currency,
        Some(info.currency_symbol),
        info.currency_position,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_urls() {
        assert_eq!(
            parse_site_url(" Example.com/shop/ ").unwrap(),
            "https://example.com/shop"
        );
        assert_eq!(
            parse_site_url("https://example.com").unwrap(),
            "https://example.com"
        );
        assert!(parse_site_url("http://example.com").is_err());
        assert!(parse_site_url("ftp://example.com").is_err());
    }
}
