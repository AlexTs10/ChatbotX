//! `packages/payments/src/paypal.ts`: a store's own PayPal REST app (Client ID
//! and Secret): orders v2, captures, refunds, webhooks and their signature
//! check (`/v1/notifications/verify-webhook-signature`).
//!
//! ponytail: the OAuth token is fetched per gateway (the original caches it
//! on the client object, which also lives for one gateway).

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{
    CheckoutStatus, CreateCheckout, CreatedCheckout, ErrorKind, PResult, PaymentMetadata,
    PaypalSecret, ProviderError, WebhookEvent, format_paypal_value, http, parse_paypal_value,
    to_paypal_minor_units,
};

pub const PAYPAL_LIVE: &str = "https://api-m.paypal.com";
pub const PAYPAL_SANDBOX: &str = "https://api-m.sandbox.paypal.com";

/// `PAYPAL_WEBHOOK_EVENTS`.
pub const WEBHOOK_EVENTS: &[&str] = &[
    "CHECKOUT.ORDER.APPROVED",
    "PAYMENT.CAPTURE.COMPLETED",
    "PAYMENT.CAPTURE.DENIED",
    "PAYMENT.CAPTURE.PENDING",
];

const TOKEN_REFRESH_MARGIN_MS: i64 = 60_000;

/// One PayPal app (live or sandbox).
#[derive(Debug, Clone)]
pub struct Paypal {
    pub base: String,
    pub client_id: String,
    pub client_secret: String,
    token: Arc<Mutex<Option<(String, i64)>>>,
}

/// A PayPal HTTP error (`PaypalHttpError`).
#[derive(Debug, Clone)]
struct HttpError {
    status: u16,
    body: Value,
    error: ProviderError,
}

impl HttpError {
    fn new(status: u16, body: Value) -> HttpError {
        let first = &body["details"][0];
        let issue = first["issue"]
            .as_str()
            .or_else(|| body["name"].as_str())
            .or_else(|| body["error"].as_str())
            .map(str::to_string);
        let message = first["description"]
            .as_str()
            .or_else(|| body["message"].as_str())
            .or_else(|| body["error_description"].as_str())
            .map_or_else(|| format!("PayPal answered {status}."), str::to_string);
        let kind = match status {
            401 | 403 => ErrorKind::CredentialsRejected,
            s if s >= 500 || s == 429 => ErrorKind::Unavailable,
            _ => ErrorKind::InvalidRequest,
        };
        HttpError {
            status,
            error: ProviderError::new(kind, message).code(issue),
            body,
        }
    }

    fn has_issue(&self, issue: &str) -> bool {
        self.body["name"] == issue
            || self.body["details"]
                .as_array()
                .is_some_and(|d| d.iter().any(|x| x["issue"] == issue))
    }
}

enum Failure {
    Provider(ProviderError),
    Http(HttpError),
}

impl Failure {
    fn into_provider(self) -> ProviderError {
        match self {
            Failure::Provider(error) => error,
            Failure::Http(error) => error.error,
        }
    }
}

fn now_ms() -> i64 {
    chatbotx_db::repo::now_ms()
}

