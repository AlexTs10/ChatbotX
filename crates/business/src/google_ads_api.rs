//! The Google Ads REST calls of `integrations/google-ads` (`apis/google-ads.ts`,
//! `candidates.ts`, `exception.ts`, `lib/failure-cause.ts`, `lib/scopes.ts`,
//! `lib/redact.ts`) and the Google OAuth token calls (`google-auth-library`).
//! Every call goes through [`Upstream`] (no retries: the caller owns them).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::connection::credentials::{Upstream, UpstreamRequest};

pub const ADS_SCOPE: &str = "https://www.googleapis.com/auth/adwords";
pub const DATA_MANAGER_SCOPE: &str = "https://www.googleapis.com/auth/datamanager";
pub const ADS_API: &str = "https://googleads.googleapis.com/v25/";
pub const DATA_MANAGER_API: &str = "https://datamanager.googleapis.com/v1/";
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const TOKENINFO_URL: &str = "https://oauth2.googleapis.com/tokeninfo";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

const NOT_ADS_USER: &str = "NOT_ADS_USER";
const MAX_MESSAGE: usize = 1000;
const MAX_FIELD_VIOLATIONS: usize = 5;
const INACTIVE_CUSTOMER_REASONS: &[&str] = &["CUSTOMER_NOT_ENABLED", "CUSTOMER_NOT_FOUND"];
pub const REDACTED: &str = "[redacted]";

// ---- scopes / upload method -------------------------------------------------

/// `parseUploadMethod`: anything but `legacy` is Data Manager.
pub fn upload_method(value: &Value) -> &'static str {
    if value.as_str() == Some("legacy") {
        "legacy"
    } else {
        "dataManager"
    }
}

/// `uploadMethodOf(auth)`.
pub fn upload_method_of(auth: &Value) -> &'static str {
    upload_method(&auth["metadata"]["uploadMethod"])
}

fn required_scopes(method: &str) -> &'static [&'static str] {
    if method == "legacy" {
        &[ADS_SCOPE]
    } else {
        &[ADS_SCOPE, DATA_MANAGER_SCOPE]
    }
}

/// `authorizeScopesFor`: required + identity scopes, space separated.
pub fn authorize_scopes(method: &str) -> String {
    let mut scopes = required_scopes(method).to_vec();
    scopes.extend(["openid", "email"]);
    scopes.join(" ")
}

pub fn parse_scopes(scope: &str) -> Vec<String> {
    scope.split_whitespace().map(str::to_string).collect()
}

/// `missingRequiredScopes`.
pub fn missing_scopes(granted: &[String], method: &str) -> Vec<&'static str> {
    required_scopes(method)
        .iter()
        .copied()
        .filter(|scope| !granted.iter().any(|g| g == scope))
        .collect()
}

/// `methodOfGrant`: a Data Manager credential is never downgraded.
pub fn method_of_grant(credential_method: &str, granted: &[String]) -> &'static str {
    if credential_method == "dataManager" || granted.iter().any(|g| g == DATA_MANAGER_SCOPE) {
        "dataManager"
    } else {
        "legacy"
    }
}

/// `hasRequiredScopes`: a row without a recorded scope passes.
pub fn has_required_scopes(auth: &Value) -> bool {
    match auth["metadata"]["scope"].as_str().filter(|s| !s.is_empty()) {
        None => true,
        Some(scope) => missing_scopes(&parse_scopes(scope), upload_method_of(auth)).is_empty(),
    }
}

/// `SCOPE_MISSING_MESSAGES`.
pub fn scope_missing_message(method: &str) -> &'static str {
    if method == "legacy" {
        "Google Ads permission was not granted"
    } else {
        "Google Ads and Data Manager permissions were not both granted"
    }
}

// ---- errors -------------------------------------------------------------------

/// A Google answer that is an error (`GoogleAdsException`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoogleError {
    pub status: u16,
    pub api_status: Option<String>,
    pub reason: Option<String>,
    /// The `errorCode` family of `reason` (`authorizationError`).
    pub category: Option<String>,
    /// Google's text (may echo ids: [`sanitize`] it before it leaves).
    pub message: String,
    pub retryable: bool,
}

impl GoogleError {
    /// A terminal local refusal (a malformed id or answer).
    pub fn terminal(status: u16, reason: &str, message: &str) -> Self {
        GoogleError {
            status,
            reason: Some(reason.into()),
            message: message.into(),
            ..GoogleError::default()
        }
    }
}

