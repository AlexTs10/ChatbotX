//! The Google Ads `connection` adapter (`integrations/google-ads/src/
//! integration.ts`, `connection-auth.ts`, `candidates.ts`) and its store
//! (`IntegrationGoogleAds`, a workspace singleton): multi-account, the
//! grant's accessible customers are the candidates.

use chatbotx_db::repo::{iso, new_id};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use super::credentials::Upstream;
use super::oauth::{ProviderFailure, oauth2_auth, query_url, text, unique_or_repo};
use super::{ConnectionRow, find_or_fail, repo_err};
use crate::google_ads_api::{self as api, Failure};
use crate::{BusinessError, Result};

pub const PROVIDER: &str = "googleAds";

/// `authorizeUrl` (`generateAuthUrl`, the credential's method's scopes).
pub fn authorize_url(config: &Value, callback_url: &str, state: &str) -> String {
    let method = api::upload_method(&config["uploadMethod"]);
    query_url(
        api::AUTH_URL,
        &[
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("scope", &api::authorize_scopes(method)),
            ("state", state),
            ("response_type", "code"),
            ("client_id", text(config, "clientId")),
            ("redirect_uri", callback_url),
        ],
    )
}

fn rejected(message: impl Into<String>) -> ProviderFailure {
    ProviderFailure::Rejected(message.into())
}

/// `exchangeCode` + `addGoogleAdsIdentity`: the tokens, the Google account,
/// the method the grant was made for and (session only) the developer token.
pub async fn exchange_code(
    up: &impl Upstream,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    let tokens = api::token_request(
        up,
        &[
            ("code", code),
            ("client_id", text(config, "clientId")),
            ("client_secret", text(config, "clientSecret")),
            ("redirect_uri", callback_url),
            ("grant_type", "authorization_code"),
        ],
    )
    .await
    .map_err(|(revoked, message)| {
        if revoked {
            rejected(message)
        } else {
            ProviderFailure::Unavailable(503)
        }
    })?;
    let access = tokens["access_token"].as_str().unwrap_or_default();
    if access.is_empty() {
        return Err(rejected("Google OAuth response has no access token"));
    }
    let Some(refresh) = tokens["refresh_token"].as_str() else {
        return Err(rejected("Google OAuth response has no refresh token"));
    };
    let info = api::token_info(up, access).await.map_err(|f| match f {
        Failure::Network(_) => ProviderFailure::Unavailable(503),
        Failure::Google(e) if e.retryable => ProviderFailure::Unavailable(502),
        _ => rejected("Google Ads credentials were rejected"),
    })?;
    let granted_scope = tokens["scope"].as_str().filter(|s| !s.is_empty());
    let granted = match granted_scope {
        Some(scope) => api::parse_scopes(scope),
        None => api::parse_scopes(info["scope"].as_str().unwrap_or_default()),
    };
    let method = api::method_of_grant(api::upload_method(&config["uploadMethod"]), &granted);
    if !api::missing_scopes(&granted, method).is_empty() {
        return Err(rejected(api::scope_missing_message(method)));
    }
    let account = info["sub"]
        .as_str()
        .or_else(|| info["user_id"].as_str())
        .ok_or_else(|| rejected("Google Ads token info has no stable account id"))?;
    let mut metadata = json!({
        "scope": granted_scope.map_or_else(|| granted.join(" "), str::to_string),
        "accountId": account,
        "email": info["email"],
        "uploadMethod": method,
    });
    if let Some(token) = config["developerToken"].as_str().filter(|t| !t.is_empty()) {
        metadata["developerToken"] = json!(token);
    }
    let expires_at = tokens["expires_in"]
        .as_i64()
        .map(|s| iso(chatbotx_db::repo::now_ms() + s * 1000));
    Ok(oauth2_auth(
        config,
        callback_url,
        json!({ "accessToken": access, "expiresAt": expires_at, "refreshToken": refresh }),
        Some(metadata),
    ))
}

