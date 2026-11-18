//! `packages/business/src/shopify/{auth,errors,integration.service}.ts`: a
//! workspace's Shopify store as its catalog and order source. The secrets
//! are kept encrypted (AAD `shopify:<ws>`); [`sync`] is the worker side.

pub mod client;
pub mod mappers;
pub mod sync;

use std::sync::LazyLock;

use chatbotx_core::encryption::EncryptedData;
use chatbotx_db::repo::models::IntegrationShopify;
use chatbotx_db::repo::{iso, store_integration};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use chatbotx_kv::lock::{LockMap, LockOpts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::client::{ApiError, Credentials};
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::external_store;
use crate::tx;

pub const SOURCE: &str = "shopify";

/// The webhook topics registered on connect (GraphQL enum spelling).
pub const WEBHOOK_TOPICS: &[&str] = &[
    "PRODUCTS_CREATE",
    "PRODUCTS_UPDATE",
    "PRODUCTS_DELETE",
    "INVENTORY_LEVELS_UPDATE",
    "ORDERS_CREATE",
    "ORDERS_UPDATED",
    "ORDERS_PAID",
    "ORDERS_FULFILLED",
    "ORDERS_CANCELLED",
    "FULFILLMENTS_UPDATE",
    "DISCOUNTS_CREATE",
    "DISCOUNTS_UPDATE",
    "DISCOUNTS_DELETE",
    "APP_UNINSTALLED",
];

/// `IntegrationJobAction.shopifySync`.
pub const SYNC_JOB: &str = "shopifySync";

/// A token this close to expiring is renewed before use.
const TOKEN_RENEW_MARGIN_MS: i64 = 5 * 60 * 1000;
/// A queued or running sync older than this is treated as dead.
const STALE_SYNC_MS: i64 = 30 * 60 * 1000;

// ---- errors.ts ---------------------------------------------------------------

pub fn not_connected() -> BusinessError {
    BusinessError::exception(
        "Connect a Shopify store in Ecommerce → Settings → Shopify first.",
        "shopifyNotConnected",
        400,
    )
}

pub fn credentials_invalid(detail: Option<&str>) -> BusinessError {
    let suffix = detail.map_or_else(|| ".".to_string(), |d| format!(": {d}"));
    BusinessError::Exception {
        message: format!(
            "Shopify did not accept these credentials{suffix} Check the store domain and the app's token or keys, and that the app is installed with the required scopes."
        ),
        code: "shopifyCredentialsInvalid".into(),
        status: 400,
        field: None,
        data: Some(json!({ "detail": detail.unwrap_or_default() })),
    }
}

pub fn sync_in_progress() -> BusinessError {
    BusinessError::exception(
        "A Shopify sync is already running. Wait for it to finish.",
        "shopifySyncInProgress",
        409,
    )
}

/// `toShopifyConnectException`.
fn connect_error(error: &ApiError) -> BusinessError {
    if error.unauthorized() || error.status == 403 || error.user_error {
        return credentials_invalid(Some(&error.message));
    }
    if error.status == 404 {
        return credentials_invalid(Some("the store was not found"));
    }
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.message.clone()))
}

// ---- auth.ts -----------------------------------------------------------------

/// `shopifyAuthSecretSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthSecret {
    Token {
        access_token: String,
        api_secret: String,
    },
    ClientCredentials {
        client_id: String,
        client_secret: String,
        access_token: Option<String>,
        /// ISO time the cached access token stops working.
        access_token_expires_at: Option<String>,
    },
}

impl AuthSecret {
    fn kind(&self) -> &'static str {
        match self {
            AuthSecret::Token { .. } => "token",
            AuthSecret::ClientCredentials { .. } => "clientCredentials",
        }
    }

    /// `webhookSecretOf`.
    pub fn webhook_secret(&self) -> &str {
        match self {
            AuthSecret::Token { api_secret, .. } => api_secret,
            AuthSecret::ClientCredentials { client_secret, .. } => client_secret,
        }
    }

    /// The access token when it still works.
    fn usable_token(&self) -> Option<&str> {
        match self {
            AuthSecret::Token { access_token, .. } => Some(access_token),
            AuthSecret::ClientCredentials {
                access_token: Some(token),
                access_token_expires_at: Some(at),
                ..
            } => chrono::DateTime::parse_from_rfc3339(at)
                .ok()
                .filter(|at| {
                    at.timestamp_millis() - chatbotx_db::repo::now_ms() > TOKEN_RENEW_MARGIN_MS
                })
                .map(|_| token.as_str()),
            AuthSecret::ClientCredentials { .. } => None,
        }
    }
}

