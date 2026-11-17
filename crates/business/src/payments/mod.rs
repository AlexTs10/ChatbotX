//! `packages/payments`: the store's Stripe and PayPal accounts (checkout
//! pages, status reads, refunds, webhook checks) and the minor-unit maths.
//!
//! The provider hosts are constants (`api.stripe.com`, `api-m[.sandbox].paypal.com`),
//! never a URL a user typed, so no SSRF check is needed on these calls.

pub mod paypal;
pub mod stripe;

use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// `PaymentProviderErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    CredentialsRejected,
    Unavailable,
    AmountUnsupported,
    SignatureInvalid,
    InvalidRequest,
}

/// `PaymentProviderError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
    /// The provider's own code (`amount_too_small`, `INSTRUMENT_DECLINED`…).
    pub provider_code: Option<String>,
}

impl ProviderError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> ProviderError {
        ProviderError {
            kind,
            message: message.into(),
            provider_code: None,
        }
    }

    pub fn code(mut self, code: Option<String>) -> ProviderError {
        self.provider_code = code;
        self
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type PResult<T> = std::result::Result<T, ProviderError>;

/// `PaymentMetadata`: who an attempt belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentMetadata {
    pub workspace_id: String,
    pub order_id: String,
    pub order_payment_id: String,
}

/// `CreateCheckoutInput`.
#[derive(Debug, Clone)]
pub struct CreateCheckout {
    /// Major units in `currency`, exactly the order's total.
    pub amount: f64,
    pub currency: String,
    pub description: String,
    pub return_url: String,
    pub cancel_url: String,
    /// Unix ms.
    pub expires_at: i64,
    pub customer_email: Option<String>,
    pub metadata: PaymentMetadata,
    pub idempotency_key: String,
}

/// `CreatedCheckout`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedCheckout {
    pub reference: String,
    pub url: String,
}

/// `CheckoutStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutStatus {
    Open,
    Processing,
    Closed,
    Paid {
        transaction_id: String,
        amount_minor: i64,
        currency: String,
    },
}

impl CheckoutStatus {
    pub fn state(&self) -> &'static str {
        match self {
            CheckoutStatus::Open => "open",
            CheckoutStatus::Processing => "processing",
            CheckoutStatus::Closed => "closed",
            CheckoutStatus::Paid { .. } => "paid",
        }
    }
}

/// `PaymentWebhookEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    Checkout {
        event_id: String,
        reference: String,
        metadata: Option<PaymentMetadata>,
        /// Stripe Connect: the account it came from.
        account: Option<String>,
    },
    Deauthorized {
        event_id: String,
        account: String,
        /// Unix ms.
        created_at: i64,
    },
    Ignored {
        event_id: String,
    },
}

/// `PaypalSecret`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaypalSecret {
    pub client_id: String,
    pub client_secret: String,
    pub livemode: bool,
    pub webhook_id: String,
}

/// `StripeKeySecret`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripeKeySecret {
    pub secret_key: String,
    pub livemode: bool,
    pub account_id: String,
    pub webhook_id: String,
    pub webhook_secret: String,
}

/// `PaymentGateway`: one store's account at one provider.
#[derive(Debug, Clone)]
pub enum Gateway {
    Stripe(stripe::StripeAccount),
    Paypal(paypal::Paypal),
}