/// `listCandidates`: one candidate per account, its auth carrying the
/// customer and never the developer token (`stripDeveloperToken`).
pub async fn list_candidates(
    up: &impl Upstream,
    auth: &Value,
) -> std::result::Result<Vec<Value>, ProviderFailure> {
    let access = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let developer = auth["metadata"]["developerToken"].as_str();
    let accounts = match api::collect_candidates(up, access, developer).await {
        Ok(accounts) => accounts,
        Err(failure) => {
            return match api::classify(&failure) {
                Some("no_ads_account") => Ok(Vec::new()),
                Some(cause) => Err(rejected(api::failure_message(cause))),
                None if failure.retryable() => Err(ProviderFailure::Unavailable(503)),
                None => Err(rejected("Failed to list Google Ads accounts")),
            };
        }
    };
    let mut base = auth.clone();
    if let Some(metadata) = base["metadata"].as_object_mut() {
        metadata.remove("developerToken");
    }
    Ok(accounts
        .into_iter()
        .map(|account| {
            let mut candidate_auth = base.clone();
            let metadata = &mut candidate_auth["metadata"];
            metadata["customerId"] = json!(account.customer_id);
            metadata["loginCustomerId"] = json!(account.login_customer_id);
            if let Some(name) = &account.descriptive_name {
                metadata["descriptiveName"] = json!(name);
            }
            if let Some(code) = &account.currency_code {
                metadata["currencyCode"] = json!(code);
            }
            json!({
                "sourceId": account.customer_id,
                "displayName": format!(
                    "{} ({})",
                    account.descriptive_name.as_deref().unwrap_or("Google Ads"),
                    api::format_customer_id(&account.customer_id)
                ),
                "auth": candidate_auth,
            })
        })
        .collect())
}

/// `describe`: the customer id and display name of a candidate auth.
pub fn describe(auth: &Value) -> Result<(String, String)> {
    let customer = text(&auth["metadata"], "customerId");
    if customer.is_empty() {
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
            "Google Ads auth has no customer identity",
        )));
    }
    let name = auth["metadata"]["descriptiveName"]
        .as_str()
        .or_else(|| auth["metadata"]["email"].as_str())
        .unwrap_or("Google Ads");
    Ok((customer.to_string(), name.to_string()))
}

fn already_connected() -> BusinessError {
    super::credentials::already_connected()
}