impl Paypal {
    pub fn new(client_id: &str, client_secret: &str, livemode: bool) -> Paypal {
        Paypal {
            base: if livemode {
                PAYPAL_LIVE
            } else {
                PAYPAL_SANDBOX
            }
            .to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_secret(secret: &PaypalSecret) -> Paypal {
        Paypal::new(&secret.client_id, &secret.client_secret, secret.livemode)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, Failure> {
        request.send().await.map_err(|_| {
            Failure::Provider(ProviderError::new(
                ErrorKind::Unavailable,
                "PayPal did not answer.",
            ))
        })
    }

    async fn read_error(response: reqwest::Response) -> HttpError {
        let status = response.status().as_u16();
        let body = response.json::<Value>().await.unwrap_or(json!({}));
        HttpError::new(status, if body.is_object() { body } else { json!({}) })
    }

    async fn get_token(&self) -> Result<String, Failure> {
        let mut slot = self.token.lock().await;
        if let Some((value, expires_at)) = slot.as_ref()
            && *expires_at > now_ms() + TOKEN_REFRESH_MARGIN_MS
        {
            return Ok(value.clone());
        }
        let client = http().map_err(Failure::Provider)?;
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", self.client_id, self.client_secret));
        let response = self
            .send(
                client
                    .post(format!("{}/v1/oauth2/token", self.base))
                    .header("Authorization", format!("Basic {basic}"))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body("grant_type=client_credentials"),
            )
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error = Self::read_error(response).await;
            if status == 400 || status == 401 {
                return Err(Failure::Provider(
                    ProviderError::new(
                        ErrorKind::CredentialsRejected,
                        "PayPal refused the Client ID or Secret.",
                    )
                    .code(error.error.provider_code),
                ));
            }
            return Err(Failure::Http(error));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let value = body["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expires_in = body["expires_in"].as_i64().unwrap_or(0);
        *slot = Some((value.clone(), now_ms() + expires_in * 1000));
        Ok(value)
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<String>,
        request_id: Option<&str>,
    ) -> Result<Value, Failure> {
        let token = self.get_token().await?;
        let client = http().map_err(Failure::Provider)?;
        let mut request = client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("Prefer", "return=representation");
        if let Some(id) = request_id {
            request = request.header("PayPal-Request-Id", id);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = self.send(request).await?;
        if !response.status().is_success() {
            return Err(Failure::Http(Self::read_error(response).await));
        }
        if response.status().as_u16() == 204 {
            return Ok(Value::Null);
        }
        Ok(response.json().await.unwrap_or(Value::Null))
    }

    async fn get_order(&self, reference: &str) -> Result<Value, Failure> {
        self.request(
            reqwest::Method::GET,
            &format!("/v2/checkout/orders/{}", encode(reference)),
            None,
            None,
        )
        .await
    }

    pub async fn create_checkout(&self, input: &CreateCheckout) -> PResult<CreatedCheckout> {
        let minor = to_paypal_minor_units(input.amount, &input.currency)?;
        let description = String::from_utf16_lossy(
            &input
                .description
                .encode_utf16()
                .take(127)
                .collect::<Vec<_>>(),
        );
        let body = json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "reference_id": input.metadata.order_payment_id,
                "custom_id": encode_custom_id(&input.metadata),
                "description": description,
                "amount": {
                    "currency_code": input.currency.to_uppercase(),
                    "value": format_paypal_value(minor, &input.currency),
                },
            }],
            "payment_source": { "paypal": { "experience_context": {
                "return_url": input.return_url,
                "cancel_url": input.cancel_url,
                "user_action": "PAY_NOW",
                "shipping_preference": "NO_SHIPPING",
            }}},
        });
        let order = self
            .request(
                reqwest::Method::POST,
                "/v2/checkout/orders",
                Some(body.to_string()),
                Some(&input.idempotency_key),
            )
            .await
            .map_err(Failure::into_provider)?;
        let id = order["id"].as_str().unwrap_or_default().to_string();
        let url = order["links"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["rel"] == "payer-action"))
            .and_then(|l| l["href"].as_str());
        match url {
            Some(url) => Ok(CreatedCheckout {
                reference: id,
                url: url.to_string(),
            }),
            None => Err(ProviderError::new(
                ErrorKind::InvalidRequest,
                format!("PayPal created order {id} without a page to pay on."),
            )),
        }
    }

    pub async fn get_checkout_status(
        &self,
        reference: &str,
        capture: Option<&str>,
    ) -> PResult<CheckoutStatus> {
        let order = self
            .get_order(reference)
            .await
            .map_err(Failure::into_provider)?;
        let Some(key) = capture.filter(|_| order["status"] == "APPROVED") else {
            return to_checkout_status(&order);
        };
        match self
            .request(
                reqwest::Method::POST,
                &format!("/v2/checkout/orders/{}/capture", encode(reference)),
                Some("{}".into()),
                Some(key),
            )
            .await
        {
            Ok(captured) => to_checkout_status(&captured),
            // Captured meanwhile (or declined): the order says how it ended.
            Err(Failure::Http(error)) if error.status == 422 => to_checkout_status(
                &self
                    .get_order(reference)
                    .await
                    .map_err(Failure::into_provider)?,
            ),
            Err(other) => Err(other.into_provider()),
        }
    }

    pub async fn expire_checkout(&self, reference: &str) -> PResult<CheckoutStatus> {
        let status = to_checkout_status(
            &self
                .get_order(reference)
                .await
                .map_err(Failure::into_provider)?,
        )?;
        Ok(if status == CheckoutStatus::Open {
            CheckoutStatus::Closed
        } else {
            status
        })
    }

    pub async fn refund(
        &self,
        transaction_id: &str,
        idempotency_key: &str,
    ) -> PResult<Option<String>> {
        match self
            .request(
                reqwest::Method::POST,
                &format!("/v2/payments/captures/{}/refund", encode(transaction_id)),
                Some("{}".into()),
                Some(idempotency_key),
            )
            .await
        {
            Ok(refund) => Ok(refund["id"].as_str().map(str::to_string)),
            Err(Failure::Http(error)) if error.has_issue("CAPTURE_FULLY_REFUNDED") => Ok(None),
            Err(other) => Err(other.into_provider()),
        }
    }
}

fn encode(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect()
}

fn encode_custom_id(metadata: &PaymentMetadata) -> String {
    format!(
        "{}:{}:{}",
        metadata.workspace_id, metadata.order_id, metadata.order_payment_id
    )
}

