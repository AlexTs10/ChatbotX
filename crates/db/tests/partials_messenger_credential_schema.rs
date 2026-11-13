//! Port of `packages/database/__tests__/messenger-credential-schema.test.ts` (P3.2b,
//! ported: `.parse` is `parse_..` on a JSON value; `not.toHaveProperty` checks the
//! serialized JSON).

use chatbotx_db::repo::partials::credential::{
    parse_messenger_credential, parse_messenger_credential_public,
    parse_messenger_credential_update,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "clientId": "app-id",
        "version": "v25.0",
        "verifyToken": "verify",
        "clientSecret": "secret",
    })
}

fn with_config_id() -> Value {
    let mut value = base();
    value["marketingMessagesConfigId"] = json!("1234567890");
    value
}

#[test]
fn messenger_credential_schema_marketing_messages_config_id_parses_a_stored_credential_that_predates_the_field()
 {
    let parsed = parse_messenger_credential(&base()).expect("parse");
    assert_eq!(parsed.marketing_messages_config_id, None);
}

#[test]
fn messenger_credential_schema_marketing_messages_config_id_round_trips_the_config_id_when_present()
{
    let parsed = parse_messenger_credential(&with_config_id()).expect("parse");
    assert_eq!(
        parsed.marketing_messages_config_id.as_deref(),
        Some("1234567890")
    );
}

#[test]
fn messenger_credential_schema_marketing_messages_config_id_exposes_the_config_id_publicly_but_never_the_secret()
 {
    let parsed = parse_messenger_credential_public(&with_config_id()).expect("parse");
    assert_eq!(
        parsed.marketing_messages_config_id.as_deref(),
        Some("1234567890")
    );
    let json = serde_json::to_value(&parsed).expect("json");
    assert!(json.get("clientSecret").is_none(), "{json}");
}

#[test]
fn messenger_credential_schema_marketing_messages_config_id_update_schema_accepts_an_empty_submission_from_an_admin_who_has_not_configured_it()
 {
    let parsed = parse_messenger_credential_update(&base()).expect("parse");
    assert_eq!(parsed.marketing_messages_config_id, None);
}
