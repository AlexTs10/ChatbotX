//! Port of `packages/database/__tests__/google-ads-settings-partial.test.ts` (P3.2b,
//! ported: `safeParse(..).success` is `parse_..(..).is_ok()`; `result.error.issues[0]` is
//! the first `ZodError` issue; each `test.each` row is one test).

use chatbotx_db::repo::partials::google_ads::{
    GOOGLE_ADS_IDENTITY_POLICY_VALUES, GoogleAdsConsentSource, NOT_PROVIDED_CONSENT,
    parse_google_ads_consent, parse_google_ads_event_options, parse_google_ads_settings_document,
};
use serde_json::{Value, json};

fn valid_document() -> Value {
    json!({
        "version": 1,
        "consent": {
            "adUserData": { "type": "granted" },
            "adPersonalization": { "type": "variable", "template": "{{gdpr_consent}}" },
        },
    })
}

#[test]
fn google_ads_settings_document_schema_accepts_a_v1_document() {
    assert!(parse_google_ads_settings_document(&valid_document()).is_ok());
}

#[test]
fn google_ads_settings_document_schema_accepts_the_not_provided_default_wrapped_in_v1() {
    let document = json!({
        "version": 1,
        "consent": serde_json::to_value(NOT_PROVIDED_CONSENT).expect("json"),
    });
    assert!(parse_google_ads_settings_document(&document).is_ok());
}

#[test]
fn google_ads_settings_document_schema_rejects_an_unknown_version() {
    let mut document = valid_document();
    document["version"] = json!(2);
    assert!(parse_google_ads_settings_document(&document).is_err());
}

#[test]
fn google_ads_settings_document_schema_rejects_a_missing_version() {
    let document = json!({ "consent": valid_document()["consent"].clone() });
    assert!(parse_google_ads_settings_document(&document).is_err());
}

fn with_template(template: &str) -> Value {
    json!({
        "adUserData": { "type": "variable", "template": template },
        "adPersonalization": { "type": "notProvided" },
    })
}

#[test]
fn google_ads_consent_schema_rejects_a_variable_source_without_a_placeholder() {
    let error = parse_google_ads_consent(&with_template("granted")).expect_err("invalid");
    assert_eq!(
        error.messages().first().copied(),
        Some("googleAds.consent.validation.templateRequired")
    );
}

#[test]
fn google_ads_consent_schema_rejects_an_empty_template() {
    assert!(parse_google_ads_consent(&with_template("  ")).is_err());
}

#[test]
fn google_ads_consent_schema_rejects_a_template_over_200_characters() {
    let template = format!("{{{{field}}}}{}", "x".repeat(200));
    let error = parse_google_ads_consent(&with_template(&template)).expect_err("invalid");
    assert_eq!(
        error.messages().first().copied(),
        Some("googleAds.consent.validation.templateTooLong")
    );
}

#[test]
fn google_ads_consent_schema_strips_a_leftover_template_from_a_non_variable_source() {
    let result = parse_google_ads_consent(&json!({
        "adUserData": { "type": "granted", "template": "{{x}}" },
        "adPersonalization": { "type": "denied" },
    }))
    .expect("parse");
    assert_eq!(result.ad_user_data, GoogleAdsConsentSource::Granted);
    assert_eq!(
        serde_json::to_value(&result.ad_user_data).expect("json"),
        json!({ "type": "granted" })
    );
}

#[test]
fn google_ads_consent_schema_rejects_an_unknown_source_type() {
    assert!(
        parse_google_ads_consent(&json!({
            "adUserData": { "type": "maybe" },
            "adPersonalization": { "type": "denied" },
        }))
        .is_err()
    );
}

fn valid_options() -> Value {
    json!({
        "version": 1,
        "identity": {
            "version": 1,
            "configuredPolicy": "id",
            "effectivePolicy": "id",
            "keySource": "explicit",
            "id": "A-1042",
        },
        "timeSource": "provided",
        "consent": {
            "adUserData": { "status": "granted", "source": "fixed" },
            "adPersonalization": { "status": null, "source": "notProvided" },
        },
    })
}

fn with_id(id: Value) -> Value {
    let mut options = valid_options();
    options["identity"]["id"] = id;
    options
}

fn v2_with(key: &str, block: Value) -> Value {
    let mut options = valid_options();
    options["version"] = json!(2);
    options[key] = block;
    options
}

#[test]
fn google_ads_event_options_schema_accepts_v1_options_with_an_identity_block() {
    assert!(parse_google_ads_event_options(&valid_options()).is_ok());
}

