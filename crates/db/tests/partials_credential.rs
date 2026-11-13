//! Port of `packages/database/__tests__/credential.test.ts` (P3.2b, ported: `.parse` is
//! `parse_..` on a JSON value; `toEqual` / `not.toHaveProperty` compare the serialized
//! JSON; each `test.each` row is one test).

use chatbotx_db::repo::partials::credential::{
    parse_giphy_credential_update, parse_google_ads_credential, parse_google_ads_credential_public,
    parse_google_ads_credential_update, parse_google_credential, parse_google_credential_public,
    parse_google_credential_update, parse_instagram_credential_update,
    parse_messenger_credential_update, parse_smtp_credential_update,
    parse_stripe_credential_update, parse_tiktok_credential_update,
    parse_whatsapp_credential_update, parse_zalo_credential_update,
};
use chatbotx_db::repo::partials::google_ads::GoogleAdsUploadMethod;
use serde::Serialize;
use serde_json::{Value, json};

fn assert_trimmed<T: Serialize>(parsed: chatbotx_db::repo::Result<T>) {
    let json = serde_json::to_value(parsed.expect("parse")).expect("json");
    let object = json.as_object().expect("object");
    for value in object.values() {
        if let Some(text) = value.as_str() {
            assert_eq!(text, text.trim(), "{json}");
        }
    }
}

fn platform(extra: Value) -> Value {
    let mut value = json!({
        "clientId": " client-id ",
        "version": " v25.0 ",
        "verifyToken": " verify-token ",
        "clientSecret": " client-secret ",
    });
    for (key, field) in extra.as_object().expect("object") {
        value[key] = field.clone();
    }
    value
}

#[test]
fn credential_update_schemas_trims_whatsapp_credential_values() {
    assert_trimmed(parse_whatsapp_credential_update(&platform(json!({
        "configId": " config-id ",
        "systemUserId": " system-user-id ",
        "businessId": " business-id ",
        "businessName": " business name ",
        "systemUserToken": " system-user-token ",
    }))));
}

#[test]
fn credential_update_schemas_trims_messenger_credential_values() {
    assert_trimmed(parse_messenger_credential_update(&platform(json!({}))));
}

#[test]
fn credential_update_schemas_trims_google_credential_values() {
    assert_trimmed(parse_google_credential_update(&json!({
        "clientId": " client-id ",
        "clientSecret": " client-secret ",
        "verifyToken": " verify-token ",
    })));
}

#[test]
fn credential_update_schemas_trims_instagram_credential_values() {
    assert_trimmed(parse_instagram_credential_update(&platform(json!({}))));
}

#[test]
fn credential_update_schemas_trims_zalo_credential_values() {
    assert_trimmed(parse_zalo_credential_update(&platform(json!({}))));
}

#[test]
fn credential_update_schemas_trims_giphy_credential_values() {
    assert_trimmed(parse_giphy_credential_update(
        &json!({ "apiKey": " api-key " }),
    ));
}

#[test]
fn credential_update_schemas_trims_stripe_credential_values() {
    assert_trimmed(parse_stripe_credential_update(&json!({
        "publishableKey": " publishable-key ",
        "verifyToken": " verify-token ",
        "secretKey": " secret-key ",
        "connectClientId": " ca_client123 ",
    })));
}

#[test]
fn credential_update_schemas_trims_tiktok_credential_values() {
    assert_trimmed(parse_tiktok_credential_update(&json!({
        "clientId": " client-id ",
        "clientSecret": " client-secret ",
    })));
}

#[test]
fn credential_update_schemas_rejects_blank_required_values() {
    let result = parse_messenger_credential_update(&json!({
        "clientId": "   ",
        "version": "v25.0",
        "verifyToken": "verify-token",
        "clientSecret": "client-secret",
    }));
    assert!(result.is_err());
}

#[test]
fn credential_update_schemas_preserves_optional_values_while_trimming_them_when_provided() {
    let result = parse_whatsapp_credential_update(&json!({
        "clientId": "client-id",
        "version": "v25.0",
        "configId": "config-id",
        "systemUserId": "system-user-id",
        "businessId": " business-id ",
        "businessName": "business name",
        "verifyToken": "verify-token",
        "clientSecret": "client-secret",
        "systemUserToken": "system-user-token",
    }))
    .expect("parse");
    assert_eq!(result.business_id.as_deref(), Some("business-id"));
}

#[test]
fn credential_update_schemas_keeps_smtp_validation_for_trimmed_email_and_coerced_port() {
    let result = parse_smtp_credential_update(&json!({
        "host": " smtp.example.com ",
        "port": "2525",
        "username": " username ",
        "password": "",
        "fromEmail": " sender@example.com ",
        "fromName": " Sender ",
    }))
    .expect("parse");
    assert_eq!(result.host, "smtp.example.com");
    assert_eq!(result.port, 2525);
    assert_eq!(result.username, "username");
    assert_eq!(result.from_email, "sender@example.com");
    assert_eq!(result.from_name.as_deref(), Some("Sender"));
}

