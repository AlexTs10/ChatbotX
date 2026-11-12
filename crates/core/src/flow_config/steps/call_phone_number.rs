//! Port of `packages/flow-config/src/steps/call-phone-number.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const PHONE_NUMBER_EXPRESSION_MAX: usize = 100;

// HAND-PORT compactPhoneNumber:
// value=>value.replace(PHONE_SEPARATORS,"")

// HAND-PORT normalizeDialablePhoneNumber:
// value=>{const compact=compactPhoneNumber(value);if(!INTERNATIONAL_NUMBER.test(compact)){return null}const parsed=parsePhoneNumberFromString(compact);return parsed?.isValid()?parsed.number:null}

schema!(pub fn call_phone_number_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("callPhoneNumber")).describe("Step type discriminator: \"callPhoneNumber\".")),
("phoneNumber", z::string().trim().refine(is_valid_authored_phone_number, Some("invalidPhoneNumber"), None).describe("Number the button dials, in international format (E.164, e.g. +14155552671). May embed {{variables}} (max 100 characters) that resolve to such a number when the message is sent."))]));

default_fn!(pub fn call_phone_number_step_default_fn = json!({"id": create_id(), "stepType": "callPhoneNumber", "phoneNumber": ""}));

// value=>containsVariablePlaceholder(value)?value.length<=PHONE_NUMBER_EXPRESSION_MAX:isE164PhoneNumber(value)
fn is_valid_authored_phone_number(value: &Value) -> bool {
    todo!("P3.4")
}
