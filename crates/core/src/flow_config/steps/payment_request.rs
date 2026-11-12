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

// HAND-PORT createEmptyPaymentRequest:
// method=>({...paymentRequestMethodRules[method].emptyRequest})

// HAND-PORT paymentAmountToMinorUnits:
// function paymentAmountToMinorUnits(amount,fractionDigits=DEFAULT_AMOUNT_FRACTION_DIGITS){const match=DECIMAL_AMOUNT_REGEX.exec(amount.trim());if(!match){return null}const[,wholePart="",fractionPart=""]=match;if(fractionPart.length>fractionDigits){return null}const digits=`${wholePart}${fractionPart.padEnd(fractionDigits,"0")}`;if(digits.length>MAX_AMOUNT_DIGITS){return null}const minorUnits=Number(digits);return Number.isSafeInteger(minorUnits)&&minorUnits>0?minorUnits:null}

// HAND-PORT readPaymentRequestField:
// (request,field)=>{const values=request;return values[field]}

// HAND-PORT collectPaymentRequestIssues:
// function collectPaymentRequestIssues(request,options){const{requiredFields}=paymentRequestMethodRules[request.method];return paymentRequestFieldNames(request.method).flatMap(field=>{const kind=classifyField(readPaymentRequestField(request,field),field,requiredFields.includes(field),options);return kind===null?[]:[{field,kind}]})}

// HAND-PORT paymentRequestFieldNames:
// method=>{const rule=paymentRequestMethodRules[method];return[...rule.requiredFields,...rule.optionalFields]}

// ---- stage 1 API (hand-ported in stage 2) ----

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

pub fn collect_payment_request_issues(
    request: &Value,
    options: CollectPaymentRequestIssuesOptions,
) -> Vec<PaymentRequestIssue> {
    todo!("P3.4")
}

pub fn create_empty_payment_request(method: &str) -> Value {
    todo!("P3.4")
}

pub fn payment_amount_to_minor_units(amount: &str, fraction_digits: Option<u32>) -> Option<i64> {
    todo!("P3.4")
}
