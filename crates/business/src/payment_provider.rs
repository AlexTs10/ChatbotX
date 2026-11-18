//! `packages/business/src/payment-provider/{service,errors}.ts`: a store's
//! Stripe and PayPal connections (`EcommercePaymentProvider`), their
//! encrypted secrets and the gateway that takes its payments.
//!
//! ponytail: Stripe Connect through the platform app (OAuth link, callback,
//! `saveStripeConnect`) is not ported: no `/v1` route reaches it and the SPA
//! connects Stripe with a key. A row it left behind (no secret) still loads
//! its gateway and is deauthorized on disconnect.

use std::sync::LazyLock;

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::repo::models::EcommercePaymentProvider;
use chatbotx_db::repo::{find_many, find_one, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, repo};
use chatbotx_kv::lock::{LockMap, LockOpts};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::payments::{
    ErrorKind, Gateway, PaypalSecret, ProviderError, StripeKeySecret, paypal, stripe,
};
use crate::{platform_credential, tx};

pub const PROVIDERS: &[&str] = &["stripe", "paypal"];

// ---- errors.ts ---------------------------------------------------------------

pub fn not_connected(provider: &str) -> BusinessError {
    BusinessError::exception(
        format!("Connect {provider} in Settings → Integrations first."),
        "paymentProviderNotConnected",
        400,
    )
    .with_data(json!({ "provider": provider }))
}

pub fn in_use(provider: &str, count: i64) -> BusinessError {
    BusinessError::conflict(
        format!("{count} open order(s) were paid with {provider} or are waiting for it. Complete or cancel them first."),
        "paymentProviderInUse",
    )
    .with_data(json!({ "provider": provider, "count": count }))
}

pub fn platform_not_configured() -> BusinessError {
    BusinessError::exception(
        "Stripe Connect is not set up on this platform yet (secret key and Connect client id).",
        "stripePlatformNotConfigured",
        400,
    )
}

pub fn secret_key_invalid() -> BusinessError {
    BusinessError::exception(
        "Paste a Stripe secret key (sk_live_… or sk_test_…) or a restricted key (rk_…), not a publishable key (pk_…).",
        "stripeSecretKeyInvalid",
        400,
    )
}

pub fn method_unavailable(method: &str) -> BusinessError {
    BusinessError::exception(
        format!("The store does not take {method} payments."),
        "paymentMethodUnavailable",
        400,
    )
    .with_data(json!({ "method": method }))
}

/// `toPaymentProviderException`.
pub fn to_exception(provider: &str, error: &ProviderError) -> BusinessError {
    let data = json!({ "provider": provider, "providerCode": error.provider_code.clone().unwrap_or_default() });
    let (message, code, status) = match error.kind {
        ErrorKind::CredentialsRejected => (
            format!("{provider} refused the credentials."),
            "paymentProviderCredentialsRejected",
            400,
        ),
        ErrorKind::AmountUnsupported => (error.message.clone(), "paymentAmountUnsupported", 400),
        ErrorKind::Unavailable => (
            format!("{provider} is not answering right now. Try again in a moment."),
            "paymentProviderUnavailable",
            503,
        ),
        _ => (error.message.clone(), "paymentProviderRejected", 400),
    };
    BusinessError::exception(message, code, status).with_data(data)
}

// ---- service.ts ----------------------------------------------------------------

/// What the services need around a provider call.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub db: &'a Db,
    pub encryption: &'a Encryption,
    /// `resolveWorkspaceAppUrl` (the platform's app URL here).
    pub app_url: &'a str,
}

fn paypal_aad(workspace_id: i64) -> String {
    format!("ecommerce-payment:{workspace_id}:paypal")
}

fn stripe_aad(workspace_id: i64) -> String {
    format!("ecommerce-payment:{workspace_id}:stripe")
}

pub fn paypal_webhook_path(workspace_id: i64) -> String {
    format!("/api/payments/paypal/{workspace_id}/webhook")
}