/// A failed Google call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 401 (not `NOT_ADS_USER`): `AuthException`, the token is dead.
    Auth,
    Google(GoogleError),
    /// No answer (timeout, reset): transient.
    Network(String),
}

impl Failure {
    pub fn retryable(&self) -> bool {
        match self {
            Failure::Auth => false,
            Failure::Google(e) => e.retryable,
            Failure::Network(_) => true,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Failure::Google(e) => e.reason.as_deref(),
            _ => None,
        }
    }

    /// `sanitizeGoogleAdsError(error, { secrets }).message`.
    pub fn public_message(&self, secrets: &[&str]) -> String {
        match self {
            Failure::Auth => "Google Ads credentials were rejected".into(),
            Failure::Google(e) => sanitize(&e.message, secrets),
            Failure::Network(text) => sanitize(text, secrets),
        }
    }
}

/// `isRetryableGoogleAdsStatus`.
pub fn retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

fn violation(v: &Value) -> Option<String> {
    let base = [v["field"].as_str(), v["description"].as_str()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(": ");
    let text = match v["reason"].as_str() {
        Some(reason) => format!("{base} ({reason})").trim().to_string(),
        None => base,
    };
    (!text.is_empty()).then_some(text)
}

/// `parseGoogleAdsOriginError` of an HTTP error answer.
pub fn parse_error(status: u16, body: &str) -> GoogleError {
    let data: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let data = match &data {
        Value::Array(items) => items
            .iter()
            .find(|i| i.get("error").is_some())
            .cloned()
            .unwrap_or(Value::Null),
        other => other.clone(),
    };
    let error = &data["error"];
    let details = error["details"].as_array().cloned().unwrap_or_default();
    let mut reason = details
        .iter()
        .find_map(|d| d["reason"].as_str())
        .map(|r| (r.to_string(), None));
    if reason.is_none() {
        reason = details
            .iter()
            .flat_map(|d| d["errors"].as_array().cloned().unwrap_or_default())
            .find_map(|item| {
                item["errorCode"].as_object().and_then(|codes| {
                    codes
                        .iter()
                        .find_map(|(k, v)| v.as_str().map(|v| (v.to_string(), Some(k.clone()))))
                })
            });
    }
    let mut parts: Vec<String> = error["message"]
        .as_str()
        .map(str::to_string)
        .into_iter()
        .collect();
    parts.extend(details.iter().flat_map(|d| {
        d["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["message"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
    }));
    parts.extend(
        details
            .iter()
            .flat_map(|d| d["fieldViolations"].as_array().cloned().unwrap_or_default())
            .filter_map(|v| violation(&v))
            .take(MAX_FIELD_VIOLATIONS),
    );
    let message = parts.join(" | ");
    GoogleError {
        status,
        api_status: error["status"].as_str().map(str::to_string),
        category: reason.as_ref().and_then(|(_, c)| c.clone()),
        reason: reason.map(|(r, _)| r),
        message: if message.is_empty() {
            "Google Ads API call failed".into()
        } else {
            message
        },
        retryable: retryable_status(status),
    }
}

/// `rescueGoogleAds` over one answer: a 2xx JSON body, else the typed failure.
pub fn rescue(reply: Result<(u16, String), String>) -> Result<Value, Failure> {
    let (status, body) = reply.map_err(Failure::Network)?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&body).map_err(|_| {
            Failure::Google(GoogleError::terminal(
                status,
                "malformedResponse",
                "Google Ads response could not be read",
            ))
        });
    }
    let error = parse_error(status, &body);
    if status == 401 && error.reason.as_deref() != Some(NOT_ADS_USER) {
        return Err(Failure::Auth);
    }
    Err(Failure::Google(error))
}

