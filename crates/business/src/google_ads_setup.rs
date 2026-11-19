//! The setup side of `integrationGoogleAdsService` (`getSetup`,
//! `isConfigured`, `resolveDeveloperToken`, `refreshSetup`) and the token
//! upkeep of `buildContext` / `refreshAuth` for the Google Ads calls.

use chatbotx_core::encryption::Encryption;
use chatbotx_db::Db;
use chatbotx_db::repo::models::IntegrationGoogleAds;
use chatbotx_db::repo::{connection, integration_google_ads, now_ms};
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use crate::connection::credentials::Upstream;
use crate::connection::oauth::{self, OAuthCredential};
use crate::email_marketing::{open_auth, seal_auth};
use crate::errors::Result;
use crate::google_ads_api::{self as api, Creds, Failure};
use crate::{BusinessError, tx};

/// `GoogleAdsSetup`.
#[derive(Debug, Clone)]
pub struct Setup {
    pub integration: IntegrationGoogleAds,
    pub connection_id: Option<i64>,
    pub connection_status: Option<String>,
    /// `ready` | `needs_reauth` | `setup_incomplete`.
    pub readiness: &'static str,
}

/// `deriveReadiness`.
pub fn readiness(integration: &IntegrationGoogleAds, status: Option<&str>) -> &'static str {
    if !status.is_some_and(crate::connection::is_active) {
        "needs_reauth"
    } else if integration.conversion_customer_id.is_some()
        && integration.conversion_actions_synced_at.is_some()
    {
        "ready"
    } else {
        "setup_incomplete"
    }
}

/// `getSetup`.
pub async fn get_setup(db: &Db, workspace_id: i64) -> Result<Option<Setup>> {
    tx::read(db, move |conn| {
        let Some(integration) = integration_google_ads::find_by_workspace_id(conn, workspace_id)?
        else {
            return Ok(None);
        };
        let row = connection::find_by_integration_id(conn, integration.integration_id)?;
        let status = row.as_ref().map(|c| c.status.clone());
        Ok(Some(Setup {
            readiness: readiness(&integration, status.as_deref()),
            connection_id: row.map(|c| c.id),
            connection_status: status,
            integration,
        }))
    })
    .await
}

/// The decrypted stored auth of the satellite row.
pub fn auth_of(integration: &IntegrationGoogleAds) -> Value {
    serde_json::to_string(&integration.auth.0)
        .ok()
        .and_then(|raw| open_auth(&raw))
        .unwrap_or(Value::Null)
}

/// The owner's `googleAds` platform credential (`resolveForOwner`, strict).
pub async fn credential(
    db: &Db,
    encryption: &Encryption,
    app_url: &str,
    workspace_id: i64,
) -> Result<Option<OAuthCredential>> {
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    oauth::resolve_credential(db, encryption, app_url, "googleAds", Some(owner)).await
}

/// `isConfigured`: the credential has a client id and secret.
pub fn is_configured(credential: Option<&OAuthCredential>) -> bool {
    credential.is_some_and(|c| {
        !oauth_text(&c.config, "clientId").is_empty()
            && !oauth_text(&c.config, "clientSecret").is_empty()
    })
}

fn oauth_text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// `resolveDeveloperToken`: `None` = no credential; `Some(None)` = no token
/// (optional since 2026-09).
pub fn developer_token(credential: Option<&OAuthCredential>) -> Option<Option<String>> {
    credential.map(|c| {
        c.config["developerToken"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    })
}

/// Why no usable token exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenFailure {
    /// The grant is revoked: the connection is marked `needs_reauth`.
    Revoked,
    Other(String),
}