pub fn stripe_account_webhook_path(workspace_id: i64) -> String {
    format!("/api/payments/stripe/workspace/{workspace_id}/webhook")
}

fn is_stripe_key_row(row: &EcommercePaymentProvider) -> bool {
    row.provider == "stripe" && row.secret.as_ref().is_some_and(|s| !s.0.is_null())
}

fn mask_client_id(client_id: &str) -> String {
    let chars: Vec<char> = client_id.chars().collect();
    if chars.len() <= 8 {
        return "••••".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// `toView` (API shape: ids as strings, dates ISO).
pub fn to_view(row: &EcommercePaymentProvider) -> Value {
    json!({
        "provider": row.provider,
        "connectionType": if row.provider == "stripe" && !is_stripe_key_row(row) { "oauth" } else { "apiKey" },
        "livemode": row.livemode,
        "accountLabel": row.account_label,
        "accountId": row.external_account_id,
        "connectedAt": repo::iso_millis(row.created_at),
    })
}

/// One Stripe connect at a time per workspace (`STRIPE_CONNECT_LOCK_SECONDS`).
static CONNECT_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);
const CONNECT_LOCK: LockOpts = LockOpts {
    ttl_secs: 90,
    retry_secs: None,
};

pub fn find(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
) -> repo::Result<Option<EcommercePaymentProvider>> {
    find_one(
        conn,
        r#""workspaceId" = ?1 AND "provider" = ?2"#,
        &[&workspace_id, &provider],
    )
}

pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let rows: Vec<EcommercePaymentProvider> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 ORDER BY "provider" ASC"#,
            &[&workspace_id],
        )?;
        Ok(rows.iter().map(to_view).collect())
    })
    .await
}

/// `isStripeAvailable`: the owner's platform has a Stripe Connect app.
pub async fn is_stripe_available(db: &Db, encryption: &Encryption, owner_id: i64) -> bool {
    let encryption = encryption.clone();
    tx::read(db, move |conn| {
        let credential =
            platform_credential::resolve_for_owner(conn, &encryption, owner_id, "stripe", false)?;
        Ok(credential.is_some_and(|c| {
            c.text("secretKey").is_some_and(|s| !s.is_empty())
                && c.text("connectClientId").is_some_and(|s| !s.is_empty())
        }))
    })
    .await
    .unwrap_or(false)
}

/// `countOrdersNeedingProvider`.
pub fn count_orders_needing(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
) -> repo::Result<i64> {
    Ok(conn.query_row(
        r#"SELECT COUNT(*) FROM "Order" WHERE "workspaceId" = ?1 AND "paymentMethod" = ?2
           AND "status" IN ('pending','accepted','shipped') AND "paymentStatus" IN ('unpaid','paid')"#,
        params![workspace_id, provider],
        |r| r.get(0),
    )?)
}

async fn assert_not_needed_by_orders(
    db: &Db,
    workspace_id: i64,
    provider: &'static str,
) -> Result<()> {
    let count = tx::read(db, move |conn| {
        Ok(count_orders_needing(conn, workspace_id, provider)?)
    })
    .await?;
    if count > 0 {
        return Err(in_use(provider, count));
    }
    Ok(())
}

struct Upsert {
    workspace_id: i64,
    provider: &'static str,
    livemode: bool,
    account_label: Option<String>,
    external_account_id: Option<String>,
    secret: Option<Value>,
    webhook_endpoint_id: Option<String>,
}

