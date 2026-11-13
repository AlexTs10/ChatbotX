//! Port of `packages/database/__tests__/integration-whatsapp-partial.test.ts` (P3.2b,
//! ported: `toEqual(value)` compares the serialized parse with the input; the "typed back"
//! test checks the Rust field types).

use chatbotx_db::repo::partials::integration_whatsapp::{
    TextOrNumber, parse_whatsapp_registration_error,
};
use serde_json::json;

#[test]
fn whatsapp_registration_error_schema_accepts_the_full_shape_stored_on_integration_whatsapp_registration_error()
 {
    let value = json!({
        "code": 100,
        "subCode": 2_593_005,
        "message": "Invalid parameter",
        "type": "OAuthException",
        "userTitle": "Phone number is not verified",
        "userMessage": "Phone number is not verified through SMS or voice.",
        "fbtraceId": "trace-1",
        "at": "2026-07-27T08:00:00.000Z",
    });
    let parsed = parse_whatsapp_registration_error(&value).expect("parse");
    assert_eq!(serde_json::to_value(&parsed).expect("json"), value);
}

#[test]
fn whatsapp_registration_error_schema_accepts_the_minimal_required_shape_string_code_null_sub_code_no_optional_fields()
 {
    let value = json!({
        "code": "systemError",
        "subCode": null,
        "message": "Something went wrong",
        "at": "2026-07-27T08:00:00.000Z",
    });
    let parsed = parse_whatsapp_registration_error(&value).expect("parse");
    assert_eq!(serde_json::to_value(&parsed).expect("json"), value);
}

#[test]
fn whatsapp_registration_error_schema_rejects_a_payload_missing_the_required_message_field() {
    let value = json!({ "code": 1, "subCode": null, "at": "2026-01-01T00:00:00.000Z" });
    assert!(parse_whatsapp_registration_error(&value).is_err());
}

#[test]
fn whatsapp_registration_error_schema_stays_in_sync_with_the_column_s_declared_type_a_parsed_value_satisfies_integration_whatsapp_registration_error()
 {
    let value = json!({
        "code": 1,
        "subCode": null,
        "message": "x",
        "at": "2026-01-01T00:00:00.000Z",
    });
    let parsed = parse_whatsapp_registration_error(&value).expect("parse");
    assert_eq!(parsed.code, TextOrNumber::Number(1.into()));
    assert_eq!(parsed.sub_code, None);
}