static SECRET_PATTERNS: LazyLock<Vec<(Regex, bool)>> = LazyLock::new(|| {
    [
        (r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----", false),
        (r"(?i)Bearer\s+[A-Za-z0-9._~+/=-]+", false),
        (r"\bya29\.[A-Za-z0-9._-]+", false),
        (r"\b1//[A-Za-z0-9._-]{20,}", false),
        (r#"(?i)("?(?:gclid|gbraid|wbraid|access_token|refresh_token|developer-token)"?\s*[:=]\s*"?)[^"\s,}&]+"#, true),
        (r"(?i)\b[0-9a-f]{64}\b", false),
        (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", false),
        (r"\+\d{7,15}\b", false),
    ]
    .into_iter()
    .map(|(p, prefixed)| (Regex::new(p).expect("static pattern"), prefixed))
    .collect()
});

/// `redactSecrets` + the 1000-character cut of `sanitizeGoogleAdsError`.
// ponytail: the original's look-behinds are dropped (Rust `regex` has none),
// so a field path like `adIdentifiers.gclid: x` also loses its value.
pub fn sanitize(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        out = out.replace(secret, REDACTED);
    }
    for (pattern, prefixed) in SECRET_PATTERNS.iter() {
        out = if *prefixed {
            pattern
                .replace_all(&out, format!("${{1}}{REDACTED}"))
                .into_owned()
        } else {
            pattern.replace_all(&out, REDACTED).into_owned()
        };
    }
    out.chars().take(MAX_MESSAGE).collect()
}

/// `classifyGoogleAdsFailure`: the stable connect-failure cause.
pub fn classify(failure: &Failure) -> Option<&'static str> {
    let error = match failure {
        Failure::Auth => return Some("credentials_invalid"),
        Failure::Google(e) => e,
        Failure::Network(_) => return None,
    };
    let reason = error.reason.as_deref().unwrap_or_default();
    if reason == NOT_ALLOWLISTED {
        return Some("legacy_upload_not_allowed");
    }
    if reason == NOT_ADS_USER {
        return Some("no_ads_account");
    }
    if reason == "DEVELOPER_TOKEN_PARAMETER_MISSING" {
        return Some("developer_token_missing");
    }
    if reason.starts_with("DEVELOPER_TOKEN") {
        return Some("developer_token_not_approved");
    }
    if reason == "CLOUD_PROJECT_NOT_APPROVED_FOR_PRODUCTION"
        || (reason == "ACTION_NOT_PERMITTED"
            && error.category.as_deref() == Some("authorizationError"))
    {
        return Some("project_not_approved");
    }
    if matches!(reason, "USER_PERMISSION_DENIED" | "CUSTOMER_NOT_ENABLED") {
        return Some("permission_denied");
    }
    let lower = error.message.to_lowercase();
    if matches!(reason, "SERVICE_DISABLED" | "API_NOT_ENABLED")
        || lower.contains("has not been used in project")
        || lower.contains("is disabled")
        || lower.contains("api has not been used")
        || lower.contains("api has not been enabled")
    {
        return Some("api_not_enabled");
    }
    if matches!(
        reason,
        "OAUTH_TOKEN_INVALID"
            | "OAUTH_TOKEN_EXPIRED"
            | "OAUTH_TOKEN_REVOKED"
            | "OAUTH_TOKEN_HEADER_INVALID"
            | "CLIENT_CUSTOMER_ID_INVALID"
    ) || error.api_status.as_deref() == Some("UNAUTHENTICATED")
        || error.status == 401
    {
        return Some("credentials_invalid");
    }
    (error.status == 403).then_some("permission_denied")
}

/// `GOOGLE_ADS_FAILURE_MESSAGES`.
pub fn failure_message(cause: &str) -> &'static str {
    match cause {
        "developer_token_missing" => "Google Ads developer token is not configured",
        "developer_token_not_approved" => "Google rejected the Google Ads developer token",
        "project_not_approved" => {
            "The Google Cloud project is not approved for production Google Ads accounts"
        }
        "permission_denied" => {
            "The Google user does not have permission to access Google Ads accounts"
        }
        "api_not_enabled" => "The Google Ads API is not enabled for this project",
        "credentials_invalid" => "Google Ads credentials were rejected",
        "scope_missing" => "Google Ads and Data Manager permissions were not both granted",
        "legacy_upload_not_allowed" => {
            "Google does not allow the legacy upload API for this account: switch the credential to Data Manager and reconnect this workspace"
        }
        _ => "Google Ads API call failed",
    }
}

/// `CUSTOMER_NOT_ALLOWLISTED_FOR_THIS_FEATURE`.
pub const NOT_ALLOWLISTED: &str = "CUSTOMER_NOT_ALLOWLISTED_FOR_THIS_FEATURE";