fn ads_base() -> Value {
    json!({
        "clientId": "client-id",
        "clientSecret": "client-secret",
        "developerToken": "dev-token",
    })
}

fn without(value: &Value, key: &str) -> Value {
    let mut value = value.clone();
    value.as_object_mut().expect("object").remove(key);
    value
}

fn with(value: &Value, key: &str, field: Value) -> Value {
    let mut value = value.clone();
    value[key] = field;
    value
}

fn to_json<T: Serialize>(parsed: chatbotx_db::repo::Result<T>) -> Value {
    serde_json::to_value(parsed.expect("parse")).expect("json")
}

#[test]
fn google_ads_credential_requires_client_id_and_client_secret_the_developer_token_is_optional() {
    let base = ads_base();
    assert_eq!(to_json(parse_google_ads_credential(&base)), base);
    for field in ["clientId", "clientSecret"] {
        assert!(
            parse_google_ads_credential(&without(&base, field)).is_err(),
            "{field}"
        );
    }
    let without_token = without(&base, "developerToken");
    assert_eq!(
        to_json(parse_google_ads_credential(&without_token)),
        without_token
    );
}

#[test]
fn google_ads_credential_a_stored_credential_without_upload_method_still_parses_absent_data_manager()
 {
    let parsed = parse_google_ads_credential(&ads_base()).expect("parse");
    assert_eq!(parsed.upload_method, None);
}

#[test]
fn google_ads_credential_accepts_upload_method_data_manager() {
    let parsed =
        parse_google_ads_credential(&with(&ads_base(), "uploadMethod", json!("dataManager")))
            .expect("parse");
    assert_eq!(
        parsed.upload_method,
        Some(GoogleAdsUploadMethod::DataManager)
    );
}

#[test]
fn google_ads_credential_accepts_upload_method_legacy() {
    let parsed = parse_google_ads_credential(&with(&ads_base(), "uploadMethod", json!("legacy")))
        .expect("parse");
    assert_eq!(parsed.upload_method, Some(GoogleAdsUploadMethod::Legacy));
}

#[test]
fn google_ads_credential_rejects_an_unknown_upload_method() {
    assert!(
        parse_google_ads_credential(&with(&ads_base(), "uploadMethod", json!("soap"))).is_err()
    );
}

#[test]
fn google_ads_credential_the_public_projection_exposes_the_client_id_and_upload_method_only() {
    let base = ads_base();
    assert_eq!(
        to_json(parse_google_ads_credential_public(&base)),
        json!({ "clientId": "client-id", "hasDeveloperToken": true })
    );
    assert_eq!(
        to_json(parse_google_ads_credential_public(&without(
            &base,
            "developerToken"
        ))),
        json!({ "clientId": "client-id", "hasDeveloperToken": false })
    );
    assert_eq!(
        to_json(parse_google_ads_credential_public(&with(
            &base,
            "uploadMethod",
            json!("legacy")
        ))),
        json!({ "clientId": "client-id", "uploadMethod": "legacy", "hasDeveloperToken": true })
    );
}

#[test]
fn google_ads_credential_the_update_schema_trims_values_and_treats_the_secrets_as_optional() {
    assert_eq!(
        to_json(parse_google_ads_credential_update(&json!({
            "clientId": " client-id ",
            "clientSecret": " secret ",
            "developerToken": " token ",
        }))),
        json!({ "clientId": "client-id", "clientSecret": "secret", "developerToken": "token" })
    );
    assert_eq!(
        to_json(parse_google_ads_credential_update(
            &json!({ "clientId": "client-id" })
        )),
        json!({ "clientId": "client-id" })
    );
    assert_eq!(
        to_json(parse_google_ads_credential_update(&json!({
            "clientId": "client-id",
            "uploadMethod": "legacy",
        }))),
        json!({ "clientId": "client-id", "uploadMethod": "legacy" })
    );
    assert!(
        parse_google_ads_credential_update(
            &json!({ "clientId": "client-id", "uploadMethod": "x" })
        )
        .is_err()
    );
    assert!(parse_google_ads_credential_update(&json!({ "clientId": " " })).is_err());
}

#[test]
fn google_ads_credential_the_google_credential_no_longer_carries_an_ads_developer_token() {
    let parsed = to_json(parse_google_credential(&json!({
        "clientId": "c",
        "clientSecret": "s",
        "verifyToken": "v",
        "adsDeveloperToken": "t",
    })));
    assert!(parsed.get("adsDeveloperToken").is_none(), "{parsed}");
    assert_eq!(
        to_json(parse_google_credential_public(&parsed)),
        json!({ "clientId": "c" })
    );
}