/// `decodeCustomId`: `/^(\d+):(\d+):(\d+)$/`.
fn decode_custom_id(custom_id: Option<&str>) -> Option<PaymentMetadata> {
    let mut parts = custom_id?.split(':');
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    (parts.next().is_none() && digits(a) && digits(b) && digits(c)).then(|| PaymentMetadata {
        workspace_id: a.to_string(),
        order_id: b.to_string(),
        order_payment_id: c.to_string(),
    })
}

fn to_checkout_status(order: &Value) -> PResult<CheckoutStatus> {
    match order["status"].as_str() {
        Some("COMPLETED") => {
            let capture = &order["purchase_units"][0]["payments"]["captures"][0];
            match capture["status"].as_str() {
                Some("COMPLETED") => {
                    let amount = &capture["amount"];
                    let (Some(code), Some(value)) =
                        (amount["currency_code"].as_str(), amount["value"].as_str())
                    else {
                        return Err(ProviderError::new(
                            ErrorKind::InvalidRequest,
                            format!(
                                "PayPal captured order {} without its amount.",
                                order["id"].as_str().unwrap_or_default()
                            ),
                        ));
                    };
                    let currency = code.to_uppercase();
                    Ok(CheckoutStatus::Paid {
                        transaction_id: capture["id"].as_str().unwrap_or_default().to_string(),
                        amount_minor: parse_paypal_value(value, &currency)?,
                        currency,
                    })
                }
                Some("PENDING") => Ok(CheckoutStatus::Processing),
                _ => Ok(CheckoutStatus::Closed),
            }
        }
        Some("VOIDED") => Ok(CheckoutStatus::Closed),
        _ => Ok(CheckoutStatus::Open),
    }
}

const SIGNATURE_HEADERS: &[(&str, &str)] = &[
    ("auth_algo", "paypal-auth-algo"),
    ("cert_url", "paypal-cert-url"),
    ("transmission_id", "paypal-transmission-id"),
    ("transmission_sig", "paypal-transmission-sig"),
    ("transmission_time", "paypal-transmission-time"),
];

/// `parsePaypalWebhook`: PayPal itself checks the signature (with the raw
/// event spliced in, so its bytes stay as they were signed).
pub async fn parse_webhook(
    raw_body: &str,
    header: impl Fn(&str) -> Option<String>,
    secret: &PaypalSecret,
    base: Option<&str>,
) -> PResult<WebhookEvent> {
    let invalid = |m: &str| ProviderError::new(ErrorKind::SignatureInvalid, m);
    let mut fields = serde_json::Map::new();
    for (field, name) in SIGNATURE_HEADERS {
        match header(name).filter(|v| !v.is_empty()) {
            Some(value) => {
                fields.insert((*field).to_string(), Value::String(value));
            }
            None => return Err(invalid("The PayPal signature headers are missing.")),
        }
    }
    let event: Value =
        serde_json::from_str(raw_body).map_err(|_| invalid("The webhook body is not JSON."))?;
    fields.insert(
        "webhook_id".into(),
        Value::String(secret.webhook_id.clone()),
    );
    let fields = Value::Object(fields).to_string();
    let body = format!(
        "{},\"webhook_event\":{raw_body}}}",
        &fields[..fields.len() - 1]
    );
    let mut client = Paypal::from_secret(secret);
    if let Some(base) = base {
        client.base = base.to_string();
    }
    let verification = client
        .request(
            reqwest::Method::POST,
            "/v1/notifications/verify-webhook-signature",
            Some(body),
            None,
        )
        .await
        .map_err(Failure::into_provider)?;
    if verification["verification_status"] != "SUCCESS" {
        return Err(invalid("PayPal did not sign this webhook."));
    }
    let event_id = event["id"].as_str().unwrap_or_default().to_string();
    let kind = event["event_type"].as_str().unwrap_or_default();
    let resource = &event["resource"];
    if kind == "CHECKOUT.ORDER.APPROVED"
        && let Some(id) = resource["id"].as_str()
    {
        return Ok(WebhookEvent::Checkout {
            event_id,
            reference: id.to_string(),
            metadata: decode_custom_id(resource["purchase_units"][0]["custom_id"].as_str()),
            account: None,
        });
    }
    if kind.starts_with("PAYMENT.CAPTURE.")
        && let Some(order_id) = resource["supplementary_data"]["related_ids"]["order_id"].as_str()
    {
        return Ok(WebhookEvent::Checkout {
            event_id,
            reference: order_id.to_string(),
            metadata: decode_custom_id(resource["custom_id"].as_str()),
            account: None,
        });
    }
    Ok(WebhookEvent::Ignored { event_id })
}