// ---- Google Ads API -------------------------------------------------------------

/// `GoogleAdsCredentials`.
#[derive(Debug, Clone, Copy)]
pub struct Creds<'a> {
    pub access_token: &'a str,
    pub developer_token: Option<&'a str>,
    pub login_customer_id: Option<&'a str>,
}

/// `adsHeaders`.
pub fn ads_headers(creds: Creds<'_>) -> Vec<(String, String)> {
    let mut headers = vec![(
        "authorization".to_string(),
        format!("Bearer {}", creds.access_token),
    )];
    if let Some(token) = creds.developer_token.filter(|t| !t.is_empty()) {
        headers.push(("developer-token".into(), token.into()));
    }
    if let Some(login) = creds.login_customer_id.filter(|t| !t.is_empty()) {
        headers.push(("login-customer-id".into(), login.into()));
    }
    headers
}

/// `customerIdSchema` (10 digits): ids reach URLs and headers.
pub fn checked_id<'a>(id: &'a str, label: &str) -> Result<&'a str, Failure> {
    if id.len() == 10 && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(Failure::Google(GoogleError::terminal(
            400,
            "invalidIdentifier",
            &format!("Google Ads {label} is not a valid id"),
        )))
    }
}

/// `formatCustomerId`: `123-456-7890`.
pub fn format_customer_id(id: &str) -> String {
    if id.len() == 10 {
        format!("{}-{}-{}", &id[..3], &id[3..6], &id[6..])
    } else {
        id.to_string()
    }
}

/// An int64 in proto3 JSON (a string, sometimes a number).
pub fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

async fn search_stream(
    up: &impl Upstream,
    creds: Creds<'_>,
    customer_id: &str,
    query: &str,
) -> Result<Vec<Value>, Failure> {
    let id = checked_id(customer_id, "customer id")?;
    let body = rescue(
        up.send(UpstreamRequest {
            method: "POST",
            url: format!("{ADS_API}customers/{id}/googleAds:searchStream"),
            headers: ads_headers(creds),
            body: Some(json!({ "query": query }).to_string()),
        })
        .await,
    )?;
    Ok(body
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|batch| batch["results"].as_array().cloned().unwrap_or_default())
        .collect())
}

/// `listAccessibleCustomers`.
pub async fn list_accessible_customers(
    up: &impl Upstream,
    access_token: &str,
    developer_token: Option<&str>,
) -> Result<Vec<String>, Failure> {
    let body = rescue(
        up.send(UpstreamRequest {
            method: "GET",
            url: format!("{ADS_API}customers:listAccessibleCustomers"),
            headers: ads_headers(Creds {
                access_token,
                developer_token,
                login_customer_id: None,
            }),
            body: None,
        })
        .await,
    )?;
    Ok(body["resourceNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| name.trim_start_matches("customers/").to_string())
        .collect())
}

/// `GoogleAdsCustomer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub id: String,
    pub descriptive_name: Option<String>,
    pub manager: bool,
    pub status: Option<String>,
    pub currency_code: Option<String>,
    pub conversion_customer_id: Option<String>,
    /// `None` = unknown (no tracking setting).
    pub accepted_customer_data_terms: Option<bool>,
}

const CUSTOMER_QUERY: &str = "SELECT customer.id, customer.descriptive_name, customer.manager, customer.status, customer.currency_code, customer.conversion_tracking_setting.google_ads_conversion_customer, customer.conversion_tracking_setting.accepted_customer_data_terms, customer.conversion_tracking_setting.conversion_tracking_status FROM customer LIMIT 1";
const CLIENT_CUSTOMERS_QUERY: &str = "SELECT customer_client.id, customer_client.descriptive_name, customer_client.currency_code, customer_client.manager, customer_client.status FROM customer_client WHERE customer_client.manager = false AND customer_client.status = 'ENABLED'";
const CONVERSION_ACTIONS_QUERY: &str = "SELECT conversion_action.id, conversion_action.resource_name, conversion_action.name, conversion_action.category, conversion_action.status, conversion_action.counting_type, conversion_action.click_through_lookback_window_days, conversion_action.attribution_model_settings.attribution_model FROM conversion_action WHERE conversion_action.type = 'UPLOAD_CLICKS'";

fn opt_text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