/// The stored auth with a live access token: refreshed (and saved) when it
/// is expired, or always when `force` (after a 401). A revoked grant marks
/// the connection `needs_reauth` (`refresh_failed`).
pub async fn fresh_auth(
    db: &Db,
    up: &impl Upstream,
    integration: &IntegrationGoogleAds,
    force: bool,
) -> std::result::Result<Value, TokenFailure> {
    let auth = auth_of(integration);
    if auth.is_null() {
        return Err(TokenFailure::Other(
            "Google Ads auth could not be read".into(),
        ));
    }
    if !force && !api::token_expired(&auth, now_ms()) {
        return Ok(auth);
    }
    let (id, ws, integration_id) = (
        integration.id,
        integration.workspace_id,
        integration.integration_id,
    );
    match api::refresh_auth(up, &auth).await {
        Ok(next) => {
            let sealed = seal_auth(&next).map_err(|e| TokenFailure::Other(e.to_string()))?;
            tx::write(db, move |conn| {
                conn.execute(
                    r#"UPDATE "IntegrationGoogleAds" SET "auth" = ?3, "updatedAt" = ?4
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws, sealed, now_ms()],
                )
                .map_err(|e| BusinessError::Repo(e.into()))
            })
            .await
            .map_err(|e| TokenFailure::Other(e.to_string()))?;
            Ok(next)
        }
        Err((true, message)) => {
            tracing::warn!(workspace_id = ws, %message, "google ads: grant revoked, connection needs reauth");
            let _ = tx::write(db, move |conn| {
                conn.execute(
                    r#"UPDATE "Connection" SET "status" = 'needs_reauth', "statusReason" = 'refresh_failed',
                           "lastError" = ?2, "updatedAt" = ?3
                       WHERE "integrationId" = ?1 AND "status" IN ('connected', 'degraded')"#,
                    params![integration_id, message, now_ms()],
                )
                .map_err(|e| BusinessError::Repo(e.into()))
            })
            .await;
            Err(TokenFailure::Revoked)
        }
        Err((false, message)) => Err(TokenFailure::Other(message)),
    }
}

/// `GoogleAdsSetupError` of a failed sync (`toSetupError`).
fn setup_error(failure: &Failure) -> &'static str {
    match failure {
        Failure::Google(e) if matches!(e.status, 403 | 404) => "conversion_customer_inaccessible",
        _ => "sync_failed",
    }
}

async fn record_setup_error(db: &Db, integration: &IntegrationGoogleAds, code: &str) -> Result<()> {
    let (id, ws, code) = (integration.id, integration.workspace_id, code.to_string());
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "IntegrationGoogleAds" SET "setupError" = ?3, "setupErrorAt" = ?4, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, ws, code, now],
        )
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await?;
    Ok(())
}

/// `resolveConversionCustomer` + `listConversionActions` on a live token.
async fn sync_from_google(
    up: &impl Upstream,
    auth: &Value,
    integration: &IntegrationGoogleAds,
    developer_token: Option<&str>,
) -> std::result::Result<(String, Option<bool>, Vec<Value>), Failure> {
    let creds = Creds {
        access_token: auth["tokens"]["accessToken"].as_str().unwrap_or_default(),
        developer_token,
        login_customer_id: integration.login_customer_id.as_deref(),
    };
    let customer = api::get_customer(up, creds, &integration.customer_id)
        .await?
        .ok_or_else(|| {
            Failure::Google(api::GoogleError::terminal(
                404,
                "customerNotFound",
                "Google Ads customer was not found",
            ))
        })?;
    let conversion_customer = customer.conversion_customer_id.unwrap_or(customer.id);
    let actions = api::list_conversion_actions(up, creds, &conversion_customer).await?;
    Ok((
        conversion_customer,
        customer.accepted_customer_data_terms,
        actions,
    ))
}

/// `getConversionReport`: Google's own conversion counts of the synced
/// actions for `from..=to`; `{ status: "unavailable" }` on any failure.
pub async fn conversion_report(
    db: &Db,
    up: &impl Upstream,
    credential: Option<&OAuthCredential>,
    workspace_id: i64,
    from: &str,
    to: &str,
) -> Result<Value> {
    let unavailable = json!({ "status": "unavailable" });
    let Some(setup) = get_setup(db, workspace_id).await? else {
        return Ok(unavailable);
    };
    let integration = &setup.integration;
    let Some(customer) = integration.conversion_customer_id.as_deref() else {
        return Ok(unavailable);
    };
    if setup.readiness != "ready" {
        return Ok(unavailable);
    }
    let Some(developer) = developer_token(credential) else {
        return Ok(unavailable);
    };
    let Ok(auth) = fresh_auth(db, up, integration, false).await else {
        return Ok(unavailable);
    };
    let creds = Creds {
        access_token: auth["tokens"]["accessToken"].as_str().unwrap_or_default(),
        developer_token: developer.as_deref(),
        login_customer_id: integration.login_customer_id.as_deref(),
    };
    let rows = match api::conversion_report(up, creds, customer, from, to).await {
        Ok(rows) => rows,
        Err(failure) => {
            tracing::warn!(workspace_id, error = %failure.public_message(&[]), "Google Ads report unavailable");
            return Ok(unavailable);
        }
    };
    let synced: Vec<String> = integration
        .conversion_actions
        .as_ref()
        .and_then(|actions| actions.0.as_array())
        .into_iter()
        .flatten()
        .filter_map(|a| api::id_text(&a["id"]))
        .collect();
    let mut by_action: Vec<Value> = Vec::new();
    for row in rows {
        let id = row["conversionActionId"].as_str().unwrap_or_default();
        if !synced.iter().any(|s| s == id) {
            continue;
        }
        let conversions = row["conversions"].as_f64().unwrap_or(0.0);
        match by_action.iter_mut().find(|a| a["conversionActionId"] == id) {
            Some(entry) => {
                let sum = entry["conversions"].as_f64().unwrap_or(0.0) + conversions;
                entry["conversions"] = json!(sum);
            }
            None => by_action.push(json!({
                "conversionActionId": id,
                "name": row["name"],
                "conversions": conversions,
            })),
        }
    }
    let total: f64 = by_action
        .iter()
        .map(|a| a["conversions"].as_f64().unwrap_or(0.0))
        .sum();
    Ok(json!({ "status": "ok", "total": total, "byAction": by_action }))
}

