//! Port of `packages/business/__tests__/google-ads-consent.test.ts`
//! (`toConsentInput`; `consentTemplatesOf` is inline in the worker producer,
//! `consentForTransport` is covered by the delivery tests).

use chatbotx_business::google_ads_conversion::to_consent_input;
use serde_json::{Map, Value, json};

fn variable(template: &str) -> Value {
    json!({ "type": "variable", "template": template })
}

fn settings(ad_user_data: Value, ad_personalization: Value) -> Value {
    json!({ "adUserData": ad_user_data, "adPersonalization": ad_personalization })
}

fn not_provided() -> Value {
    json!({ "type": "notProvided" })
}

fn resolved(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

/// Port of: "toConsentInput > variable %j resolves to %s"
#[test]
fn a_variable_resolves_to_its_status() {
    for (raw, status) in [
        ("Granted", "granted"),
        (" DENIED ", "denied"),
        ("granted", "granted"),
    ] {
        let result = to_consent_input(
            &settings(variable("{{a}}"), not_provided()),
            &resolved(&[("adUserData", raw)]),
        );
        assert_eq!(
            result,
            Ok(json!({
                "adUserData": { "status": status, "source": "variable" },
                "adPersonalization": { "status": null, "source": "notProvided" },
            }))
        );
    }
}

/// Port of: "toConsentInput > variable %s is omitted"
#[test]
fn an_empty_or_unresolved_variable_is_omitted() {
    for raw in [
        Some(""),
        Some("   "),
        Some("{{x}}"),
        Some("granted {{x}}"),
        None,
    ] {
        let values = raw.map_or_else(Map::new, |raw| resolved(&[("adUserData", raw)]));
        let result =
            to_consent_input(&settings(variable("{{a}}"), not_provided()), &values).expect("ok");
        assert_eq!(
            result["adUserData"],
            json!({ "status": null, "source": "variable" }),
            "{raw:?}"
        );
    }
}

/// Port of: "a value that is not granted or denied fails naming the setting" (+ "the failure never carries the raw value")
#[test]
fn another_value_fails_naming_the_setting() {
    let s = settings(variable("{{a}}"), not_provided());
    assert_eq!(
        to_consent_input(&s, &resolved(&[("adUserData", "yes")])),
        Err("adUserData")
    );
    let failure = to_consent_input(&s, &resolved(&[("adUserData", "SECRET-raw-value")]));
    assert!(!format!("{failure:?}").contains("SECRET-raw-value"));
}

/// Port of: "fixed and not-provided sources ignore resolved values"
#[test]
fn fixed_and_not_provided_sources_ignore_resolved_values() {
    let result = to_consent_input(
        &settings(json!({ "type": "granted" }), json!({ "type": "denied" })),
        &resolved(&[("adUserData", "denied")]),
    );
    assert_eq!(
        result,
        Ok(json!({
            "adUserData": { "status": "granted", "source": "fixed" },
            "adPersonalization": { "status": "denied", "source": "fixed" },
        }))
    );
    let result =
        to_consent_input(&settings(not_provided(), not_provided()), &Map::new()).expect("ok");
    assert_eq!(
        result["adUserData"],
        json!({ "status": null, "source": "notProvided" })
    );
}

/// Port of: "names adPersonalization when only the second setting is bad"
#[test]
fn names_ad_personalization_when_only_it_is_bad() {
    let result = to_consent_input(
        &settings(variable("{{a}}"), variable("{{b}}")),
        &resolved(&[("adUserData", "granted"), ("adPersonalization", "maybe")]),
    );
    assert_eq!(result, Err("adPersonalization"));
}