impl Gateway {
    pub fn provider(&self) -> &'static str {
        match self {
            Gateway::Stripe(_) => "stripe",
            Gateway::Paypal(_) => "paypal",
        }
    }

    pub fn to_minor_units(&self, amount: f64, currency: &str) -> PResult<i64> {
        match self {
            Gateway::Stripe(_) => to_stripe_minor_units(amount, currency),
            Gateway::Paypal(_) => to_paypal_minor_units(amount, currency),
        }
    }

    pub async fn create_checkout(&self, input: &CreateCheckout) -> PResult<CreatedCheckout> {
        match self {
            Gateway::Stripe(s) => s.create_checkout(input).await,
            Gateway::Paypal(p) => p.create_checkout(input).await,
        }
    }

    /// `getCheckoutStatus`; `capture` = the PayPal capture idempotency key.
    pub async fn get_checkout_status(
        &self,
        reference: &str,
        capture: Option<&str>,
    ) -> PResult<CheckoutStatus> {
        match self {
            Gateway::Stripe(s) => s.get_checkout_status(reference).await,
            Gateway::Paypal(p) => p.get_checkout_status(reference, capture).await,
        }
    }

    pub async fn expire_checkout(&self, reference: &str) -> PResult<CheckoutStatus> {
        match self {
            Gateway::Stripe(s) => s.expire_checkout(reference).await,
            Gateway::Paypal(p) => p.expire_checkout(reference).await,
        }
    }

    /// A full refund; `Ok(None)` = the provider says it was already refunded.
    pub async fn refund(
        &self,
        transaction_id: &str,
        idempotency_key: &str,
    ) -> PResult<Option<String>> {
        match self {
            Gateway::Stripe(s) => s.refund(transaction_id, idempotency_key).await,
            Gateway::Paypal(p) => p.refund(transaction_id, idempotency_key).await,
        }
    }
}

/// One HTTP client for both providers: 20 s per request (`timeout: 20_000`).
pub(crate) fn http() -> PResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| ProviderError::new(ErrorKind::Unavailable, e.clone()))
}

// ---- minor-units.ts ---------------------------------------------------------

const STRIPE_ZERO_DECIMAL: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "JPY", "KMF", "KRW", "MGA", "PYG", "RWF", "VND", "VUV", "XAF",
    "XOF", "XPF",
];
const STRIPE_THREE_DECIMAL: &[&str] = &["BHD", "JOD", "KWD", "OMR", "TND"];
const STRIPE_WHOLE_UNITS_AS_TWO_DECIMAL: &[&str] = &["ISK", "UGX"];
const PAYPAL_ZERO_DECIMAL: &[&str] = &["HUF", "JPY", "TWD"];

/// `PAYPAL_SUPPORTED_CURRENCIES`.
pub const PAYPAL_SUPPORTED_CURRENCIES: &[&str] = &[
    "AUD", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD", "HUF", "ILS", "JPY",
    "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "SEK", "SGD", "THB", "TWD", "USD",
];

pub fn is_paypal_currency(currency: &str) -> bool {
    PAYPAL_SUPPORTED_CURRENCIES.contains(&currency.to_uppercase().as_str())
}

const EPSILON: f64 = 1e-6;
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

fn unsupported(amount: f64, currency: &str) -> ProviderError {
    ProviderError::new(
        ErrorKind::AmountUnsupported,
        format!(
            "{} {currency} cannot be charged exactly in this currency.",
            crate::payments::js_number(amount)
        ),
    )
    .code(Some("amount_not_representable".into()))
}

/// JS `String(number)` for a finite amount.
pub(crate) fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn scale_exactly(amount: f64, digits: i32, currency: &str) -> PResult<i64> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(unsupported(amount, currency));
    }
    let scaled = amount * 10f64.powi(digits);
    let minor = scaled.round();
    if (scaled - minor).abs() > EPSILON || minor.abs() > MAX_SAFE {
        return Err(unsupported(amount, currency));
    }
    Ok(minor as i64)
}

/// `toStripeMinorUnits`.
pub fn to_stripe_minor_units(amount: f64, currency: &str) -> PResult<i64> {
    let code = currency.to_uppercase();
    if STRIPE_ZERO_DECIMAL.contains(&code.as_str()) {
        return scale_exactly(amount, 0, &code);
    }
    if STRIPE_WHOLE_UNITS_AS_TWO_DECIMAL.contains(&code.as_str()) {
        return Ok(scale_exactly(amount, 0, &code)? * 100);
    }
    if STRIPE_THREE_DECIMAL.contains(&code.as_str()) {
        let minor = scale_exactly(amount, 3, &code)?;
        if minor % 10 != 0 {
            return Err(unsupported(amount, &code));
        }
        return Ok(minor);
    }
    scale_exactly(amount, 2, &code)
}

fn paypal_digits(currency: &str) -> i32 {
    if PAYPAL_ZERO_DECIMAL.contains(&currency.to_uppercase().as_str()) {
        0
    } else {
        2
    }
}

