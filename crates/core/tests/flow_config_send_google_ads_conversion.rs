//! Port of `packages/flow-config/__tests__/send-google-ads-conversion.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn configured() -> Value {
    spread(
        &fc::send_google_ads_conversion_default_fn(),
        json!({ "conversionActionId": "123456789", "dedupMode": "click" }),
    )
}

fn issues_of(input: &Value) -> Vec<Issue> {
    fc::send_google_ads_conversion_schema()
        .safe_parse(input)
        .issues()
        .to_vec()
}

fn ok_with(patch: Value) -> bool {
    fc::send_google_ads_conversion_schema()
        .safe_parse(&spread(&configured(), patch))
        .success
}

fn parse_with(patch: Value) -> Value {
    fc::send_google_ads_conversion_schema()
        .parse(&spread(&configured(), patch))
        .unwrap()
}

fn contains_issue(issues: &[Issue], path: Option<Value>, message: &str) -> bool {
    issues.iter().any(|issue| {
        issue.message == message && path.as_ref().is_none_or(|p| &issue.path_json() == p)
    })
}

#[test]
fn send_google_ads_conversion_flow_contract_default_fn_is_an_unconfigured_step_with_success_error_states()
 {
    let defaults = fc::send_google_ads_conversion_default_fn();

    assert_eq!(defaults["stepType"], "sendGoogleAdsConversion");
    assert_eq!(defaults["conversionActionId"], "");
    assert_eq!(defaults["dedupMode"], "id");
    assert!(defaults.get("dedupId").is_none());
    assert!(defaults.get("conversionTime").is_none());
    assert_eq!(defaults["states"].as_array().unwrap().len(), 2);
    assert!(
        !fc::send_google_ads_conversion_schema()
            .safe_parse(&defaults)
            .success
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_is_registered_as_a_step_type_and_an_action_step() {
    assert!(
        fc::step_types()
            .enum_values()
            .contains(&"sendGoogleAdsConversion".to_owned())
    );
    let schema = fc::send_google_ads_conversion_schema();
    assert!(fc::action_steps().iter().any(|s| s.same(&schema)));
}

#[test]
fn send_google_ads_conversion_flow_contract_accepts_a_numeric_conversion_action_id() {
    assert_match_object(
        &fc::send_google_ads_conversion_schema()
            .parse(&configured())
            .unwrap(),
        &json!({ "stepType": "sendGoogleAdsConversion", "conversionActionId": "123456789" }),
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_rejects_conversion_action_id_j() {
    for conversion_action_id in ["", "abc", "12a", "12 3", "-1"] {
        assert!(
            !ok_with(json!({ "conversionActionId": conversion_action_id })),
            "{conversion_action_id:?}"
        );
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_rejects_the_wrong_step_type() {
    assert!(!ok_with(json!({ "stepType": "sendText" })));
}

#[test]
fn send_google_ads_conversion_flow_contract_normalises_a_static_value_and_currency() {
    let parsed = parse_with(json!({ "value": "49.90", "currency": "usd" }));

    assert_eq!(parsed["value"], "49.90");
    assert_eq!(parsed["currency"], "USD");
}

#[test]
fn send_google_ads_conversion_flow_contract_passes_variable_templates_through_untouched() {
    let parsed = parse_with(json!({
        "value": "{{order_total}}",
        "currency": "{{currency_field}}",
        "dedupMode": "id",
        "dedupId": "{{order_id}}",
        "conversionTime": "{{closed_at}}",
    }));

    assert_eq!(parsed["value"], "{{order_total}}");
    assert_eq!(parsed["currency"], "{{currency_field}}");
    assert_eq!(parsed["dedupId"], "{{order_id}}");
    assert_eq!(parsed["conversionTime"], "{{closed_at}}");
}

#[test]
fn send_google_ads_conversion_flow_contract_rejects_a_malformed_static_value_or_currency() {
    assert!(!ok_with(json!({ "value": "abc", "currency": "USD" })));
    assert!(!ok_with(json!({ "value": "10", "currency": "dollars" })));
}

#[test]
fn send_google_ads_conversion_flow_contract_treats_blank_value_currency_dedup_id_and_conversion_time_as_unset()
 {
    let parsed =
        parse_with(json!({ "value": "  ", "currency": "", "dedupId": " ", "conversionTime": "" }));

    for key in ["value", "currency", "dedupId", "conversionTime"] {
        assert!(parsed.get(key).is_none(), "{key}");
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_requires_value_and_currency_together_or_neither() {
    assert!(!ok_with(json!({ "value": "5" })));
    assert!(!ok_with(json!({ "currency": "USD" })));
    assert!(ok_with(json!({ "value": "5", "currency": "USD" })));
    assert!(
        fc::send_google_ads_conversion_schema()
            .safe_parse(&configured())
            .success
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_has_no_orderid_field_any_more() {
    assert!(!fc::google_ads_conversion_fields_schema().has_field("orderId"));
}

#[test]
fn send_google_ads_conversion_flow_contract_dedupmode_is_required_no_silent_default_for_flow_spec_authors()
 {
    let without_mode = without(&configured(), &["dedupMode"]);

    assert!(
        issues_of(&without_mode)
            .iter()
            .any(|issue| issue.path.first() == Some(&PathSegment::from("dedupMode")))
    );
    assert!(!ok_with(json!({ "dedupMode": "unknown" })));
}

#[test]
fn send_google_ads_conversion_flow_contract_customer_matching_variables_j_are_valid_s() {
    let cases = [
        (
            json!({ "matchEmail": "{{email}}", "matchPhone": "{{phone}}" }),
            true,
        ),
        (json!({ "matchEmail": "{{custom_work_email}}" }), true),
        (json!({ "matchEmail": "", "matchPhone": "" }), true),
        (json!({}), true),
        (json!({ "matchEmail": "jane@example.com" }), false),
        (json!({ "matchPhone": "+84901234567" }), false),
        (json!({ "matchEmail": "{{email}} " }), true),
        (json!({ "matchEmail": "mail: {{email}}" }), false),
        (json!({ "matchEmail": "{{email}}{{phone}}" }), false),
        (json!({ "matchEmail": "{{}}" }), false),
        (
            json!({ "matchEmail": format!("{{{{{}}}}}", rep("a", 196)) }),
            true,
        ),
        (
            json!({ "matchEmail": format!("{{{{{}}}}}", rep("a", 197)) }),
            false,
        ),
    ];
    for (matching, valid) in cases {
        assert_eq!(ok_with(matching.clone()), valid, "{matching}");
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_a_literal_e_mail_is_refused_with_an_i18n_key_never_echoed()
 {
    let issues = issues_of(&spread(
        &configured(),
        json!({ "matchEmail": "jane@example.com" }),
    ));

    assert!(contains_issue(
        &issues,
        Some(json!(["matchEmail"])),
        "googleAds.conversionFields.validation.matchTemplateInvalid"
    ));
    assert!(
        !serde_json::to_string(&issues)
            .unwrap()
            .contains("jane@example.com")
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_customer_properties_j_are_valid_s() {
    let cases = [
        (
            json!({ "customerType": "NEW", "customerValueBucket": "HIGH" }),
            true,
        ),
        (
            json!({ "customerType": "returning", "customerValueBucket": " low " }),
            true,
        ),
        (
            json!({ "customerType": "{{contact.type}}", "customerValueBucket": "{{tier}}" }),
            true,
        ),
        (
            json!({ "customerType": "", "customerValueBucket": "" }),
            true,
        ),
        (json!({}), true),
        (json!({ "customerType": "VIP" }), false),
        (json!({ "customerType": "REENGAGED" }), false),
        (json!({ "customerValueBucket": "GOLD" }), false),
        // A template is checked only once it is resolved, at record time.
        (json!({ "customerType": "{{a}} NEW" }), true),
    ];
    for (properties, valid) in cases {
        assert_eq!(ok_with(properties.clone()), valid, "{properties}");
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_a_static_customer_property_is_normalised_to_upper_case()
{
    let parsed = parse_with(json!({ "customerType": " new ", "customerValueBucket": "medium" }));

    assert_eq!(parsed["customerType"], "NEW");
    assert_eq!(parsed["customerValueBucket"], "MEDIUM");
}

#[test]
fn send_google_ads_conversion_flow_contract_an_invalid_customer_property_is_refused_with_an_i18n_key()
 {
    assert!(contains_issue(
        &issues_of(&spread(&configured(), json!({ "customerType": "VIP" }))),
        Some(json!(["customerType"])),
        "googleAds.conversionFields.validation.customerTypeInvalid"
    ));
    assert!(contains_issue(
        &issues_of(&spread(
            &configured(),
            json!({ "customerValueBucket": "GOLD" })
        )),
        Some(json!(["customerValueBucket"])),
        "googleAds.conversionFields.validation.customerValueBucketInvalid"
    ));
}

#[test]
fn send_google_ads_conversion_flow_contract_a_new_step_sets_no_customer_properties() {
    let step = fc::send_google_ads_conversion_default_fn();

    assert!(step.get("customerType").is_none());
    assert!(step.get("customerValueBucket").is_none());
}

#[test]
fn send_google_ads_conversion_flow_contract_a_new_step_has_customer_matching_off() {
    let step = fc::send_google_ads_conversion_default_fn();

    assert!(step.get("matchEmail").is_none());
    assert!(step.get("matchPhone").is_none());
}

#[test]
fn send_google_ads_conversion_flow_contract_event_mode_needs_no_dedup_id_every_run_is_its_own_conversion()
 {
    let without_id = without(&configured(), &["dedupId"]);

    assert!(
        fc::send_google_ads_conversion_schema()
            .safe_parse(&spread(&without_id, json!({ "dedupMode": "event" })))
            .success
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_id_mode_requires_a_dedup_id_with_an_i18n_key_message() {
    let issues = issues_of(&spread(&configured(), json!({ "dedupMode": "id" })));

    assert!(contains_issue(
        &issues,
        Some(json!(["dedupId"])),
        "googleAds.conversionFields.validation.dedupIdRequired"
    ));
}

#[test]
fn send_google_ads_conversion_flow_contract_click_mode_does_not_require_a_dedup_id() {
    assert!(
        fc::send_google_ads_conversion_schema()
            .safe_parse(&configured())
            .success
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_id_mode_accepts_a_static_id_or_a_template() {
    for dedup_id in ["A-1042", "{{order_number}}"] {
        assert!(
            ok_with(json!({ "dedupMode": "id", "dedupId": dedup_id })),
            "{dedup_id}"
        );
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_bounds_the_dedup_id_at_64_characters() {
    let with_id = |dedup_id: String| {
        spread(
            &configured(),
            json!({ "dedupMode": "id", "dedupId": dedup_id }),
        )
    };

    assert!(
        fc::send_google_ads_conversion_schema()
            .safe_parse(&with_id(rep("x", 64)))
            .success
    );
    assert!(contains_issue(
        &issues_of(&with_id(rep("x", 65))),
        Some(json!(["dedupId"])),
        "googleAds.conversionFields.validation.dedupIdTooLong"
    ));
}

#[test]
fn send_google_ads_conversion_flow_contract_the_length_check_applies_to_id_mode_only() {
    let too_long = rep("x", 65);

    assert!(ok_with(
        json!({ "dedupMode": "click", "dedupId": too_long })
    ));
    assert!(ok_with(json!({ "dedupMode": "click", "dedupId": " " })));
    assert!(ok_with(
        json!({ "dedupMode": "id", "dedupId": format!("{{{{order_id}}}}-{too_long}") })
    ));
}

#[test]
fn send_google_ads_conversion_flow_contract_validation_messages_are_i18n_keys() {
    let messages = |patch: Value| -> Vec<String> {
        issues_of(&spread(&configured(), patch))
            .into_iter()
            .map(|issue| issue.message)
            .collect()
    };
    assert!(
        messages(json!({ "value": "5" }))
            .contains(&"googleAds.conversionFields.validation.currencyRequired".to_owned())
    );
    assert!(
        messages(json!({ "currency": "USD" }))
            .contains(&"googleAds.conversionFields.validation.valueRequired".to_owned())
    );
}

#[test]
fn send_google_ads_conversion_flow_contract_accepts_conversion_time_j() {
    for conversion_time in [
        "2026-10-08T14:30:00Z",
        "2026-10-08T14:30:00+07:00",
        "{{crm_closed_at}}",
    ] {
        assert!(
            ok_with(json!({ "conversionTime": conversion_time })),
            "{conversion_time}"
        );
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_rejects_static_conversion_time_j_with_a_key() {
    let cases = [
        (
            "2026-10-08T14:30:00",
            "googleAds.conversionFields.validation.timeFormat",
        ),
        (
            "2026-10-08",
            "googleAds.conversionFields.validation.timeFormat",
        ),
        (
            "2026-02-30T10:00:00Z",
            "googleAds.conversionFields.validation.timeInvalidDate",
        ),
    ];
    for (conversion_time, message) in cases {
        assert!(
            contains_issue(
                &issues_of(&spread(
                    &configured(),
                    json!({ "conversionTime": conversion_time })
                )),
                None,
                message
            ),
            "{conversion_time}"
        );
    }
}

#[test]
fn send_google_ads_conversion_flow_contract_a_future_static_time_is_a_runtime_refusal_not_a_schema_error()
 {
    assert!(ok_with(json!({ "conversionTime": "2999-01-01T00:00:00Z" })));
}

#[test]
fn send_google_ads_conversion_flow_contract_the_shared_field_schema_parses_a_stored_trigger_action_object()
 {
    let parsed = fc::google_ads_conversion_fields_schema().safe_parse(&json!({
        "type": "sendGoogleAdsConversion",
        "conversionActionId": "42",
        "dedupMode": "click",
    }));

    assert!(parsed.success);
}