/// `getCustomer`.
pub async fn get_customer(
    up: &impl Upstream,
    creds: Creds<'_>,
    customer_id: &str,
) -> Result<Option<Customer>, Failure> {
    let results = search_stream(up, creds, customer_id, CUSTOMER_QUERY).await?;
    let Some(customer) = results
        .first()
        .map(|r| &r["customer"])
        .filter(|c| c.is_object())
    else {
        return Ok(None);
    };
    let setting = &customer["conversionTrackingSetting"];
    Ok(Some(Customer {
        id: id_text(&customer["id"]).unwrap_or_default(),
        descriptive_name: opt_text(&customer["descriptiveName"]),
        manager: customer["manager"].as_bool().unwrap_or(false),
        status: opt_text(&customer["status"]),
        currency_code: opt_text(&customer["currencyCode"]),
        conversion_customer_id: setting["googleAdsConversionCustomer"]
            .as_str()
            .map(|r| r.trim_start_matches("customers/").to_string()),
        accepted_customer_data_terms: setting.is_object().then(|| {
            setting["acceptedCustomerDataTerms"]
                .as_bool()
                .unwrap_or(false)
        }),
    }))
}

/// `listClientCustomers` (through the manager).
async fn list_client_customers(
    up: &impl Upstream,
    access_token: &str,
    developer_token: Option<&str>,
    manager_id: &str,
) -> Result<Vec<Customer>, Failure> {
    let creds = Creds {
        access_token,
        developer_token,
        login_customer_id: Some(manager_id),
    };
    let results = search_stream(up, creds, manager_id, CLIENT_CUSTOMERS_QUERY).await?;
    Ok(results
        .iter()
        .map(|r| &r["customerClient"])
        .filter(|c| c.is_object())
        .map(|c| Customer {
            id: id_text(&c["id"]).unwrap_or_default(),
            descriptive_name: opt_text(&c["descriptiveName"]),
            manager: false,
            status: None,
            currency_code: opt_text(&c["currencyCode"]),
            conversion_customer_id: None,
            accepted_customer_data_terms: None,
        })
        .collect())
}

/// `listUploadClickConversionActions`: the cache entries.
pub async fn list_conversion_actions(
    up: &impl Upstream,
    creds: Creds<'_>,
    conversion_customer_id: &str,
) -> Result<Vec<Value>, Failure> {
    let results =
        search_stream(up, creds, conversion_customer_id, CONVERSION_ACTIONS_QUERY).await?;
    Ok(results
        .iter()
        .map(|r| &r["conversionAction"])
        .filter(|a| a.is_object())
        .map(|a| {
            let lookback = match &a["clickThroughLookbackWindowDays"] {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            };
            json!({
                "id": id_text(&a["id"]),
                "resourceName": a["resourceName"],
                "name": a["name"],
                "category": a["category"].as_str().unwrap_or("DEFAULT"),
                "status": a["status"].as_str().unwrap_or("UNKNOWN"),
                "countingType": a["countingType"].as_str().unwrap_or("UNKNOWN"),
                "clickThroughLookbackWindowDays": lookback.filter(|n| n.is_finite()),
                "attributionModel": a["attributionModelSettings"]["attributionModel"],
            })
        })
        .collect())
}

/// One connectable account (`CandidateAccount`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub customer_id: String,
    pub login_customer_id: Option<String>,
    pub descriptive_name: Option<String>,
    pub currency_code: Option<String>,
}

fn project_not_approved(failure: &Failure) -> bool {
    classify(failure) == Some("project_not_approved")
}