/// `connectAndPersist` for a picked customer. The workspace holds one Google
/// Ads account: an active one refuses (`connectionAlreadyConnected`); an
/// inactive one is replaced in place (its satellite row and `Connection`
/// take the new customer), so the event history keeps its integration.
pub fn persist(
    conn: &Connection,
    workspace_id: i64,
    auth: &Value,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let (customer, name) = describe(auth)?;
    let metadata = &auth["metadata"];
    let sealed = crate::email_marketing::seal_auth(auth)?;
    let same_source: Option<(i64, String)> = conn
        .query_row(
            r#"SELECT "id", "status" FROM "Connection"
               WHERE "workspaceId" = ?1 AND "provider" = 'googleAds' AND "sourceId" = ?2"#,
            params![workspace_id, customer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo_err)?;
    if same_source
        .as_ref()
        .is_some_and(|(_, s)| super::is_active(s))
    {
        return Err(already_connected());
    }
    let satellite: Option<(i64, String)> = conn
        .query_row(
            r#"SELECT "integrationId", "customerId" FROM "IntegrationGoogleAds" WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo_err)?;
    let integration_id = match satellite {
        Some((integration_id, old_customer)) => {
            let holder: Option<(i64, String)> = conn
                .query_row(
                    r#"SELECT "id", "status" FROM "Connection" WHERE "integrationId" = ?1"#,
                    [integration_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(repo_err)?;
            if holder.as_ref().is_some_and(|(_, s)| super::is_active(s)) {
                return Err(already_connected());
            }
            // Another customer's setup does not carry over.
            let reset = old_customer != customer;
            conn.execute(
                r#"UPDATE "IntegrationGoogleAds" SET "auth" = ?2, "customerId" = ?3, "loginCustomerId" = ?4,
                       "descriptiveName" = ?5, "currencyCode" = ?6, "updatedAt" = ?7,
                       "conversionCustomerId" = CASE WHEN ?8 THEN NULL ELSE "conversionCustomerId" END,
                       "acceptedCustomerDataTerms" = CASE WHEN ?8 THEN NULL ELSE "acceptedCustomerDataTerms" END,
                       "conversionActions" = CASE WHEN ?8 THEN NULL ELSE "conversionActions" END,
                       "conversionActionsSyncedAt" = CASE WHEN ?8 THEN NULL ELSE "conversionActionsSyncedAt" END,
                       "setupError" = CASE WHEN ?8 THEN NULL ELSE "setupError" END,
                       "setupErrorAt" = CASE WHEN ?8 THEN NULL ELSE "setupErrorAt" END
                   WHERE "integrationId" = ?1"#,
                params![
                    integration_id,
                    sealed,
                    customer,
                    metadata["loginCustomerId"].as_str(),
                    metadata["descriptiveName"].as_str(),
                    metadata["currencyCode"].as_str(),
                    now,
                    reset
                ],
            )
            .map_err(repo_err)?;
            // The integration's connection moves to the row of this customer.
            if let (Some((holder_id, _)), Some((same_id, _))) = (&holder, &same_source)
                && holder_id != same_id
            {
                conn.execute(
                    r#"UPDATE "Connection" SET "integrationId" = NULL, "updatedAt" = ?2 WHERE "id" = ?1"#,
                    params![holder_id, now],
                )
                .map_err(repo_err)?;
            }
            integration_id
        }
        None => {
            let integration_id = new_id();
            conn.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'googleAds')"#,
                params![integration_id, workspace_id],
            )
            .map_err(repo_err)?;
            conn.execute(
                r#"INSERT INTO "IntegrationGoogleAds" ("id", "createdAt", "updatedAt", "workspaceId",
                       "integrationId", "auth", "customerId", "loginCustomerId", "descriptiveName", "currencyCode")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
                params![
                    new_id(),
                    now,
                    workspace_id,
                    integration_id,
                    sealed,
                    customer,
                    metadata["loginCustomerId"].as_str(),
                    metadata["descriptiveName"].as_str(),
                    metadata["currencyCode"].as_str()
                ],
            )
            .map_err(unique_or_repo)?;
            integration_id
        }
    };
    let target: Option<i64> = match same_source {
        Some((id, _)) => Some(id),
        None => conn
            .query_row(
                r#"SELECT "id" FROM "Connection" WHERE "integrationId" = ?1"#,
                [integration_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?,
    };
    let connection_id = match target {
        Some(id) => {
            conn.execute(
                r#"UPDATE "Connection" SET "sourceId" = ?3, "integrationId" = ?4, "displayName" = ?5,
                       "lastError" = NULL, "status" = 'connected', "statusReason" = NULL,
                       "disconnectedAt" = NULL, "connectedAt" = ?6, "updatedAt" = ?6
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id, customer, integration_id, name, now],
            )
            .map_err(unique_or_repo)?;
            id
        }
        None => {
            let id = new_id();
            conn.execute(
                r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                       "kind", "integrationId", "sourceId", "displayName", "status", "createdBy", "connectedAt")
                   VALUES (?1, ?2, ?2, ?3, 'googleAds', 'integration', ?4, ?5, ?6, 'connected', ?7, ?2)"#,
                params![id, now, workspace_id, integration_id, customer, name, actor_user_id],
            )
            .map_err(unique_or_repo)?;
            id
        }
    };
    find_or_fail(conn, connection_id, workspace_id)
}

/// The reconnect write: the integration's satellite takes the re-granted
/// auth. `false` = no satellite row.
pub fn save_reconnect_auth(
    conn: &Connection,
    integration_id: Option<i64>,
    workspace_id: i64,
    auth: &Value,
    now: i64,
) -> Result<bool> {
    let sealed = crate::email_marketing::seal_auth(auth)?;
    let updated = conn
        .execute(
            r#"UPDATE "IntegrationGoogleAds" SET "auth" = ?3, "updatedAt" = ?4
               WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
            params![integration_id, workspace_id, sealed, now],
        )
        .map_err(repo_err)?;
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_asks_the_method_scopes() {
        let config = json!({ "clientId": "cid", "uploadMethod": "legacy" });
        let url = url::Url::parse(&authorize_url(&config, "https://app/cb", "1.n")).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        assert_eq!(pairs["scope"], api::authorize_scopes("legacy"));
        assert_eq!(pairs["state"], "1.n");
        assert_eq!(pairs["access_type"], "offline");
        assert_eq!(pairs["redirect_uri"], "https://app/cb");
    }

    #[test]
    fn describe_needs_a_customer() {
        let auth = json!({ "metadata": { "customerId": "1234567890", "email": "a@b.c" } });
        assert_eq!(
            describe(&auth).unwrap(),
            ("1234567890".to_string(), "a@b.c".to_string())
        );
        assert!(describe(&json!({ "metadata": {} })).is_err());
    }
}
