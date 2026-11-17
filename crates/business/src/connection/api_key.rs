//! The email-marketing `apiKeyConnection` providers
//! (`integrations/<name>/src/integration.ts`: `fields`, `buildAuth`, `probe`,
//! `isRevoked`), their `connectFromCredentials` and `verify`.

use base64::Engine as _;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::credentials::{
    SatelliteTarget, Upstream, UpstreamRequest, config_fields, credentials_rejected, parse_config,
    persist_key_connection, provider_unavailable_status, set_health,
};
use super::{ConnectionRow, repo_err};
use crate::{BusinessError, Result, tx};

/// An `apiKeyConnection` provider with an `auth`-only satellite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiKeyProvider {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub table: &'static str,
}

/// The email-marketing provider of `provider`.
pub fn api_key_provider(provider: &str) -> Option<ApiKeyProvider> {
    let (provider, display_name, table) = match provider {
        "activeCampaign" => (
            "activeCampaign",
            "ActiveCampaign",
            "IntegrationActiveCampaign",
        ),
        "drip" => ("drip", "Drip", "IntegrationDrip"),
        "getResponse" => ("getResponse", "GetResponse", "IntegrationGetResponse"),
        "klaviyo" => ("klaviyo", "Klaviyo", "IntegrationKlaviyo"),
        "mailchimp" => ("mailchimp", "Mailchimp", "IntegrationMailchimp"),
        "mailerLite" => ("mailerLite", "MailerLite", "IntegrationMailerLite"),
        "moosend" => ("moosend", "Moosend", "IntegrationMoosend"),
        "sendGrid" => ("sendGrid", "SendGrid", "IntegrationSendGrid"),
        _ => return None,
    };
    Some(ApiKeyProvider {
        provider,
        display_name,
        table,
    })
}

/// A failed `buildAuth` / `probe`: the HTTP status the original error
/// carries (`extractHttpStatus`), `None` = no status (a zod error, a
/// transport error).
type Failure = Option<u16>;

/// `toConnectionProviderError` + `providerFailureStatus`: a 4xx (not
/// 408 / 429) is a rejection, 408 / 429 is 503, anything else is 502.
fn failure_error(status: Failure) -> BusinessError {
    match status {
        Some(408 | 429) => provider_unavailable_status(503),
        Some(status) if (400..500).contains(&status) => credentials_rejected(),
        _ => provider_unavailable_status(502),
    }
}

fn text<'a>(config: &'a Map<String, Value>, key: &str) -> &'a str {
    config.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `normalizeActiveCampaignApiUrl`.
fn normalize_active_campaign_url(api_url: &str) -> String {
    let url = api_url.trim().trim_end_matches('/');
    let lower = url.to_ascii_lowercase();
    let cut = ["/api/3/", "/api/3"]
        .iter()
        .find(|suffix| lower.ends_with(*suffix))
        .map_or(0, |suffix| suffix.len());
    url[..url.len() - cut].to_string()
}

/// `buildAuth` (`create<Name>Auth`): the stored custom auth.
fn build_auth(
    provider: &ApiKeyProvider,
    config: &Map<String, Value>,
) -> std::result::Result<Value, Failure> {
    let key_name = if provider.provider == "drip" {
        "apiToken"
    } else {
        "apiKey"
    };
    let key = text(config, key_name).trim();
    if key.is_empty() {
        return Err(None);
    }
    Ok(match provider.provider {
        "activeCampaign" => {
            let api_url = text(config, "apiUrl").trim();
            url::Url::parse(api_url).map_err(|_| None)?;
            json!({ "authType": "custom", "apiUrl": normalize_active_campaign_url(api_url), "apiKey": key })
        }
        "drip" => json!({ "authType": "custom", "apiToken": key }),
        "mailchimp" => {
            // `extractDataCenter`: `MailchimpApiError` (400) for a bad key.
            let data_center = key.rsplit('-').next().unwrap_or_default();
            let valid = data_center.len() > 2
                && data_center[..2].eq_ignore_ascii_case("us")
                && data_center[2..].bytes().all(|b| b.is_ascii_digit());
            if !valid {
                return Err(Some(400));
            }
            json!({ "authType": "custom", "apiKey": key, "dataCenter": data_center.to_ascii_lowercase() })
        }
        _ => json!({ "authType": "custom", "apiKey": key }),
    })
}

fn get(url: String, headers: &[(&str, &str)]) -> UpstreamRequest {
    UpstreamRequest {
        method: "GET",
        url,
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        body: None,
    }
}

fn basic(credential: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(credential)
    )
}

