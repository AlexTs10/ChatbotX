//! `packages/payments/src/stripe.ts` on reqwest (the `stripe` SDK calls the
//! code uses: account, webhook endpoints, Checkout Sessions, refunds,
//! Connect OAuth, `webhooks.constructEvent`).
//!
//! As the SDK (`maxNetworkRetries: 2`): a network error, a 409, a 5xx or
//! `Stripe-Should-Retry: true` is retried twice (0.5 s, 1 s), a POST carries
//! an idempotency key. ponytail: no `Stripe-Version` header (the account's
//! default API version); the fields read here are the same in every version.

use std::time::Duration;

use ring::hmac;
use serde_json::{Value, json};

use super::{
    CheckoutStatus, CreateCheckout, CreatedCheckout, ErrorKind, PResult, PaymentMetadata,
    ProviderError, WebhookEvent, http, to_stripe_minor_units,
};

/// `https://api.stripe.com` (tests point it elsewhere).
pub const STRIPE_API: &str = "https://api.stripe.com";
/// `https://connect.stripe.com` (Connect OAuth).
pub const STRIPE_CONNECT: &str = "https://connect.stripe.com";

/// `STRIPE_CONNECT_WEBHOOK_EVENTS`.
pub const CONNECT_WEBHOOK_EVENTS: &[&str] = &[
    "checkout.session.completed",
    "checkout.session.async_payment_succeeded",
    "checkout.session.async_payment_failed",
    "checkout.session.expired",
    "account.application.deauthorized",
];

/// `STRIPE_ACCOUNT_WEBHOOK_EVENTS`.
pub const ACCOUNT_WEBHOOK_EVENTS: &[&str] = &[
    "checkout.session.completed",
    "checkout.session.async_payment_succeeded",
    "checkout.session.async_payment_failed",
    "checkout.session.expired",
];

const ALREADY_REFUNDED_CODE: &str = "charge_already_refunded";
const AMOUNT_CODES: &[&str] = &[
    "amount_too_small",
    "amount_too_large",
    "invalid_currency",
    "currency_not_supported",
];
const MAX_NETWORK_RETRIES: u32 = 2;
/// `constructEvent` default tolerance.
const SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// A store's Stripe account: its own key, or the platform key acting on it
/// (`Stripe-Account`).
#[derive(Debug, Clone)]
pub struct StripeAccount {
    pub base: String,
    pub secret_key: String,
    /// `Some(acct_…)` = Connect (`Stripe-Account` header).
    pub account: Option<String>,
}

/// A Stripe API error (the SDK's `StripeError` subclasses, by status / type).
#[derive(Debug, Clone)]
struct ApiError {
    status: u16,
    kind: &'static str,
    code: Option<String>,
    message: String,
}

impl ApiError {
    /// `StripeInvalidRequestError`.
    fn is_invalid_request(&self) -> bool {
        self.kind == "invalid_request"
    }

    /// `toProviderError`.
    fn into_provider(self) -> ProviderError {
        let kind = match self.kind {
            "authentication" | "permission" | "oauth" => ErrorKind::CredentialsRejected,
            "invalid_request"
                if self
                    .code
                    .as_deref()
                    .is_some_and(|code| AMOUNT_CODES.contains(&code)) =>
            {
                ErrorKind::AmountUnsupported
            }
            "invalid_request" | "card" => ErrorKind::InvalidRequest,
            _ => ErrorKind::Unavailable,
        };
        ProviderError::new(kind, self.message).code(self.code)
    }
}

/// `qs.stringify(params, { arrayFormat: "indices" })` pairs.
pub fn form_pairs(value: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}[{key}]")
                };
                form_pairs(item, &name, out);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                form_pairs(item, &format!("{prefix}[{index}]"), out);
            }
        }
        Value::Null => {}
        Value::String(text) => out.push((prefix.to_string(), text.clone())),
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