/// `paymentProviderRepository.upsert` (`platformCredentialId` NULL: key rows).
fn upsert(conn: &Connection, v: &Upsert) -> repo::Result<EcommercePaymentProvider> {
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "EcommercePaymentProvider" ("id","createdAt","updatedAt","workspaceId","provider","livemode","accountLabel","externalAccountId","platformCredentialId","secret","webhookEndpointId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9)
           ON CONFLICT ("workspaceId","provider") DO UPDATE SET "livemode" = excluded."livemode",
             "accountLabel" = excluded."accountLabel", "externalAccountId" = excluded."externalAccountId",
             "platformCredentialId" = NULL, "secret" = excluded."secret",
             "webhookEndpointId" = excluded."webhookEndpointId", "updatedAt" = excluded."updatedAt""#,
        params![
            new_id(),
            now,
            v.workspace_id,
            v.provider,
            v.livemode,
            v.account_label,
            v.external_account_id,
            v.secret.as_ref().map(Value::to_string),
            v.webhook_endpoint_id,
        ],
    )?;
    find(conn, v.workspace_id, v.provider)?
        .ok_or_else(|| repo::Error::msg("Failed to save the payment connection"))
}

fn encrypt(encryption: &Encryption, value: &impl serde::Serialize, aad: &str) -> Result<Value> {
    let data = encryption
        .encrypt_object(value, Some(aad))
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;
    serde_json::to_value(data).map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))
}

fn decrypt<T: serde::de::DeserializeOwned>(
    encryption: &Encryption,
    row: &EcommercePaymentProvider,
    aad: &str,
) -> Option<T> {
    let blob: EncryptedData = serde_json::from_value(row.secret.as_ref()?.0.clone()).ok()?;
    encryption.decrypt_object(&blob, Some(aad)).ok()
}

fn paypal_secret(encryption: &Encryption, row: &EcommercePaymentProvider) -> Option<PaypalSecret> {
    decrypt::<PaypalSecret>(encryption, row, &paypal_aad(row.workspace_id)).filter(|s| {
        !s.client_id.is_empty() && !s.client_secret.is_empty() && !s.webhook_id.is_empty()
    })
}

fn stripe_secret(
    encryption: &Encryption,
    row: &EcommercePaymentProvider,
) -> Option<StripeKeySecret> {
    decrypt::<StripeKeySecret>(encryption, row, &stripe_aad(row.workspace_id)).filter(|s| {
        [
            &s.secret_key,
            &s.account_id,
            &s.webhook_id,
            &s.webhook_secret,
        ]
        .iter()
        .all(|v| !v.is_empty())
    })
}

async fn load(
    db: &Db,
    workspace_id: i64,
    provider: &'static str,
) -> Result<Option<EcommercePaymentProvider>> {
    tx::read(db, move |conn| Ok(find(conn, workspace_id, provider)?)).await
}