/// `connectPaypal`: live first, then sandbox; creates (or reuses) the webhook.
pub async fn connect(
    client_id: &str,
    client_secret: &str,
    webhook_url: &str,
) -> PResult<PaypalSecret> {
    let (client_id, client_secret) = (client_id.trim(), client_secret.trim());
    let mut livemode = true;
    let mut client = Paypal::new(client_id, client_secret, livemode);
    if let Err(error) = client.get_token().await {
        let error = error.into_provider();
        if error.kind != ErrorKind::CredentialsRejected {
            return Err(error);
        }
        livemode = false;
        client = Paypal::new(client_id, client_secret, livemode);
        client.get_token().await.map_err(Failure::into_provider)?;
    }
    let webhook_id = create_or_reuse_webhook(&client, webhook_url).await?;
    Ok(PaypalSecret {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        livemode,
        webhook_id,
    })
}

async fn create_or_reuse_webhook(client: &Paypal, url: &str) -> PResult<String> {
    let body = json!({
        "url": url,
        "event_types": WEBHOOK_EVENTS.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
    });
    match client
        .request(
            reqwest::Method::POST,
            "/v1/notifications/webhooks",
            Some(body.to_string()),
            None,
        )
        .await
    {
        Ok(webhook) => Ok(webhook["id"].as_str().unwrap_or_default().to_string()),
        Err(Failure::Http(error)) if error.has_issue("WEBHOOK_URL_ALREADY_EXISTS") => {
            let list = client
                .request(
                    reqwest::Method::GET,
                    "/v1/notifications/webhooks",
                    None,
                    None,
                )
                .await
                .map_err(Failure::into_provider)?;
            list["webhooks"]
                .as_array()
                .and_then(|hooks| hooks.iter().find(|h| h["url"] == url))
                .and_then(|h| h["id"].as_str())
                .map(str::to_string)
                .ok_or(error.error)
        }
        Err(other) => Err(other.into_provider()),
    }
}

/// `disconnectPaypal`: removes the webhook; one already gone is fine.
pub async fn disconnect(secret: &PaypalSecret) -> PResult<()> {
    let client = Paypal::from_secret(secret);
    match client
        .request(
            reqwest::Method::DELETE,
            &format!("/v1/notifications/webhooks/{}", encode(&secret.webhook_id)),
            None,
            None,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(Failure::Http(error)) if error.status == 404 => Ok(()),
        Err(other) => Err(other.into_provider()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_id_round_trip() {
        let metadata = PaymentMetadata {
            workspace_id: "1".into(),
            order_id: "22".into(),
            order_payment_id: "333".into(),
        };
        assert_eq!(
            decode_custom_id(Some(&encode_custom_id(&metadata))),
            Some(metadata)
        );
        assert_eq!(decode_custom_id(Some("1:2")), None);
        assert_eq!(decode_custom_id(Some("1:2:x")), None);
        assert_eq!(decode_custom_id(None), None);
    }

    #[test]
    fn order_status_mapping() {
        let paid = json!({ "id": "o", "status": "COMPLETED", "purchase_units": [{ "payments": { "captures": [{ "id": "cap", "status": "COMPLETED", "amount": { "currency_code": "eur", "value": "12.50" } }] } }] });
        assert_eq!(
            to_checkout_status(&paid).unwrap(),
            CheckoutStatus::Paid {
                transaction_id: "cap".into(),
                amount_minor: 1250,
                currency: "EUR".into()
            }
        );
        let pending = json!({ "status": "COMPLETED", "purchase_units": [{ "payments": { "captures": [{ "status": "PENDING" }] } }] });
        assert_eq!(
            to_checkout_status(&pending).unwrap(),
            CheckoutStatus::Processing
        );
        assert_eq!(
            to_checkout_status(&json!({ "status": "VOIDED" })).unwrap(),
            CheckoutStatus::Closed
        );
        assert_eq!(
            to_checkout_status(&json!({ "status": "APPROVED" })).unwrap(),
            CheckoutStatus::Open
        );
    }

    #[test]
    fn http_error_reads_the_first_detail() {
        let error = HttpError::new(
            422,
            json!({ "name": "UNPROCESSABLE_ENTITY", "details": [{ "issue": "INSTRUMENT_DECLINED", "description": "Declined." }] }),
        );
        assert_eq!(error.error.kind, ErrorKind::InvalidRequest);
        assert_eq!(
            error.error.provider_code.as_deref(),
            Some("INSTRUMENT_DECLINED")
        );
        assert_eq!(error.error.message, "Declined.");
        assert!(error.has_issue("INSTRUMENT_DECLINED"));
        assert_eq!(
            HttpError::new(503, json!({})).error.kind,
            ErrorKind::Unavailable
        );
        assert_eq!(
            HttpError::new(401, json!({})).error.kind,
            ErrorKind::CredentialsRejected
        );
    }
}
