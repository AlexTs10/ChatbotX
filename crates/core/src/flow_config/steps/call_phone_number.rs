//! Port of `packages/flow-config/src/steps/call-phone-number.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const PHONE_NUMBER_EXPRESSION_MAX: usize = 100;

schema!(pub fn call_phone_number_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("callPhoneNumber")).describe("Step type discriminator: \"callPhoneNumber\".")),
("phoneNumber", z::string().trim().refine(is_valid_authored_phone_number, Some("invalidPhoneNumber"), None).describe("Number the button dials, in international format (E.164, e.g. +14155552671). May embed {{variables}} (max 100 characters) that resolve to such a number when the message is sent."))]));

default_fn!(pub fn call_phone_number_step_default_fn = json!({"id": create_id(), "stepType": "callPhoneNumber", "phoneNumber": ""}));

/// `compactPhoneNumber`: drops spaces, `-`, `.`, `(`, `)`.
pub fn compact_phone_number(value: &str) -> String {
    value
        .chars()
        .filter(|c| !(c.is_whitespace() || matches!(c, '-' | '.' | '(' | ')')))
        .collect()
}

/// `normalizeDialablePhoneNumber`: the E.164 form of a valid international
/// number (libphonenumber validity), else `None`.
pub fn normalize_dialable_phone_number(value: &str) -> Option<String> {
    let compact = compact_phone_number(value);
    let digits = compact.strip_prefix('+')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parsed = phonenumber::parse(None, &compact).ok()?;
    if !phonenumber::is_valid(&parsed) {
        return None;
    }
    Some(parsed.format().mode(phonenumber::Mode::E164).to_string())
}

fn is_e164_phone_number(value: &str) -> bool {
    normalize_dialable_phone_number(value).as_deref() == Some(value)
}

fn is_valid_authored_phone_number(value: &Value) -> bool {
    let text = value.as_str().unwrap_or("");
    if crate::utils::variables::contains_variable_placeholder(text) {
        crate::js::utf16_len(text) <= PHONE_NUMBER_EXPRESSION_MAX
    } else {
        is_e164_phone_number(text)
    }
}