/// `connectStripeKey`: the key is checked, a webhook created on the account
/// and both saved encrypted. Returns the view with `chargesEnabled`.
pub async fn connect_stripe_key(
    ctx: Ctx<'_>,
    workspace_id: i64,
    secret_key: &str,
) -> Result<Value> {
    let secret_key = secret_key.trim().to_string();
    if !stripe::is_stripe_secret_key(&secret_key) {
        return Err(secret_key_invalid());
    }
    let account = stripe::read_key_account(stripe::STRIPE_API, &secret_key)
        .await
        .map_err(|e| to_exception("stripe", &e))?;
    let key = format!("payment-provider-connect:{workspace_id}:stripe");
    let _guard = CONNECT_LOCKS
        .acquire(&key, CONNECT_LOCK)
        .await
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;

    let previous = load(ctx.db, workspace_id, "stripe").await?;
    let replaces_account = previous.as_ref().is_some_and(|p| {
        p.external_account_id.as_deref() != Some(account.account_id.as_str())
            || p.livemode != account.livemode
    });
    if replaces_account {
        assert_not_needed_by_orders(ctx.db, workspace_id, "stripe").await?;
    }
    let url = chatbotx_core::utils::web_url::join_app_url(
        ctx.app_url,
        &stripe_account_webhook_path(workspace_id),
    );
    let (webhook_id, webhook_secret) =
        stripe::create_account_webhook(stripe::STRIPE_API, &secret_key, &url)
            .await
            .map_err(|e| to_exception("stripe", &e))?;
    let connection = StripeKeySecret {
        secret_key: secret_key.clone(),
        livemode: account.livemode,
        account_id: account.account_id.clone(),
        webhook_id: webhook_id.clone(),
        webhook_secret,
    };
    let values = Upsert {
        workspace_id,
        provider: "stripe",
        livemode: account.livemode,
        account_label: account.label.clone(),
        external_account_id: Some(account.account_id.clone()),
        secret: Some(encrypt(
            ctx.encryption,
            &connection,
            &stripe_aad(workspace_id),
        )?),
        webhook_endpoint_id: Some(webhook_id.clone()),
    };
    let saved = tx::write(ctx.db, move |tx| Ok(upsert(tx, &values)?)).await;
    let row = match saved {
        Ok(row) => row,
        Err(error) => {
            remove_stripe_webhook_quietly(workspace_id, &[&secret_key], &webhook_id).await;
            return Err(error);
        }
    };
    if let Some(previous) = previous {
        if is_stripe_key_row(&previous) {
            // On the same account the new key removes the old webhook first.
            let old = stripe_secret(ctx.encryption, &previous);
            let webhook = old
                .as_ref()
                .map(|s| s.webhook_id.clone())
                .or(previous.webhook_endpoint_id.clone());
            let mut keys: Vec<&str> = Vec::new();
            if !replaces_account {
                keys.push(&secret_key);
            }
            if let Some(old) = &old
                && !keys.contains(&old.secret_key.as_str())
            {
                keys.push(&old.secret_key);
            }
            if let Some(webhook) = webhook {
                remove_stripe_webhook_quietly(workspace_id, &keys, &webhook).await;
            }
        } else {
            deauthorize_quietly(ctx, &previous).await;
        }
    }
    let mut view = to_view(&row);
    view["chargesEnabled"] = json!(account.charges_enabled);
    Ok(view)
}

/// `connectPaypal`: live first, then sandbox; the webhook created; the keys
/// saved encrypted.
pub async fn connect_paypal(
    ctx: Ctx<'_>,
    workspace_id: i64,
    client_id: &str,
    client_secret: &str,
) -> Result<Value> {
    let previous = load(ctx.db, workspace_id, "paypal").await?;
    if let Some(previous) = &previous {
        let same = paypal_secret(ctx.encryption, previous)
            .is_some_and(|s| s.client_id == client_id.trim());
        if !same {
            assert_not_needed_by_orders(ctx.db, workspace_id, "paypal").await?;
        }
    }
    let url = chatbotx_core::utils::web_url::join_app_url(
        ctx.app_url,
        &paypal_webhook_path(workspace_id),
    );
    let secret = paypal::connect(client_id, client_secret, &url)
        .await
        .map_err(|e| to_exception("paypal", &e))?;
    let values = Upsert {
        workspace_id,
        provider: "paypal",
        livemode: secret.livemode,
        account_label: Some(mask_client_id(&secret.client_id)),
        external_account_id: None,
        secret: Some(encrypt(ctx.encryption, &secret, &paypal_aad(workspace_id))?),
        webhook_endpoint_id: Some(secret.webhook_id.clone()),
    };
    let row = tx::write(ctx.db, move |tx| Ok(upsert(tx, &values)?)).await?;
    if let Some(previous) = previous
        && previous.webhook_endpoint_id != row.webhook_endpoint_id
    {
        remove_paypal_webhook_quietly(ctx.encryption, &previous).await;
    }
    Ok(to_view(&row))
}