fn aad(workspace_id: i64) -> String {
    format!("shopify:{workspace_id}")
}

fn crypto_error(e: impl std::fmt::Display) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string()))
}

pub fn encrypt_auth(workspace_id: i64, secret: &AuthSecret) -> Result<Value> {
    let data = crate::meta_catalog::process_encryption()
        .encrypt_object(secret, Some(&aad(workspace_id)))
        .map_err(crypto_error)?;
    serde_json::to_value(data).map_err(crypto_error)
}

pub fn decrypt_auth(row: &IntegrationShopify) -> Result<AuthSecret> {
    let data: EncryptedData =
        serde_json::from_value(row.encrypted_auth.0.clone()).map_err(crypto_error)?;
    crate::meta_catalog::process_encryption()
        .decrypt_object(&data, Some(&aad(row.workspace_id)))
        .map_err(crypto_error)
}

/// `issueClientCredentialsToken`.
async fn issue_client_credentials_token(
    shop_domain: &str,
    client_id: &str,
    client_secret: &str,
) -> std::result::Result<AuthSecret, ApiError> {
    let (token, expires_in) =
        client::request_access_token(shop_domain, client_id, client_secret).await?;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in);
    Ok(AuthSecret::ClientCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        access_token: Some(token),
        access_token_expires_at: Some(
            expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    })
}

static TOKEN_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);

/// `resolveShopifyCredentials`: a working access token; a 24-hour token is
/// renewed under a lock so two jobs do not both ask Shopify.
pub async fn resolve_credentials(
    db: &chatbotx_db::Db,
    row: &IntegrationShopify,
) -> std::result::Result<Credentials, sync::SyncError> {
    let secret = decrypt_auth(row)?;
    if let Some(token) = secret.usable_token() {
        return Ok(Credentials {
            shop_domain: row.shop_domain.clone(),
            access_token: token.to_string(),
        });
    }
    // ponytail: an in-process lock (one worker process); the original's is Redis.
    let _guard = TOKEN_LOCKS
        .acquire(
            &format!("shopify-token:{}", row.workspace_id),
            LockOpts {
                ttl_secs: 30,
                retry_secs: Some(30),
            },
        )
        .await
        .map_err(|e| crypto_error(e.to_string()))?;
    let ws = row.workspace_id;
    let latest = tx::read(db, move |conn| {
        Ok(store_integration::find_shopify(conn, ws)?)
    })
    .await?
    .ok_or_else(not_connected)?;
    let current = decrypt_auth(&latest)?;
    if let Some(token) = current.usable_token() {
        return Ok(Credentials {
            shop_domain: latest.shop_domain.clone(),
            access_token: token.to_string(),
        });
    }
    let AuthSecret::ClientCredentials {
        client_id,
        client_secret,
        ..
    } = &current
    else {
        return Err(not_connected().into());
    };
    let renewed =
        issue_client_credentials_token(&latest.shop_domain, client_id, client_secret).await?;
    let encrypted = encrypt_auth(ws, &renewed)?;
    external_store::update_connection(
        db,
        SOURCE,
        ws,
        vec![("encryptedAuth", Sql::Text(encrypted.to_string()))],
    )
    .await?;
    Ok(Credentials {
        shop_domain: latest.shop_domain,
        access_token: renewed.usable_token().unwrap_or_default().to_string(),
    })
}

// ---- integration.service.ts --------------------------------------------------

/// `shopifyWebhookPath`.
pub fn webhook_path(workspace_id: i64) -> String {
    format!("/api/shopify/{workspace_id}/webhook")
}

fn webhook_url(app_url: &str, workspace_id: i64) -> String {
    chatbotx_core::utils::web_url::join_app_url(app_url, &webhook_path(workspace_id))
}

/// `toView`: what the builder and the public API show; never a secret.
pub fn view(row: &IntegrationShopify, webhook_url: &str) -> Value {
    json!({
        "connected": true,
        "shopDomain": row.shop_domain,
        "shopName": row.shop_name,
        "currency": row.currency,
        "authKind": row.auth_kind,
        "status": row.status,
        "statusReason": row.status_reason,
        "syncStatus": row.sync_status,
        "syncError": row.sync_error,
        "lastSyncedAt": row.last_synced_at.map(iso),
        "productCount": row.product_count,
        "orderCount": row.order_count,
        "webhookUrl": webhook_url,
        "adminUrl": client::admin_url(&row.shop_domain),
        "connectedAt": iso(row.created_at),
    })
}