/// The `probe` request of `provider`.
fn probe_request(provider: &ApiKeyProvider, auth: &Value) -> UpstreamRequest {
    let key = auth["apiKey"].as_str().unwrap_or_default();
    const ACCEPT: (&str, &str) = ("Accept", "application/json");
    match provider.provider {
        "activeCampaign" => get(
            format!(
                "{}/api/3/accounts",
                auth["apiUrl"].as_str().unwrap_or_default()
            ),
            &[ACCEPT, ("Api-Token", key)],
        ),
        "drip" => {
            let token = auth["apiToken"].as_str().unwrap_or_default();
            get(
                "https://api.getdrip.com/v2/accounts".into(),
                &[ACCEPT, ("Authorization", &basic(&format!("{token}:")))],
            )
        }
        "getResponse" => get(
            "https://api.getresponse.com/v3/accounts".into(),
            &[ACCEPT, ("X-Auth-Token", &format!("api-key {key}"))],
        ),
        "klaviyo" => get(
            "https://a.klaviyo.com/api/lists?page%5Bsize%5D=1".into(),
            &[
                ACCEPT,
                ("Authorization", &format!("Klaviyo-API-Key {key}")),
                ("revision", "2026-04-15"),
            ],
        ),
        "mailchimp" => get(
            format!(
                "https://{}.api.mailchimp.com/3.0/ping",
                auth["dataCenter"].as_str().unwrap_or_default()
            ),
            &[
                ACCEPT,
                ("Authorization", &basic(&format!("anystring:{key}"))),
            ],
        ),
        "mailerLite" => get(
            "https://connect.mailerlite.com/api/groups?page=1&limit=1".into(),
            &[ACCEPT, ("Authorization", &format!("Bearer {key}"))],
        ),
        "moosend" => {
            let mut url =
                url::Url::parse("https://api.moosend.com/v3/lists/1/1.json").expect("static url");
            url.query_pairs_mut().append_pair("apikey", key);
            get(url.into(), &[ACCEPT])
        }
        _ => get(
            "https://api.sendgrid.com/v3/scopes".into(),
            &[ACCEPT, ("Authorization", &format!("Bearer {key}"))],
        ),
    }
}

/// The `probe` check of an answer (the `<name>Request` status / envelope
/// checks and the probe's own body checks).
// ponytail: the zod response schemas are not ported; only the fields a
// probe decides on are checked.
fn probe_outcome(
    provider: &ApiKeyProvider,
    status: u16,
    body: &str,
) -> std::result::Result<(), Failure> {
    let ok = (200..300).contains(&status);
    let body: Option<Value> = serde_json::from_str(body).ok();
    match provider.provider {
        "moosend" => {
            let envelope = body.as_ref().filter(|b| b["Code"].is_i64());
            let failed = envelope.is_none_or(|b| {
                b["Code"].as_i64() != Some(0)
                    || b["Error"].as_str().is_some_and(|e| !e.is_empty())
                    || b["ErrorMessage"].as_str().is_some_and(|e| !e.is_empty())
            });
            if failed || status != 200 {
                return Err(Some(status));
            }
        }
        "getResponse" | "klaviyo" | "mailerLite" if status != 200 => return Err(Some(status)),
        _ if !ok => return Err(Some(status)),
        "drip" => {
            let accounts = body
                .as_ref()
                .and_then(|b| b["accounts"].as_array())
                .ok_or(None)?;
            if accounts.is_empty() {
                // `DripNoAccountError` (400).
                return Err(Some(400));
            }
        }
        "sendGrid" => {
            let scopes = body
                .as_ref()
                .and_then(|b| b["scopes"].as_array())
                .ok_or(None)?;
            let has_read = scopes.iter().any(|s| {
                matches!(
                    s.as_str(),
                    Some("marketing.read" | "marketing_campaigns.read")
                )
            });
            if !has_read {
                // `SendGridMissingScopesError` (400).
                return Err(Some(400));
            }
        }
        _ => {}
    }
    Ok(())
}

async fn probe(
    upstream: &impl Upstream,
    provider: &ApiKeyProvider,
    auth: &Value,
) -> std::result::Result<(), Failure> {
    match upstream.send(probe_request(provider, auth)).await {
        Ok((status, body)) => probe_outcome(provider, status, &body),
        Err(error) => {
            tracing::warn!(provider = provider.provider, error = %error, "connection credentials: provider validation failed");
            Err(None)
        }
    }
}

/// `connectFromCredentials` of an email-marketing provider.
pub async fn connect(
    db: &Db,
    upstream: &impl Upstream,
    workspace_id: i64,
    provider: ApiKeyProvider,
    config: &Map<String, Value>,
    actor_user_id: Option<i64>,
) -> Result<ConnectionRow> {
    let fields = config_fields(provider.provider);
    let parsed = parse_config(fields, config)?;
    let rejected: Vec<&str> = config
        .keys()
        .map(String::as_str)
        .filter(|key| !fields.iter().any(|field| field.name == *key))
        .collect();
    if let Some(first) = rejected.first() {
        return Err(BusinessError::validation(
            first,
            format!(
                "Unsupported config field(s) for {}: {}",
                provider.provider,
                rejected.join(", ")
            ),
        ));
    }
    let auth = build_auth(&provider, &parsed).map_err(failure_error)?;
    probe(upstream, &provider, &auth)
        .await
        .map_err(failure_error)?;
    let target = SatelliteTarget {
        provider: provider.provider,
        display_name: provider.display_name,
        table: provider.table,
        ai_defaults: None,
    };
    persist_key_connection(
        db,
        workspace_id,
        target,
        // `encryptUtils.encryptObject(auth)`: the satellite auth is sealed.
        crate::email_marketing::seal_auth(&auth)?,
        Vec::new(),
        actor_user_id,
        false,
    )
    .await
}