/// `toPaypalMinorUnits`.
pub fn to_paypal_minor_units(amount: f64, currency: &str) -> PResult<i64> {
    let code = currency.to_uppercase();
    if !is_paypal_currency(&code) {
        return Err(ProviderError::new(
            ErrorKind::AmountUnsupported,
            format!("PayPal does not take payments in {code}."),
        )
        .code(Some("currency_not_supported".into())));
    }
    scale_exactly(amount, paypal_digits(&code), &code)
}

/// `formatPaypalValue`.
pub fn format_paypal_value(minor: i64, currency: &str) -> String {
    if paypal_digits(currency) == 0 {
        return minor.to_string();
    }
    format!("{}.{:02}", minor.div_euclid(100), minor.rem_euclid(100))
}

/// `parsePaypalValue`.
pub fn parse_paypal_value(value: &str, currency: &str) -> PResult<i64> {
    let trimmed = value.trim();
    let (whole, fraction) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    let digits_only = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty()
        || !digits_only(whole)
        || !digits_only(fraction)
        || (trimmed.contains('.') && fraction.is_empty())
    {
        return Err(ProviderError::new(
            ErrorKind::InvalidRequest,
            format!("PayPal sent an amount that is not a number: {value}."),
        ));
    }
    let digits = paypal_digits(currency) as usize;
    let mut fraction = fraction.to_string();
    while fraction.len() < digits {
        fraction.push('0');
    }
    if fraction.len() > digits && fraction[digits..].bytes().any(|b| b != b'0') {
        return Err(ProviderError::new(
            ErrorKind::InvalidRequest,
            format!("PayPal sent more decimals than {currency} has: {value}."),
        ));
    }
    format!("{whole}{}", &fraction[..digits])
        .parse::<i64>()
        .map_err(|_| {
            ProviderError::new(
                ErrorKind::InvalidRequest,
                format!("PayPal sent an amount that is not a number: {value}."),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of packages/payments/__tests__/minor-units.test.ts.
    #[test]
    fn stripe_minor_units() {
        assert_eq!(to_stripe_minor_units(12.5, "usd").unwrap(), 1250);
        assert_eq!(to_stripe_minor_units(0.1 + 0.2, "USD").unwrap(), 30);
        assert_eq!(to_stripe_minor_units(1500.0, "JPY").unwrap(), 1500);
        assert_eq!(to_stripe_minor_units(10.0, "ISK").unwrap(), 1000);
        assert_eq!(to_stripe_minor_units(1.23, "KWD").unwrap(), 1230);
        assert!(to_stripe_minor_units(1.234, "KWD").is_err());
        assert!(to_stripe_minor_units(12.345, "USD").is_err());
        assert!(to_stripe_minor_units(10.5, "JPY").is_err());
        assert!(to_stripe_minor_units(0.0, "USD").is_err());
        let error = to_stripe_minor_units(-1.0, "USD").unwrap_err();
        assert_eq!(error.kind, ErrorKind::AmountUnsupported);
        assert_eq!(
            error.provider_code.as_deref(),
            Some("amount_not_representable")
        );
    }

    #[test]
    fn paypal_minor_units_and_values() {
        assert_eq!(to_paypal_minor_units(12.5, "EUR").unwrap(), 1250);
        assert_eq!(to_paypal_minor_units(1500.0, "JPY").unwrap(), 1500);
        let error = to_paypal_minor_units(10.0, "VND").unwrap_err();
        assert_eq!(
            error.provider_code.as_deref(),
            Some("currency_not_supported")
        );
        assert_eq!(format_paypal_value(1250, "EUR"), "12.50");
        assert_eq!(format_paypal_value(5, "USD"), "0.05");
        assert_eq!(format_paypal_value(1500, "JPY"), "1500");
        assert_eq!(parse_paypal_value("12.50", "EUR").unwrap(), 1250);
        assert_eq!(parse_paypal_value("12.5", "EUR").unwrap(), 1250);
        assert_eq!(parse_paypal_value("12", "EUR").unwrap(), 1200);
        assert_eq!(parse_paypal_value("12.500", "EUR").unwrap(), 1250);
        assert!(parse_paypal_value("12.505", "EUR").is_err());
        assert!(parse_paypal_value("abc", "EUR").is_err());
        assert_eq!(parse_paypal_value("1500", "JPY").unwrap(), 1500);
    }
}