async fn find(db: &chatbotx_db::Db, workspace_id: i64) -> Result<Option<IntegrationShopify>> {
    tx::read(db, move |conn| {
        Ok(store_integration::find_shopify(conn, workspace_id)?)
    })
    .await
}

/// `getStatus`.
pub async fn get_status(db: &chatbotx_db::Db, app_url: &str, workspace_id: i64) -> Result<Value> {
    let url = webhook_url(app_url, workspace_id);
    Ok(match find(db, workspace_id).await? {
        Some(row) => view(&row, &url),
        None => json!({ "connected": false, "webhookUrl": url }),
    })
}

/// `shopifyShopDomainSchema`: lower-cased, without scheme or path.
pub fn parse_shop_domain(input: &str) -> Result<String> {
    let lower = input.trim().to_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let domain = without_scheme.split('/').next().unwrap_or_default();
    let name = domain.strip_suffix(".myshopify.com").unwrap_or_default();
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(BusinessError::Exception {
            message: "Enter the store's .myshopify.com domain, e.g. your-store.myshopify.com."
                .into(),
            code: "validationError".into(),
            status: 400,
            field: Some("shopDomain".into()),
            data: None,
        });
    }
    Ok(domain.to_string())
}

/// `ShopifyConnectInput` (after the request schema).
#[derive(Debug, Clone)]
pub enum ConnectAuth {
    Token {
        access_token: String,
        api_secret: String,
    },
    ClientCredentials {
        client_id: String,
        client_secret: String,
    },
}

/// What connect / sync / disconnect reach besides the database.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub db: &'a chatbotx_db::Db,
    pub jobs: Option<&'a dyn Enqueuer>,
    pub app_url: &'a str,
    pub audit: &'a AuditService,
}

async fn remove_subscriptions_quietly(credentials: &Credentials, ids: &[String]) {
    for id in ids {
        if let Err(error) = client::delete_webhook_subscription(credentials, id).await {
            tracing::warn!(%error, subscription_id = %id, "[shopify] could not remove a webhook subscription");
        }
    }
}

async fn remove_previous_subscriptions_quietly(db: &chatbotx_db::Db, row: &IntegrationShopify) {
    match resolve_credentials(db, row).await {
        Ok(credentials) => {
            let ids: Vec<String> =
                serde_json::from_value(row.webhook_subscription_ids.0.clone()).unwrap_or_default();
            remove_subscriptions_quietly(&credentials, &ids).await;
        }
        Err(error) => tracing::warn!(
            %error,
            workspace_id = row.workspace_id,
            "[shopify] could not reach the store to remove its webhooks"
        ),
    }
}

/// `connect`: checks the credentials, subscribes our webhook URL to every
/// topic, saves the connection encrypted and queues a full sync.
pub async fn connect(
    ctx: Ctx<'_>,
    workspace_id: i64,
    shop_domain: &str,
    auth: ConnectAuth,
) -> Result<Value> {
    let shop_domain = parse_shop_domain(shop_domain)?;
    let secret = match auth {
        ConnectAuth::Token {
            access_token,
            api_secret,
        } => AuthSecret::Token {
            access_token: access_token.trim().to_string(),
            api_secret: api_secret.trim().to_string(),
        },
        ConnectAuth::ClientCredentials {
            client_id,
            client_secret,
        } => issue_client_credentials_token(&shop_domain, client_id.trim(), client_secret.trim())
            .await
            .map_err(|e| connect_error(&e))?,
    };
    let credentials = Credentials {
        shop_domain: shop_domain.clone(),
        access_token: secret.usable_token().unwrap_or_default().to_string(),
    };
    let shop = client::get_shop(&credentials)
        .await
        .map_err(|e| connect_error(&e))?;

    let previous = find(ctx.db, workspace_id).await?;
    let url = webhook_url(ctx.app_url, workspace_id);
    let mut ids: Vec<String> = Vec::new();
    let subscribed: std::result::Result<(), ApiError> = async {
        // Ours already on the store would make Shopify refuse the address again.
        let existing = client::list_webhook_subscription_ids(&credentials, &url).await?;
        remove_subscriptions_quietly(&credentials, &existing).await;
        for topic in WEBHOOK_TOPICS {
            ids.push(client::create_webhook_subscription(&credentials, topic, &url).await?);
        }
        Ok(())
    }
    .await;
    if let Err(error) = subscribed {
        remove_subscriptions_quietly(&credentials, &ids).await;
        return Err(connect_error(&error));
    }

    let encrypted = encrypt_auth(workspace_id, &secret)?;
    let values: Vec<(&'static str, Sql)> = vec![
        ("shopDomain", Sql::Text(shop_domain.clone())),
        ("shopName", external_store::text(shop["name"].as_str())),
        (
            "currency",
            Sql::Text(shop["currencyCode"].as_str().unwrap_or("USD").to_string()),
        ),
        ("authKind", Sql::Text(secret.kind().into())),
        ("encryptedAuth", Sql::Text(encrypted.to_string())),
        ("webhookSubscriptionIds", Sql::Text(json!(ids).to_string())),
        ("status", Sql::Text("active".into())),
        ("statusReason", Sql::Null),
        ("syncStatus", Sql::Text("idle".into())),
        ("syncError", Sql::Null),
        ("productCount", Sql::Integer(0)),
        ("orderCount", Sql::Integer(0)),
    ];
    let saved = tx::write(ctx.db, move |tx| {
        let conn: &Connection = tx;
        Ok(store_integration::upsert_shopify(
            conn,
            workspace_id,
            &values,
        )?)
    })
    .await;
    if let Err(error) = saved {
        remove_subscriptions_quietly(&credentials, &ids).await;
        return Err(error);
    }
    // The old connection's subscriptions would keep calling us.
    if let Some(previous) = previous {
        remove_previous_subscriptions_quietly(ctx.db, &previous).await;
    }
    ctx.audit
        .audit("connect", &format!("connected Shopify store {shop_domain}"))
        .await;
    request_sync(ctx, workspace_id, "all").await
}