/// `connectionService.verify` (`probeVerify` + `isRevoked`) of an active
/// email-marketing connection.
pub async fn verify(
    db: &Db,
    upstream: &impl Upstream,
    provider: ApiKeyProvider,
    current: ConnectionRow,
) -> Result<ConnectionRow> {
    let (integration_id, workspace_id) = (current.integration_id, current.workspace_id);
    let auth: Option<String> = tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT "auth" FROM "{}" WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                provider.table
            ),
            params![integration_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let auth: Value = auth
        .and_then(|auth| crate::email_marketing::open_auth(&auth))
        .unwrap_or(Value::Null);
    let (status, reason) = match probe(upstream, &provider, &auth).await {
        Ok(()) => ("connected", None),
        // `isUnauthorizedStatusError` (401); moosend: `invalid_credentials`
        // (401 / 403).
        Err(Some(401)) => ("needs_reauth", Some("token_revoked")),
        Err(Some(403)) if provider.provider == "moosend" => ("needs_reauth", Some("token_revoked")),
        Err(_) => ("degraded", Some("verify_failed")),
    };
    set_health(db, current, status, reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ApiKeyProvider {
        api_key_provider(name).expect("provider")
    }

    fn config(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), json!(v)))
            .collect()
    }

    #[test]
    fn auth_and_probe_requests() {
        let auth = build_auth(
            &provider("activeCampaign"),
            &config(&[
                ("apiUrl", " https://acme.api-us1.com/api/3/ "),
                ("apiKey", " k "),
            ]),
        )
        .expect("auth");
        assert_eq!(
            auth,
            json!({ "authType": "custom", "apiUrl": "https://acme.api-us1.com", "apiKey": "k" })
        );
        assert_eq!(
            probe_request(&provider("activeCampaign"), &auth).url,
            "https://acme.api-us1.com/api/3/accounts"
        );
        let auth =
            build_auth(&provider("mailchimp"), &config(&[("apiKey", "abc-US21")])).expect("auth");
        assert_eq!(auth["dataCenter"], "us21");
        let request = probe_request(&provider("mailchimp"), &auth);
        assert_eq!(request.url, "https://us21.api.mailchimp.com/3.0/ping");
        assert_eq!(request.headers[1].1, basic("anystring:abc-US21"));
        assert_eq!(
            build_auth(&provider("mailchimp"), &config(&[("apiKey", "abc")])),
            Err(Some(400))
        );
        assert_eq!(
            build_auth(
                &provider("activeCampaign"),
                &config(&[("apiUrl", "nope"), ("apiKey", "k")])
            ),
            Err(None)
        );
        let auth = build_auth(&provider("moosend"), &config(&[("apiKey", "a&b")])).expect("auth");
        assert_eq!(
            probe_request(&provider("moosend"), &auth).url,
            "https://api.moosend.com/v3/lists/1/1.json?apikey=a%26b"
        );
    }

    #[test]
    fn probe_outcomes_and_failures() {
        let drip = provider("drip");
        assert_eq!(
            probe_outcome(&drip, 200, r#"{"accounts":[{"id":"1"}]}"#),
            Ok(())
        );
        assert_eq!(
            probe_outcome(&drip, 200, r#"{"accounts":[]}"#),
            Err(Some(400))
        );
        assert_eq!(probe_outcome(&drip, 401, ""), Err(Some(401)));
        let sendgrid = provider("sendGrid");
        assert_eq!(
            probe_outcome(&sendgrid, 200, r#"{"scopes":["marketing_campaigns.read"]}"#),
            Ok(())
        );
        assert_eq!(
            probe_outcome(&sendgrid, 200, r#"{"scopes":["mail.send"]}"#),
            Err(Some(400))
        );
        let moosend = provider("moosend");
        assert_eq!(
            probe_outcome(&moosend, 200, r#"{"Code":0,"Context":{}}"#),
            Ok(())
        );
        assert_eq!(
            probe_outcome(&moosend, 200, r#"{"Code":1,"Error":"X"}"#),
            Err(Some(200))
        );
        assert_eq!(probe_outcome(&provider("klaviyo"), 204, ""), Err(Some(204)));
        let code = |status| match failure_error(status) {
            BusinessError::Exception { code, status, .. } => (code, status),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            code(Some(401)),
            ("connectionCredentialsRejected".into(), 400)
        );
        assert_eq!(
            code(Some(429)),
            ("connectionProviderUnavailable".into(), 503)
        );
        assert_eq!(
            code(Some(500)),
            ("connectionProviderUnavailable".into(), 502)
        );
        assert_eq!(
            code(Some(200)),
            ("connectionProviderUnavailable".into(), 502)
        );
        assert_eq!(code(None), ("connectionProviderUnavailable".into(), 502));
    }
}
