//! Port of `packages/flow-config/src/steps/payment-request.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn payment_request_methods = z::enum_(&["pix", "boleto", "payment_link", "card_on_file"]));

schema!(pub fn payment_request_schema = z::discriminated_union("method", vec![z::object(vec![("method", z::literal(json!("pix"))),
("code", z::string().describe("Pix dynamic code (copy & paste payload).")),
("amount", z::string().describe("Amount in major units, e.g. `12.34`. Server converts it to minor units.").optional().describe("Optional amount shown to the customer."))]).strict(),
z::object(vec![("method", z::literal(json!("boleto"))),
("digitableLine", z::string().describe("Boleto digitable line."))]).strict(),
z::object(vec![("method", z::literal(json!("payment_link"))),
("url", z::string().describe("Payment link URL."))]).strict(),
z::object(vec![("method", z::literal(json!("card_on_file"))),
("lastFourDigits", z::string().describe("Last four digits of the saved card.")),
("credentialId", z::string().optional().describe("Identifier of the saved card credential.")),
("amount", z::string().describe("Amount to charge."))]).strict()]));

pub fn payment_request_method_rules() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"pix":{"requiredFields":["code"],"optionalFields":["amount"],"emptyRequest":{"method":"pix","code":""}},"boleto":{"requiredFields":["digitableLine"],"optionalFields":[],"emptyRequest":{"method":"boleto","digitableLine":""}},"payment_link":{"requiredFields":["url"],"optionalFields":[],"emptyRequest":{"method":"payment_link","url":""}},"card_on_file":{"requiredFields":["lastFourDigits","amount"],"optionalFields":["credentialId"],"emptyRequest":{"method":"card_on_file","lastFourDigits":"","amount":""}}}"#).expect("valid JSON")
    });
    &V
}

/// `CollectPaymentRequestIssuesOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectPaymentRequestIssuesOptions {
    pub allow_variables: bool,
}

/// `PaymentRequestIssue`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentRequestIssue {
    pub field: String,
    pub kind: &'static str,
}

/// `createEmptyPaymentRequest(method)`: a fresh copy of the method's seed.
pub fn create_empty_payment_request(method: &str) -> Value {
    payment_request_method_rules()[method]["emptyRequest"].clone()
}

const DEFAULT_AMOUNT_FRACTION_DIGITS: u32 = 2;
const MAX_AMOUNT_DIGITS: usize = 16;

fn decimal_amount_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^([0-9]+)(?:\.([0-9]+))?$").expect("amount regex"))
}

/// `paymentAmountToMinorUnits(amount, fractionDigits = 2)`: a positive safe
/// integer of minor units, or `None`.
pub fn payment_amount_to_minor_units(amount: &str, fraction_digits: Option<u32>) -> Option<i64> {
    let fraction_digits = fraction_digits.unwrap_or(DEFAULT_AMOUNT_FRACTION_DIGITS) as usize;
    let captures = decimal_amount_re().captures(crate::js::trim(amount))?;
    let whole = captures.get(1).map_or("", |m| m.as_str());
    let fraction = captures.get(2).map_or("", |m| m.as_str());
    if fraction.len() > fraction_digits {
        return None;
    }
    let digits = format!("{whole}{fraction:0<fraction_digits$}");
    if digits.len() > MAX_AMOUNT_DIGITS {
        return None;
    }
    let minor_units: f64 = digits.parse().ok()?;
    (minor_units > 0.0 && minor_units <= 9_007_199_254_740_991.0).then_some(minor_units as i64)
}

/// `readPaymentRequestField(request, field)`.
pub fn read_payment_request_field<'a>(request: &'a Value, field: &str) -> Option<&'a str> {
    request.get(field).and_then(Value::as_str)
}

/// `paymentRequestFieldNames(method)`: required then optional fields.
pub fn payment_request_field_names(method: &str) -> Vec<String> {
    let rule = &payment_request_method_rules()[method];
    arr_field(rule, "requiredFields")
        .iter()
        .chain(arr_field(rule, "optionalFields"))
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn field_format_ok(field: &str, value: &str) -> Option<bool> {
    match field {
        "url" => Some(crate::utils::zod::is_url(value)),
        "lastFourDigits" => Some(value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit())),
        "amount" => Some(payment_amount_to_minor_units(value, None).is_some()),
        _ => None,
    }
}

fn classify_field(
    value: Option<&str>,
    field: &str,
    is_required: bool,
    options: CollectPaymentRequestIssuesOptions,
) -> Option<&'static str> {
    let trimmed = value.map(crate::js::trim).unwrap_or("");
    if trimmed.is_empty() {
        return is_required.then_some("required");
    }
    if crate::utils::variables::contains_variable_placeholder(trimmed) {
        return (!options.allow_variables).then_some("unresolved");
    }
    (field_format_ok(field, trimmed) == Some(false)).then_some("invalid")
}

/// `collectPaymentRequestIssues(request, options)`.
pub fn collect_payment_request_issues(
    request: &Value,
    options: CollectPaymentRequestIssuesOptions,
) -> Vec<PaymentRequestIssue> {
    let method = str_field(request, "method");
    let rule = &payment_request_method_rules()[method];
    let required: Vec<&str> = arr_field(rule, "requiredFields")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    payment_request_field_names(method)
        .into_iter()
        .filter_map(|field| {
            let kind = classify_field(
                read_payment_request_field(request, &field),
                &field,
                required.contains(&field.as_str()),
                options,
            )?;
            Some(PaymentRequestIssue { field, kind })
        })
        .collect()
}