/// `requestSync`: queues a sync unless a fresh one is queued or running.
pub async fn request_sync(ctx: Ctx<'_>, workspace_id: i64, scope: &str) -> Result<Value> {
    let row = find(ctx.db, workspace_id)
        .await?
        .ok_or_else(not_connected)?;
    let stale = chatbotx_db::repo::now_ms() - row.updated_at > STALE_SYNC_MS;
    if matches!(row.sync_status.as_str(), "queued" | "running") && !stale {
        return Err(sync_in_progress());
    }
    let set = |status: &str, error: Option<&str>| {
        vec![
            ("syncStatus", Sql::Text(status.to_string())),
            ("syncError", external_store::text(error)),
        ]
    };
    external_store::update_connection(ctx.db, SOURCE, workspace_id, set("queued", None)).await?;
    let job_id = format!(
        "shopify-sync-{workspace_id}-{scope}-{}",
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
        // Not left "queued" forever, or no later sync could start.
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
    Ok(view(&updated, &webhook_url(ctx.app_url, workspace_id)))
}

/// `disconnect`: Shopify stops calling us (best effort), the connection is
/// removed and the store goes back to its own catalog.
pub async fn disconnect(ctx: Ctx<'_>, workspace_id: i64) -> Result<()> {
    let Some(row) = find(ctx.db, workspace_id).await? else {
        return Ok(());
    };
    remove_previous_subscriptions_quietly(ctx.db, &row).await;
    tx::write(ctx.db, move |tx| {
        let conn: &Connection = tx;
        Ok(store_integration::delete_shopify(conn, workspace_id)?)
    })
    .await?;
    external_store::switch_store_back_from(ctx.db, workspace_id, SOURCE).await?;
    ctx.audit
        .audit(
            "disconnect",
            &format!("disconnected Shopify store {}", row.shop_domain),
        )
        .await;
    Ok(())
}

/// `markInvalid`: the app was uninstalled, or Shopify refused the credentials.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_domains() {
        assert_eq!(
            parse_shop_domain(" HTTPS://My-Store.myshopify.com/admin ").unwrap(),
            "my-store.myshopify.com"
        );
        for bad in [
            "example.com",
            "-x.myshopify.com",
            ".myshopify.com",
            "a_b.myshopify.com",
        ] {
            assert!(parse_shop_domain(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn secrets_round_trip_as_the_original() {
        let secret: AuthSecret = serde_json::from_value(json!({
            "kind": "clientCredentials", "clientId": "id", "clientSecret": "s",
            "accessToken": null, "accessTokenExpiresAt": null
        }))
        .unwrap();
        assert_eq!(secret.webhook_secret(), "s");
        assert_eq!(secret.usable_token(), None);
        let token = AuthSecret::Token {
            access_token: "shpat".into(),
            api_secret: "k".into(),
        };
        assert_eq!(serde_json::to_value(&token).unwrap()["apiSecret"], "k");
        assert_eq!(token.usable_token(), Some("shpat"));
    }
}