/// `disconnect`: the provider side first (best effort), then the row and the
/// payment method together. `false` = nothing was connected.
pub async fn disconnect(ctx: Ctx<'_>, workspace_id: i64, provider: &'static str) -> Result<bool> {
    let Some(row) = load(ctx.db, workspace_id, provider).await? else {
        return Ok(false);
    };
    assert_not_needed_by_orders(ctx.db, workspace_id, provider).await?;
    if is_stripe_key_row(&row) {
        let secret = stripe_secret(ctx.encryption, &row);
        let webhook = secret
            .as_ref()
            .map(|s| s.webhook_id.clone())
            .or(row.webhook_endpoint_id.clone());
        if let (Some(secret), Some(webhook)) = (&secret, webhook) {
            remove_stripe_webhook_quietly(workspace_id, &[&secret.secret_key], &webhook).await;
        }
    } else if row.provider == "stripe" {
        deauthorize_quietly(ctx, &row).await;
    } else {
        remove_paypal_webhook_quietly(ctx.encryption, &row).await;
    }
    remove_connection(ctx.db, workspace_id, provider).await?;
    Ok(true)
}

async fn remove_connection(db: &Db, workspace_id: i64, provider: &'static str) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "EcommercePaymentProvider" WHERE "workspaceId" = ?1 AND "provider" = ?2"#,
            params![workspace_id, provider],
        )?;
        Ok(())
    })
    .await?;
    crate::ecommerce_settings::drop_payment_method(db, workspace_id, provider).await
}

/// `handleStripeDeauthorized`: an event older than the connection leaves it.
pub async fn handle_stripe_deauthorized(
    db: &Db,
    platform_credential_id: i64,
    account_id: &str,
    deauthorized_at: i64,
) -> Result<()> {
    let account_id = account_id.to_string();
    let row: Option<EcommercePaymentProvider> = tx::read(db, move |conn| {
        Ok(find_one(
            conn,
            r#""provider" = 'stripe' AND "platformCredentialId" = ?1 AND "externalAccountId" = ?2"#,
            &[&platform_credential_id, &account_id],
        )?)
    })
    .await?;
    match row {
        Some(row) if row.updated_at <= deauthorized_at => {
            remove_connection(db, row.workspace_id, "stripe").await
        }
        _ => Ok(()),
    }
}

/// `loadGateway`: the gateway and its mode.
pub fn load_gateway(
    conn: &Connection,
    encryption: &Encryption,
    workspace_id: i64,
    provider: &str,
) -> Result<(Gateway, bool)> {
    let provider_name: &'static str = if provider == "stripe" {
        "stripe"
    } else {
        "paypal"
    };
    let Some(row) = find(conn, workspace_id, provider_name)? else {
        return Err(not_connected(provider_name));
    };
    if is_stripe_key_row(&row) {
        let secret = stripe_secret(encryption, &row).ok_or_else(|| not_connected("stripe"))?;
        return Ok((
            Gateway::Stripe(stripe::StripeAccount::own(&secret.secret_key)),
            row.livemode,
        ));
    }
    if row.provider == "stripe" {
        let platform = match row.platform_credential_id {
            Some(id) => platform_credential::find_decrypted_by_id(conn, encryption, id, "stripe")?,
            None => None,
        };
        let key = platform
            .as_ref()
            .and_then(|p| p.text("secretKey"))
            .filter(|k| !k.is_empty());
        let (Some(key), Some(account)) = (key, row.external_account_id.as_deref()) else {
            return Err(not_connected("stripe"));
        };
        return Ok((
            Gateway::Stripe(stripe::StripeAccount::connected(key, account)),
            row.livemode,
        ));
    }
    let secret = paypal_secret(encryption, &row).ok_or_else(|| not_connected("paypal"))?;
    Ok((
        Gateway::Paypal(paypal::Paypal::from_secret(&secret)),
        row.livemode,
    ))
}

/// `loadPaypalSecret`.
pub fn load_paypal_secret(
    conn: &Connection,
    encryption: &Encryption,
    workspace_id: i64,
) -> repo::Result<Option<PaypalSecret>> {
    Ok(find(conn, workspace_id, "paypal")?.and_then(|row| paypal_secret(encryption, &row)))
}

/// `loadStripeAccountWebhook`: the store key's webhook secret.
pub fn load_stripe_account_webhook(
    conn: &Connection,
    encryption: &Encryption,
    workspace_id: i64,
) -> repo::Result<Option<StripeKeySecret>> {
    Ok(find(conn, workspace_id, "stripe")?
        .filter(is_stripe_key_row)
        .and_then(|row| stripe_secret(encryption, &row)))
}