/// `refreshSetup`: re-resolves the conversion customer and re-reads its
/// offline-import conversion actions. Every failure lands in `setupError`
/// as a stable code; `None` = no Google Ads integration.
pub async fn refresh_setup(
    db: &Db,
    up: &impl Upstream,
    credential: Option<&OAuthCredential>,
    workspace_id: i64,
) -> Result<Option<Setup>> {
    let Some(setup) = get_setup(db, workspace_id).await? else {
        return Ok(None);
    };
    let integration = &setup.integration;
    let Some(developer) = developer_token(credential) else {
        record_setup_error(db, integration, "sync_failed").await?;
        return get_setup(db, workspace_id).await;
    };
    let mut synced = Err(Failure::Auth);
    for force in [false, true] {
        let auth = match fresh_auth(db, up, integration, force).await {
            Ok(auth) => auth,
            Err(_) => break,
        };
        synced = sync_from_google(up, &auth, integration, developer.as_deref()).await;
        if synced != Err(Failure::Auth) {
            break;
        }
    }
    match synced {
        Ok((conversion_customer, accepted, actions)) => {
            let (id, ws) = (integration.id, integration.workspace_id);
            let terms_refused = accepted == Some(false);
            tx::write(db, move |conn| {
                let now = now_ms();
                conn.execute(
                    r#"UPDATE "IntegrationGoogleAds" SET "conversionCustomerId" = ?3,
                           "acceptedCustomerDataTerms" = ?4, "conversionActions" = ?5,
                           "conversionActionsSyncedAt" = ?6, "setupError" = ?7, "setupErrorAt" = ?8,
                           "updatedAt" = ?6
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![
                        id,
                        ws,
                        conversion_customer,
                        accepted,
                        json!(actions).to_string(),
                        now,
                        terms_refused.then_some("customer_data_terms_not_accepted"),
                        terms_refused.then_some(now)
                    ],
                )
                .map_err(|e| BusinessError::Repo(e.into()))
            })
            .await?;
        }
        Err(failure) => {
            tracing::warn!(workspace_id, error = %failure.public_message(&[]), "google ads: setup sync failed");
            record_setup_error(db, integration, setup_error(&failure)).await?;
        }
    }
    get_setup(db, workspace_id).await
}

/// The provider-side half of `disconnect` (best effort): revokes the
/// grant. The error text is token-free.
pub async fn revoke(
    up: &impl Upstream,
    integration: &IntegrationGoogleAds,
) -> std::result::Result<(), String> {
    let auth = auth_of(integration);
    if auth.is_null() {
        return Err("Provider authentication was unavailable for teardown".into());
    }
    api::revoke(up, &auth).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatbotx_db::repo::Json;

    fn row(customer: Option<&str>, synced: Option<i64>) -> IntegrationGoogleAds {
        IntegrationGoogleAds {
            id: 1,
            created_at: 0,
            updated_at: 0,
            workspace_id: 1,
            integration_id: 2,
            auth: Json(json!({})),
            customer_id: "1234567890".into(),
            login_customer_id: None,
            descriptive_name: None,
            currency_code: None,
            conversion_customer_id: customer.map(str::to_string),
            accepted_customer_data_terms: None,
            conversion_actions: None,
            conversion_actions_synced_at: synced,
            setup_error: None,
            setup_error_at: None,
        }
    }

    #[test]
    fn readiness_needs_an_active_connection_and_a_sync() {
        assert_eq!(
            readiness(&row(Some("1"), Some(1)), Some("connected")),
            "ready"
        );
        assert_eq!(
            readiness(&row(Some("1"), None), Some("degraded")),
            "setup_incomplete"
        );
        assert_eq!(
            readiness(&row(Some("1"), Some(1)), Some("needs_reauth")),
            "needs_reauth"
        );
        assert_eq!(readiness(&row(None, None), None), "needs_reauth");
    }
}
