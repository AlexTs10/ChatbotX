//! Port of `packages/database/__tests__/whatsapp-credential-schema.test.ts` (P3.2b,
//! ported: `.parse` is `parse_..` on a JSON value; `not.toHaveProperty` checks the
//! serialized JSON).

use chatbotx_db::repo::partials::credential::{
    parse_whatsapp_credential, parse_whatsapp_credential_public, parse_whatsapp_credential_update,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "clientId": "app-id",
        "version": "v25.0",
        "configId": "signup-config",
        "systemUserId": "system-user",
        "businessName": "Acme",
        "verifyToken": "verify",
        "clientSecret": "secret",
        "systemUserToken": "system-token",
    })
}

fn with_config_id(id: &str) -> Value {
    let mut value = base();
    value["capiConfigId"] = json!(id);
    value
}

#[test]
fn whatsapp_credential_schema_capi_config_id_parses_a_stored_credential_that_predates_the_field() {
    let parsed = parse_whatsapp_credential(&base()).expect("parse");
    assert_eq!(parsed.capi_config_id, None);
}

#[test]
fn whatsapp_credential_schema_capi_config_id_round_trips_the_config_id_when_present() {
    let parsed = parse_whatsapp_credential(&with_config_id("1234567890")).expect("parse");
    assert_eq!(parsed.capi_config_id.as_deref(), Some("1234567890"));
}

#[test]
fn whatsapp_credential_schema_capi_config_id_exposes_the_config_id_publicly_but_never_a_secret() {
    let parsed = parse_whatsapp_credential_public(&with_config_id("1234567890")).expect("parse");
    assert_eq!(parsed.capi_config_id.as_deref(), Some("1234567890"));
    let json = serde_json::to_value(&parsed).expect("json");
    assert!(json.get("clientSecret").is_none(), "{json}");
    assert!(json.get("systemUserToken").is_none(), "{json}");
}

#[test]
fn whatsapp_credential_schema_capi_config_id_update_schema_accepts_a_submission_from_an_admin_who_has_not_configured_it()
 {
    let parsed = parse_whatsapp_credential_update(&base()).expect("parse");
    assert_eq!(parsed.capi_config_id, None);
}

#[test]
fn whatsapp_credential_schema_capi_config_id_update_schema_trims_the_config_id() {
    let parsed =
        parse_whatsapp_credential_update(&with_config_id("  1234567890  ")).expect("parse");
    assert_eq!(parsed.capi_config_id.as_deref(), Some("1234567890"));
}