/// `loadStripeConnectWebhook`: `(platformSecretKey, webhookSecret)`.
pub fn load_stripe_connect_webhook(
    conn: &Connection,
    encryption: &Encryption,
    platform_credential_id: i64,
) -> Result<Option<(String, String)>> {
    let platform = platform_credential::find_decrypted_by_id(
        conn,
        encryption,
        platform_credential_id,
        "stripe",
    )?;
    Ok(platform.and_then(|p| {
        let key = p.text("secretKey").filter(|s| !s.is_empty())?;
        let secret = p.text("connectWebhookSecret").filter(|s| !s.is_empty())?;
        Some((key.to_string(), secret.to_string()))
    }))
}

/// `findByStripeAccount`: the workspace a Connect event belongs to.
pub fn find_by_stripe_account(
    conn: &Connection,
    platform_credential_id: i64,
    account_id: &str,
) -> repo::Result<Option<EcommercePaymentProvider>> {
    find_one(
        conn,
        r#""provider" = 'stripe' AND "platformCredentialId" = ?1 AND "externalAccountId" = ?2"#,
        &[&platform_credential_id, &account_id],
    )
}

async fn remove_stripe_webhook_quietly(workspace_id: i64, keys: &[&str], webhook_id: &str) {
    let mut last = None;
    for key in keys {
        match stripe::delete_webhook(stripe::STRIPE_API, key, webhook_id).await {
            Ok(()) => return,
            Err(error) => last = Some(error),
        }
    }
    if let Some(error) = last {
        tracing::warn!(%error, workspace_id, "[payments] could not remove the Stripe webhook");
    }
}

async fn deauthorize_quietly(ctx: Ctx<'_>, row: &EcommercePaymentProvider) {
    let (Some(id), Some(account)) = (row.platform_credential_id, row.external_account_id.clone())
    else {
        return;
    };
    let encryption = ctx.encryption.clone();
    let platform = tx::read(ctx.db, move |conn| {
        platform_credential::find_decrypted_by_id(conn, &encryption, id, "stripe")
    })
    .await;
    let Ok(Some(platform)) = platform else {
        return;
    };
    let (Some(key), Some(client)) = (platform.text("secretKey"), platform.text("connectClientId"))
    else {
        return;
    };
    if let Err(error) = stripe::deauthorize(key, client, &account).await {
        tracing::warn!(%error, workspace_id = row.workspace_id, "[payments] could not deauthorize the Stripe account");
    }
}

async fn remove_paypal_webhook_quietly(encryption: &Encryption, row: &EcommercePaymentProvider) {
    if let Some(secret) = paypal_secret(encryption, row)
        && let Err(error) = paypal::disconnect(&secret).await
    {
        tracing::warn!(%error, workspace_id = row.workspace_id, "[payments] could not remove the PayPal webhook");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_the_client_id() {
        assert_eq!(mask_client_id("short"), "••••");
        assert_eq!(mask_client_id("AbCdEfGhIjwxyz"), "AbCd…wxyz");
    }

    #[test]
    fn provider_errors_map_like_the_original() {
        let e = to_exception("stripe", &ProviderError::new(ErrorKind::Unavailable, "x"));
        assert_eq!(
            (e.code(), e.status()),
            (Some("paymentProviderUnavailable"), Some(503))
        );
        let e = to_exception(
            "paypal",
            &ProviderError::new(ErrorKind::CredentialsRejected, "x"),
        );
        assert_eq!(e.message(), "paypal refused the credentials.");
        let e = to_exception(
            "stripe",
            &ProviderError::new(ErrorKind::InvalidRequest, "No such thing."),
        );
        assert_eq!(
            (e.code(), e.message().as_str()),
            (Some("paymentProviderRejected"), "No such thing.")
        );
    }
}