fn form_body(params: &Value) -> String {
    let mut pairs = Vec::new();
    form_pairs(params, "", &mut pairs);
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn random_key() -> String {
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

enum Failure {
    Network(String),
    Api(ApiError),
}

impl Failure {
    fn into_provider(self) -> ProviderError {
        match self {
            Failure::Network(message) => ProviderError::new(ErrorKind::Unavailable, message),
            Failure::Api(error) => error.into_provider(),
        }
    }
}

/// One SDK request with its retries.
async fn call(
    base: &str,
    key: &str,
    account: Option<&str>,
    method: reqwest::Method,
    path: &str,
    params: Option<&Value>,
    idempotency_key: Option<&str>,
) -> Result<Value, Failure> {
    let client = http().map_err(|e| Failure::Network(e.message))?;
    let body = params.map(form_body);
    let idempotency = match (&method, idempotency_key) {
        (_, Some(key)) => Some(key.to_string()),
        (&reqwest::Method::POST, None) => Some(format!("stripe-node-retry-{}", random_key())),
        _ => None,
    };
    let mut attempt = 0u32;
    loop {
        let mut request = client
            .request(method.clone(), format!("{base}{path}"))
            .bearer_auth(key);
        if let Some(account) = account {
            request = request.header("Stripe-Account", account);
        }
        if let Some(key) = &idempotency {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(body) = &body {
            request = request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body.clone());
        }
        let outcome = request.send().await;
        let retry = match &outcome {
            Err(_) => true,
            Ok(response) => {
                let status = response.status().as_u16();
                match response
                    .headers()
                    .get("stripe-should-retry")
                    .and_then(|v| v.to_str().ok())
                {
                    Some("true") => true,
                    Some("false") => false,
                    _ => status == 409 || status >= 500,
                }
            }
        };
        if retry && attempt < MAX_NETWORK_RETRIES {
            attempt += 1;
            tokio::time::sleep(Duration::from_millis(500 * (1 << (attempt - 1)))).await;
            continue;
        }
        let response = outcome.map_err(|e| {
            Failure::Network(format!(
                "An error occurred with our connection to Stripe. {e}"
            ))
        })?;
        let status = response.status().as_u16();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let error = body.get("error").cloned().unwrap_or(Value::Null);
        let message = error["message"]
            .as_str()
            .or_else(|| error["error_description"].as_str())
            .or_else(|| body["error_description"].as_str())
            .unwrap_or("Stripe refused the request.")
            .to_string();
        let raw_type = error["type"]
            .as_str()
            .or_else(|| body["error"].as_str())
            .unwrap_or_default();
        let kind = match status {
            401 => "authentication",
            403 => "permission",
            429 => "rate_limit",
            _ => match raw_type {
                "card_error" => "card",
                "invalid_request_error" => "invalid_request",
                "authentication_error" => "authentication",
                "invalid_client" => "invalid_client",
                "invalid_grant"
                | "invalid_request"
                | "unsupported_grant_type"
                | "invalid_scope"
                | "unsupported_response_type" => "oauth",
                _ => "api",
            },
        };
        let code = error["code"]
            .as_str()
            .or_else(|| (kind == "oauth" || kind == "invalid_client").then_some(raw_type))
            .map(str::to_string);
        return Err(Failure::Api(ApiError {
            status,
            kind,
            code,
            message,
        }));
    }
}

fn read_metadata(metadata: &Value) -> Option<PaymentMetadata> {
    let get = |key: &str| {
        metadata[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Some(PaymentMetadata {
        workspace_id: get("workspaceId")?,
        order_id: get("orderId")?,
        order_payment_id: get("orderPaymentId")?,
    })
}

fn to_checkout_status(session: &Value) -> PResult<CheckoutStatus> {
    if session["payment_status"] == "paid" {
        let transaction = match &session["payment_intent"] {
            Value::String(id) => Some(id.clone()),
            Value::Object(intent) => intent.get("id").and_then(Value::as_str).map(str::to_string),
            _ => None,
        };
        let currency = session["currency"].as_str().filter(|c| !c.is_empty());
        let amount = session["amount_total"].as_i64();
        return match (transaction.filter(|t| !t.is_empty()), currency, amount) {
            (Some(transaction_id), Some(currency), Some(amount_minor)) => {
                Ok(CheckoutStatus::Paid {
                    transaction_id,
                    amount_minor,
                    currency: currency.to_uppercase(),
                })
            }
            _ => Err(ProviderError::new(
                ErrorKind::InvalidRequest,
                format!(
                    "Stripe reports session {} paid without its payment.",
                    session["id"].as_str().unwrap_or_default()
                ),
            )),
        };
    }
    Ok(match session["status"].as_str() {
        Some("expired") => CheckoutStatus::Closed,
        Some("complete") => CheckoutStatus::Processing,
        _ => CheckoutStatus::Open,
    })
}

fn account_label(account: &Value) -> Option<String> {
    [
        &account["business_profile"]["name"],
        &account["settings"]["dashboard"]["display_name"],
        &account["email"],
    ]
    .into_iter()
    .find_map(|v| v.as_str().map(str::to_string))
}

impl StripeAccount {
    /// The store's own key.
    pub fn own(secret_key: &str) -> StripeAccount {
        StripeAccount {
            base: STRIPE_API.to_string(),
            secret_key: secret_key.to_string(),
            account: None,
        }
    }

    /// The platform key acting on a connected account.
    pub fn connected(platform_secret_key: &str, account_id: &str) -> StripeAccount {
        StripeAccount {
            base: STRIPE_API.to_string(),
            secret_key: platform_secret_key.to_string(),
            account: Some(account_id.to_string()),
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        params: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Failure> {
        call(
            &self.base,
            &self.secret_key,
            self.account.as_deref(),
            method,
            path,
            params,
            idempotency_key,
        )
        .await
    }

    pub async fn create_checkout(&self, input: &CreateCheckout) -> PResult<CreatedCheckout> {
        let unit_amount = to_stripe_minor_units(input.amount, &input.currency)?;
        let metadata = json!({
            "workspaceId": input.metadata.workspace_id,
            "orderId": input.metadata.order_id,
            "orderPaymentId": input.metadata.order_payment_id,
        });
        let mut params = json!({
            "mode": "payment",
            "payment_method_types": ["card"],
            "line_items": [{
                "quantity": 1,
                "price_data": {
                    "currency": input.currency.to_lowercase(),
                    "unit_amount": unit_amount,
                    "product_data": { "name": input.description },
                },
            }],
            "success_url": input.return_url,
            "cancel_url": input.cancel_url,
            "expires_at": input.expires_at.div_euclid(1000),
            "client_reference_id": input.metadata.order_payment_id,
        });
        if let Some(email) = input.customer_email.as_deref().filter(|e| !e.is_empty()) {
            params["customer_email"] = json!(email);
        }
        params["metadata"] = metadata.clone();
        params["payment_intent_data"] = json!({ "metadata": metadata });
        let session = self
            .request(
                reqwest::Method::POST,
                "/v1/checkout/sessions",
                Some(&params),
                Some(&input.idempotency_key),
            )
            .await
            .map_err(Failure::into_provider)?;
        let id = session["id"].as_str().unwrap_or_default().to_string();
        match session["url"].as_str().filter(|u| !u.is_empty()) {
            Some(url) => Ok(CreatedCheckout {
                reference: id,
                url: url.to_string(),
            }),
            None => Err(ProviderError::new(
                ErrorKind::InvalidRequest,
                format!("Stripe created session {id} without a page."),
            )),
        }
    }

    pub async fn get_checkout_status(&self, reference: &str) -> PResult<CheckoutStatus> {
        let session = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/checkout/sessions/{}", encode(reference)),
                None,
                None,
            )
            .await
            .map_err(Failure::into_provider)?;
        to_checkout_status(&session)
    }

    pub async fn expire_checkout(&self, reference: &str) -> PResult<CheckoutStatus> {
        match self
            .request(
                reqwest::Method::POST,
                &format!("/v1/checkout/sessions/{}/expire", encode(reference)),
                Some(&json!({})),
                None,
            )
            .await
        {
            Ok(session) => to_checkout_status(&session),
            // Only an open session can be expired: its status says why not.
            Err(Failure::Api(error)) if error.is_invalid_request() => {
                self.get_checkout_status(reference).await
            }
            Err(other) => Err(other.into_provider()),
        }
    }

    pub async fn refund(
        &self,
        transaction_id: &str,
        idempotency_key: &str,
    ) -> PResult<Option<String>> {
        match self
            .request(
                reqwest::Method::POST,
                "/v1/refunds",
                Some(&json!({ "payment_intent": transaction_id })),
                Some(idempotency_key),
            )
            .await
        {
            Ok(refund) => Ok(refund["id"].as_str().map(str::to_string)),
            Err(Failure::Api(error))
                if error.is_invalid_request()
                    && error.code.as_deref() == Some(ALREADY_REFUNDED_CODE) =>
            {
                Ok(None)
            }
            Err(other) => Err(other.into_provider()),
        }
    }
}

fn encode(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect()
}

// ---- a store's own secret key ------------------------------------------------

/// `isStripeSecretKey`: `/^(sk|rk)_(live|test)_[A-Za-z0-9]+$/`.
pub fn is_stripe_secret_key(key: &str) -> bool {
    let rest = ["sk_live_", "sk_test_", "rk_live_", "rk_test_"]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix));
    rest.is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `isStripeLiveKey`.
pub fn is_stripe_live_key(key: &str) -> bool {
    key.starts_with("sk_live_") || key.starts_with("rk_live_")
}

/// What `readStripeKeyAccount` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyAccount {
    pub account_id: String,
    pub label: Option<String>,
    pub charges_enabled: bool,
    pub livemode: bool,
}

/// `readStripeKeyAccount`: `GET /v1/account` with the store's key.
pub async fn read_key_account(base: &str, secret_key: &str) -> PResult<KeyAccount> {
    let account = call(
        base,
        secret_key,
        None,
        reqwest::Method::GET,
        "/v1/account",
        None,
        None,
    )
    .await
    .map_err(Failure::into_provider)?;
    Ok(KeyAccount {
        account_id: account["id"].as_str().unwrap_or_default().to_string(),
        label: account_label(&account),
        charges_enabled: account["charges_enabled"].as_bool().unwrap_or(false),
        livemode: is_stripe_live_key(secret_key),
    })
}

/// `createStripeAccountWebhook`.
pub async fn create_account_webhook(
    base: &str,
    secret_key: &str,
    url: &str,
) -> PResult<(String, String)> {
    let params = json!({
        "url": url,
        "enabled_events": ACCOUNT_WEBHOOK_EVENTS,
        "description": "Web shop payments",
    });
    let endpoint = call(
        base,
        secret_key,
        None,
        reqwest::Method::POST,
        "/v1/webhook_endpoints",
        Some(&params),
        None,
    )
    .await
    .map_err(Failure::into_provider)?;
    let secret = endpoint["secret"].as_str().filter(|s| !s.is_empty());
    match secret {
        Some(secret) => Ok((
            endpoint["id"].as_str().unwrap_or_default().to_string(),
            secret.to_string(),
        )),
        None => Err(ProviderError::new(
            ErrorKind::InvalidRequest,
            "Stripe created the webhook without a signing secret.",
        )),
    }
}

/// `deleteStripeAccountWebhook` / `deleteStripeConnectWebhook`: one already
/// gone is fine.
pub async fn delete_webhook(base: &str, secret_key: &str, endpoint_id: &str) -> PResult<()> {
    match call(
        base,
        secret_key,
        None,
        reqwest::Method::DELETE,
        &format!("/v1/webhook_endpoints/{}", encode(endpoint_id)),
        None,
        None,
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(Failure::Api(error)) if error.is_invalid_request() && error.status == 404 => Ok(()),
        Err(other) => Err(other.into_provider()),
    }
}

// ---- Stripe Connect ------------------------------------------------------------

/// `buildStripeAuthorizeUrl` (`oauth.authorizeUrl`).
pub fn authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    email: Option<&str>,
) -> String {
    let mut pairs = vec![
        ("response_type".to_string(), "code".to_string()),
        ("client_id".to_string(), client_id.to_string()),
        ("scope".to_string(), "read_write".to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
        ("state".to_string(), state.to_string()),
    ];
    if let Some(email) = email.filter(|e| !e.is_empty()) {
        pairs.push(("stripe_user[email]".to_string(), email.to_string()));
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &pairs {
        serializer.append_pair(key, value);
    }
    format!("{STRIPE_CONNECT}/oauth/authorize?{}", serializer.finish())
}

/// `exchangeStripeCode`: `(accountId, livemode)`.
pub async fn exchange_code(platform_secret_key: &str, code: &str) -> PResult<(String, bool)> {
    let token = call(
        STRIPE_CONNECT,
        platform_secret_key,
        None,
        reqwest::Method::POST,
        "/oauth/token",
        Some(&json!({ "grant_type": "authorization_code", "code": code })),
        None,
    )
    .await
    .map_err(Failure::into_provider)?;
    match (
        token["stripe_user_id"].as_str(),
        token["livemode"].as_bool(),
    ) {
        (Some(id), Some(livemode)) if !id.is_empty() => Ok((id.to_string(), livemode)),
        _ => Err(ProviderError::new(
            ErrorKind::InvalidRequest,
            "Stripe connected the account without its id.",
        )),
    }
}

/// `readStripeAccount`: `(label, chargesEnabled)` of a connected account.
pub async fn read_connected_account(
    platform_secret_key: &str,
    account_id: &str,
) -> PResult<(Option<String>, bool)> {
    let account = call(
        STRIPE_API,
        platform_secret_key,
        None,
        reqwest::Method::GET,
        &format!("/v1/accounts/{}", encode(account_id)),
        None,
        None,
    )
    .await
    .map_err(Failure::into_provider)?;
    Ok((
        account_label(&account),
        account["charges_enabled"].as_bool().unwrap_or(false),
    ))
}

/// `deauthorizeStripe`: an account already disconnected counts as done.
pub async fn deauthorize(
    platform_secret_key: &str,
    client_id: &str,
    account_id: &str,
) -> PResult<()> {
    match call(
        STRIPE_CONNECT,
        platform_secret_key,
        None,
        reqwest::Method::POST,
        "/oauth/deauthorize",
        Some(&json!({ "client_id": client_id, "stripe_user_id": account_id })),
        None,
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(Failure::Api(error)) if error.kind == "invalid_client" => Ok(()),
        Err(other) => Err(other.into_provider()),
    }
}

// ---- webhooks ----------------------------------------------------------------------

fn signature_invalid(message: &str) -> ProviderError {
    ProviderError::new(ErrorKind::SignatureInvalid, message)
}

/// `webhooks.constructEvent`: `t=…,v1=…` over `"{t}.{body}"` with HMAC-SHA256,
/// within 300 s of `now_secs`.
pub fn verify_signature(
    raw_body: &str,
    header: &str,
    secret: &str,
    now_secs: i64,
) -> PResult<Value> {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        if let Some((key, value)) = part.trim().split_once('=') {
            match key {
                "t" => timestamp = value.parse::<i64>().ok(),
                "v1" => signatures.push(value.to_string()),
                _ => {}
            }
        }
    }
    let Some(timestamp) = timestamp else {
        return Err(signature_invalid(
            "Unable to extract timestamp and signatures from header",
        ));
    };
    if signatures.is_empty() {
        return Err(signature_invalid(
            "No signatures found with expected scheme",
        ));
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let payload = format!("{timestamp}.{raw_body}");
    let matched = signatures.iter().any(|sig| {
        decode_hex(sig).is_some_and(|bytes| hmac::verify(&key, payload.as_bytes(), &bytes).is_ok())
    });
    if !matched {
        return Err(signature_invalid(
            "No signatures found matching the expected signature for payload.",
        ));
    }
    if (now_secs - timestamp).abs() > SIGNATURE_TOLERANCE_SECS {
        return Err(signature_invalid("Timestamp outside the tolerance zone"));
    }
    serde_json::from_str(raw_body).map_err(|_| signature_invalid("The webhook body is not JSON."))
}

/// The `v1` signature of `body` at `timestamp` (tests and the parity fake).
pub fn sign(raw_body: &str, secret: &str, timestamp: i64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, format!("{timestamp}.{raw_body}").as_bytes());
    let hex: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("t={timestamp},v1={hex}")
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `parseStripeConnectWebhook`.
pub fn parse_connect_webhook(
    raw_body: &str,
    signature: Option<&str>,
    webhook_secret: &str,
    now_secs: i64,
) -> PResult<WebhookEvent> {
    let Some(signature) = signature else {
        return Err(signature_invalid("The Stripe-Signature header is missing."));
    };
    let event = verify_signature(raw_body, signature, webhook_secret, now_secs)?;
    let event_id = event["id"].as_str().unwrap_or_default().to_string();
    let kind = event["type"].as_str().unwrap_or_default();
    let account = event["account"].as_str().filter(|a| !a.is_empty());
    if kind == "account.application.deauthorized"
        && let Some(account) = account
    {
        return Ok(WebhookEvent::Deauthorized {
            event_id,
            account: account.to_string(),
            created_at: event["created"].as_i64().unwrap_or(0) * 1000,
        });
    }
    if !ACCOUNT_WEBHOOK_EVENTS.contains(&kind) {
        return Ok(WebhookEvent::Ignored { event_id });
    }
    let session = &event["data"]["object"];
    Ok(WebhookEvent::Checkout {
        event_id,
        reference: session["id"].as_str().unwrap_or_default().to_string(),
        metadata: read_metadata(&session["metadata"]),
        account: account.map(str::to_string),
    })
}

/// `parseStripeAccountWebhook`: as Connect, no `deauthorized`, no account.
pub fn parse_account_webhook(
    raw_body: &str,
    signature: Option<&str>,
    webhook_secret: &str,
    now_secs: i64,
) -> PResult<WebhookEvent> {
    Ok(
        match parse_connect_webhook(raw_body, signature, webhook_secret, now_secs)? {
            WebhookEvent::Deauthorized { event_id, .. } => WebhookEvent::Ignored { event_id },
            WebhookEvent::Checkout {
                event_id,
                reference,
                metadata,
                ..
            } => WebhookEvent::Checkout {
                event_id,
                reference,
                metadata,
                account: None,
            },
            ignored => ignored,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_key_shapes() {
        assert!(is_stripe_secret_key("sk_test_abc123"));
        assert!(is_stripe_secret_key("rk_live_X9"));
        assert!(!is_stripe_secret_key("pk_test_abc"));
        assert!(!is_stripe_secret_key("sk_test_has spaces"));
        assert!(!is_stripe_secret_key("sk_test_"));
        assert!(is_stripe_live_key("rk_live_1"));
        assert!(!is_stripe_live_key("sk_test_1"));
    }

    #[test]
    fn form_encoding_uses_bracket_indices() {
        let body = form_body(&json!({
            "enabled_events": ["a", "b"],
            "line_items": [{ "quantity": 1, "price_data": { "currency": "usd" } }],
        }));
        assert_eq!(
            body,
            "enabled_events%5B0%5D=a&enabled_events%5B1%5D=b&line_items%5B0%5D%5Bquantity%5D=1&line_items%5B0%5D%5Bprice_data%5D%5Bcurrency%5D=usd"
        );
    }

    #[test]
    fn webhook_signature_is_checked_like_construct_event() {
        let body = r#"{"id":"evt_1","type":"checkout.session.completed","account":"acct_1","data":{"object":{"id":"cs_1","metadata":{"workspaceId":"1","orderId":"2","orderPaymentId":"3"}}}}"#;
        let header = sign(body, "whsec_x", 1_000);
        let event = parse_connect_webhook(body, Some(&header), "whsec_x", 1_100).unwrap();
        assert_eq!(
            event,
            WebhookEvent::Checkout {
                event_id: "evt_1".into(),
                reference: "cs_1".into(),
                metadata: Some(PaymentMetadata {
                    workspace_id: "1".into(),
                    order_id: "2".into(),
                    order_payment_id: "3".into(),
                }),
                account: Some("acct_1".into()),
            }
        );
        let own = parse_account_webhook(body, Some(&header), "whsec_x", 1_100).unwrap();
        assert!(matches!(own, WebhookEvent::Checkout { account: None, .. }));
        let wrong = parse_connect_webhook(body, Some(&header), "whsec_y", 1_100).unwrap_err();
        assert_eq!(wrong.kind, ErrorKind::SignatureInvalid);
        let late = parse_connect_webhook(body, Some(&header), "whsec_x", 1_400).unwrap_err();
        assert_eq!(late.kind, ErrorKind::SignatureInvalid);
        let missing = parse_connect_webhook(body, None, "whsec_x", 1_100).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::SignatureInvalid);
    }

    #[test]
    fn deauthorized_and_ignored_events() {
        let body = r#"{"id":"evt_2","type":"account.application.deauthorized","account":"acct_9","created":50}"#;
        let header = sign(body, "s", 60);
        assert_eq!(
            parse_connect_webhook(body, Some(&header), "s", 60).unwrap(),
            WebhookEvent::Deauthorized {
                event_id: "evt_2".into(),
                account: "acct_9".into(),
                created_at: 50_000,
            }
        );
        assert_eq!(
            parse_account_webhook(body, Some(&header), "s", 60).unwrap(),
            WebhookEvent::Ignored {
                event_id: "evt_2".into()
            }
        );
        let other = r#"{"id":"evt_3","type":"charge.succeeded"}"#;
        let header = sign(other, "s", 60);
        assert_eq!(
            parse_connect_webhook(other, Some(&header), "s", 60).unwrap(),
            WebhookEvent::Ignored {
                event_id: "evt_3".into()
            }
        );
    }

    #[test]
    fn session_status_mapping() {
        assert_eq!(
            to_checkout_status(&json!({ "id": "cs", "payment_status": "paid", "payment_intent": "pi_1", "currency": "usd", "amount_total": 1000 })).unwrap(),
            CheckoutStatus::Paid { transaction_id: "pi_1".into(), amount_minor: 1000, currency: "USD".into() }
        );
        assert!(to_checkout_status(&json!({ "id": "cs", "payment_status": "paid" })).is_err());
        assert_eq!(
            to_checkout_status(&json!({ "status": "expired" })).unwrap(),
            CheckoutStatus::Closed
        );
        assert_eq!(
            to_checkout_status(&json!({ "status": "complete", "payment_status": "unpaid" }))
                .unwrap(),
            CheckoutStatus::Processing
        );
        assert_eq!(
            to_checkout_status(&json!({ "status": "open" })).unwrap(),
            CheckoutStatus::Open
        );
    }
}