/// `collectCandidateAccounts`: the enabled non-manager accounts reachable
/// directly, plus the clients of each reachable manager. One unreadable
/// account never hides the others; nothing found while something failed
/// for the project (or transiently) is that failure.
// ponytail: lookups run one by one (the original batches 5 in parallel).
pub async fn collect_candidates(
    up: &impl Upstream,
    access_token: &str,
    developer_token: Option<&str>,
) -> Result<Vec<Account>, Failure> {
    let creds = Creds {
        access_token,
        developer_token,
        login_customer_id: None,
    };
    let mut customers = Vec::new();
    let mut failures = Vec::new();
    for id in list_accessible_customers(up, access_token, developer_token).await? {
        match get_customer(up, creds, &id).await {
            Ok(Some(customer)) => customers.push(customer),
            Ok(None) => {}
            Err(failure)
                if failure
                    .reason()
                    .is_some_and(|r| INACTIVE_CUSTOMER_REASONS.contains(&r)) =>
            {
                tracing::info!(customer_id = %id, "google ads: inactive customer account skipped");
            }
            Err(failure) => {
                tracing::warn!(customer_id = %id, error = %failure.public_message(&[]), "google ads: accessible customer could not be read");
                failures.push(failure);
            }
        }
    }
    if customers.is_empty()
        && let Some(first) = failures
            .iter()
            .find(|f| project_not_approved(f))
            .or(failures.first())
    {
        return Err(first.clone());
    }
    customers.retain(|c| c.status.as_deref() == Some("ENABLED"));
    let mut accounts: Vec<Account> = customers
        .iter()
        .filter(|c| !c.manager)
        .map(|c| Account {
            customer_id: c.id.clone(),
            login_customer_id: None,
            descriptive_name: c.descriptive_name.clone(),
            currency_code: c.currency_code.clone(),
        })
        .collect();
    for manager in customers.iter().filter(|c| c.manager) {
        match list_client_customers(up, access_token, developer_token, &manager.id).await {
            Ok(clients) => {
                for client in clients {
                    if !accounts.iter().any(|a| a.customer_id == client.id) {
                        accounts.push(Account {
                            customer_id: client.id,
                            login_customer_id: Some(manager.id.clone()),
                            descriptive_name: client.descriptive_name,
                            currency_code: client.currency_code,
                        });
                    }
                }
            }
            Err(failure) => {
                tracing::warn!(manager_customer_id = %manager.id, error = %failure.public_message(&[]), "google ads: manager clients could not be listed");
                failures.push(failure);
            }
        }
    }
    if accounts.is_empty()
        && let Some(failure) = failures
            .iter()
            .find(|f| project_not_approved(f))
            .or_else(|| failures.iter().find(|f| f.retryable()))
    {
        return Err(failure.clone());
    }
    Ok(accounts)
}

// ---- OAuth tokens -------------------------------------------------------------

fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}