#[test]
fn google_ads_event_options_schema_accepts_a_v2_snapshot_with_customer_matching_sources_and_no_values()
 {
    let options = v2_with(
        "matching",
        json!({ "status": "enabled", "email": "{{email}}", "phone": "{{phone}}" }),
    );
    assert!(parse_google_ads_event_options(&options).is_ok());
}

fn assert_rejects_matching(matching: Value) {
    assert!(parse_google_ads_event_options(&v2_with("matching", matching)).is_err());
}

#[test]
fn google_ads_event_options_schema_rejects_v2_matching_with_an_unknown_status() {
    assert_rejects_matching(json!({ "status": "sent", "email": null, "phone": null }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_matching_with_an_object_source() {
    assert_rejects_matching(json!({
        "status": "enabled",
        "email": { "type": "contact" },
        "phone": null,
    }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_matching_with_an_empty_template() {
    assert_rejects_matching(json!({ "status": "enabled", "email": "", "phone": null }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_matching_with_a_missing_source_key() {
    assert_rejects_matching(json!({ "status": "enabled", "phone": null }));
}

#[test]
fn google_ads_event_options_schema_accepts_a_v2_snapshot_with_customer_properties() {
    let options = v2_with(
        "customerProperties",
        json!({ "status": "enabled", "customerType": "NEW", "customerValueBucket": null }),
    );
    assert!(parse_google_ads_event_options(&options).is_ok());
}

fn assert_rejects_customer_properties(properties: Value) {
    assert!(parse_google_ads_event_options(&v2_with("customerProperties", properties)).is_err());
}

#[test]
fn google_ads_event_options_schema_rejects_v2_customer_properties_with_an_unknown_status() {
    assert_rejects_customer_properties(json!({
        "status": "sent",
        "customerType": null,
        "customerValueBucket": null,
    }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_customer_properties_with_an_unknown_type() {
    assert_rejects_customer_properties(json!({
        "status": "enabled",
        "customerType": "VIP",
        "customerValueBucket": null,
    }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_customer_properties_with_a_lowercase_bucket() {
    assert_rejects_customer_properties(json!({
        "status": "enabled",
        "customerType": null,
        "customerValueBucket": "high",
    }));
}

#[test]
fn google_ads_event_options_schema_rejects_v2_customer_properties_with_a_missing_key() {
    assert_rejects_customer_properties(json!({ "status": "enabled", "customerType": "NEW" }));
}

#[test]
fn google_ads_event_options_schema_accepts_a_click_identity_with_a_null_id() {
    let mut options = with_id(Value::Null);
    options["identity"]["configuredPolicy"] = json!("click");
    options["identity"]["effectivePolicy"] = json!("click");
    options["identity"]["keySource"] = json!("click");
    assert!(parse_google_ads_event_options(&options).is_ok());
}

#[test]
fn google_ads_event_options_schema_accepts_an_id_of_exactly_64_characters() {
    assert!(parse_google_ads_event_options(&with_id(json!("x".repeat(64)))).is_ok());
}

#[test]
fn google_ads_event_options_schema_rejects_an_id_longer_than_64_characters() {
    assert!(parse_google_ads_event_options(&with_id(json!("x".repeat(65)))).is_err());
}

#[test]
fn google_ads_event_options_schema_rejects_an_empty_id() {
    assert!(parse_google_ads_event_options(&with_id(json!(""))).is_err());
}

#[test]
fn google_ads_event_options_schema_rejects_an_unknown_version() {
    let mut options = valid_options();
    options["version"] = json!(3);
    assert!(parse_google_ads_event_options(&options).is_err());
}

#[test]
fn google_ads_event_options_schema_rejects_a_policy_outside_click_id_and_event() {
    let mut options = valid_options();
    options["identity"]["configuredPolicy"] = json!("everything");
    assert!(parse_google_ads_event_options(&options).is_err());
    assert_eq!(GOOGLE_ADS_IDENTITY_POLICY_VALUES, ["click", "id", "event"]);
}

#[test]
fn google_ads_event_options_schema_accepts_an_event_policy_snapshot_that_keeps_no_key() {
    let mut options = with_id(Value::Null);
    options["identity"]["configuredPolicy"] = json!("event");
    options["identity"]["effectivePolicy"] = json!("event");
    options["identity"]["keySource"] = json!("occurrence");
    assert!(parse_google_ads_event_options(&options).is_ok());
}

#[test]
fn google_ads_event_options_schema_rejects_a_consent_status_other_than_granted_denied_or_null() {
    let mut options = valid_options();
    options["consent"]["adUserData"]["status"] = json!("unknown");
    assert!(parse_google_ads_event_options(&options).is_err());
}