fn form_post(url: &str, pairs: &[(&str, &str)]) -> UpstreamRequest {
    UpstreamRequest {
        method: "POST",
        url: url.to_string(),
        headers: vec![(
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(form(pairs)),
    }
}

/// The token-endpoint answer of a grant: `Ok(tokens)` or `Err(revoked?)`.
pub async fn token_request(
    up: &impl Upstream,
    pairs: &[(&str, &str)],
) -> Result<Value, (bool, String)> {
    match up.send(form_post(TOKEN_URL, pairs)).await {
        Ok((status, body)) if (200..300).contains(&status) => serde_json::from_str(&body)
            .map_err(|_| (false, "Google token answer could not be read".into())),
        Ok((status, body)) => {
            let answer: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let code = answer["error"].as_str().unwrap_or_default();
            // `isGoogleRevokedError`: `invalid_grant` is a revoked / expired grant.
            let revoked = code == "invalid_grant" || status == 401;
            Err((
                revoked,
                format!("Google token request failed ({status} {code})"),
            ))
        }
        Err(error) => Err((false, sanitize(&error, &[]))),
    }
}

/// `getTokenInfo`.
pub async fn token_info(up: &impl Upstream, access_token: &str) -> Result<Value, Failure> {
    let url = format!(
        "{TOKENINFO_URL}?{}",
        form(&[("access_token", access_token)])
    );
    rescue(
        up.send(UpstreamRequest {
            method: "GET",
            url,
            headers: Vec::new(),
            body: None,
        })
        .await,
    )
}

/// `refreshAuth`: a new access token on the stored refresh token.
/// `Err(true)` = the grant is revoked (reconnect needed).
pub async fn refresh_auth(up: &impl Upstream, auth: &Value) -> Result<Value, (bool, String)> {
    let refresh = auth["tokens"]["refreshToken"].as_str().unwrap_or_default();
    if refresh.is_empty() {
        return Err((true, "Google Ads refresh token not available".into()));
    }
    let answer = token_request(
        up,
        &[
            ("client_id", auth["clientId"].as_str().unwrap_or_default()),
            (
                "client_secret",
                auth["clientSecret"].as_str().unwrap_or_default(),
            ),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ],
    )
    .await?;
    let Some(access) = answer["access_token"].as_str().filter(|t| !t.is_empty()) else {
        return Err((true, "Google Ads refresh returned no token".into()));
    };
    let mut next = auth.clone();
    next["tokens"]["accessToken"] = json!(access);
    if let Some(token) = answer["refresh_token"].as_str() {
        next["tokens"]["refreshToken"] = json!(token);
    }
    if let Some(seconds) = answer["expires_in"].as_i64() {
        next["tokens"]["expiresAt"] = json!(chatbotx_db::repo::iso(
            chatbotx_db::repo::now_ms() + seconds * 1000
        ));
    }
    Ok(next)
}

/// `disconnect`: revokes the grant (the refresh token, else the access
/// token). An already invalid token is the wanted state.
pub async fn revoke(up: &impl Upstream, auth: &Value) -> Result<(), String> {
    let token = auth["tokens"]["refreshToken"]
        .as_str()
        .or_else(|| auth["tokens"]["accessToken"].as_str())
        .unwrap_or_default();
    match up.send(form_post(REVOKE_URL, &[("token", token)])).await {
        Ok((200..=299, _)) => Ok(()),
        Ok((400, body)) if body.contains("invalid_token") || body.contains("invalid_grant") => {
            Ok(())
        }
        Ok((status, _)) => Err(format!("Google Ads token revoke failed ({status})")),
        Err(error) => Err(format!(
            "Google Ads token revoke failed: {}",
            sanitize(&error, &[])
        )),
    }
}

/// Whether `auth`'s access token is expired (or expires within a minute).
pub fn token_expired(auth: &Value, now_ms: i64) -> bool {
    auth["tokens"]["expiresAt"]
        .as_str()
        .and_then(chatbotx_core::js::parse_date)
        .is_some_and(|at| at <= now_ms + 60_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_follow_the_upload_method() {
        assert_eq!(
            authorize_scopes("legacy"),
            format!("{ADS_SCOPE} openid email")
        );
        let granted = parse_scopes(&format!("{ADS_SCOPE} email"));
        assert_eq!(
            missing_scopes(&granted, "dataManager"),
            vec![DATA_MANAGER_SCOPE]
        );
        assert!(missing_scopes(&granted, "legacy").is_empty());
        assert_eq!(method_of_grant("legacy", &granted), "legacy");
        assert_eq!(method_of_grant("dataManager", &granted), "dataManager");
        assert!(has_required_scopes(&json!({ "metadata": {} })));
        assert!(!has_required_scopes(
            &json!({ "metadata": { "scope": ADS_SCOPE } })
        ));
    }

    #[test]
    fn parses_and_classifies_google_errors() {
        let body = r#"[{"error":{"code":403,"message":"denied","status":"PERMISSION_DENIED","details":[{"errors":[{"errorCode":{"authorizationError":"DEVELOPER_TOKEN_NOT_APPROVED"},"message":"token"}]}]}}]"#;
        let error = parse_error(403, body);
        assert_eq!(
            error.reason.as_deref(),
            Some("DEVELOPER_TOKEN_NOT_APPROVED")
        );
        assert_eq!(error.category.as_deref(), Some("authorizationError"));
        assert_eq!(error.message, "denied | token");
        assert_eq!(
            classify(&Failure::Google(error)),
            Some("developer_token_not_approved")
        );
        assert_eq!(rescue(Ok((401, "{}".into()))), Err(Failure::Auth));
        let not_ads = r#"{"error":{"details":[{"reason":"NOT_ADS_USER"}]}}"#;
        let failure = rescue(Ok((401, not_ads.into()))).unwrap_err();
        assert_eq!(classify(&failure), Some("no_ads_account"));
        assert!(rescue(Ok((503, String::new()))).unwrap_err().retryable());
    }

    #[test]
    fn sanitize_removes_tokens_and_click_ids() {
        let text = "Bearer ya29.abc gclid: Cj0KCQ x@y.com +4915112345678 secret-1";
        let out = sanitize(text, &["secret-1"]);
        assert!(!out.contains("ya29"));
        assert!(!out.contains("Cj0KCQ"));
        assert!(!out.contains("x@y.com"));
        assert!(!out.contains("4915112345678"));
        assert!(!out.contains("secret-1"));
        assert_eq!(format_customer_id("1234567890"), "123-456-7890");
        assert!(checked_id("12345", "customer id").is_err());
    }
}
