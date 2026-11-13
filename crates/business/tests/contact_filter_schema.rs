//! Port of `apps/builder/__tests__/contact-filter-schema.test.ts` (P3.6-D2
//! slice contact filter schema). Ported: the zod `safeParse(..).success`
//! checks become `is_ok()` on the Rust parsers. The `listContactsRequest`
//! cases check the `contactFilter` key (`parse_lenient_param`); the
//! pagination / keyword keys of that request are parsed by the API handler.

use chatbotx_business::contact_filter::{
    Surface, parse_bot_field_condition, parse_condition, parse_coupon_topic_condition,
    parse_criteria, parse_custom_field_condition, parse_leaderboard_score_condition,
    parse_lenient_param, parse_static_field_condition,
};
use serde_json::{Value, json};

fn static_ok(field: &str, condition: Value) -> bool {
    parse_static_field_condition(field, &condition).is_ok()
}

fn single_ok(condition: Value) -> bool {
    parse_condition(&condition, Surface::Contacts).is_ok()
}

// ---- staticFieldFilter ------------------------------------------------------

#[test]
fn static_field_filter_accepts_enabled_operators_for_text_dropdown_boolean_and_date_fields() {
    assert!(static_ok(
        "fullName",
        json!({"field": "fullName", "operator": "startsWith", "value": "Al"})
    ));
    assert!(static_ok(
        "tags",
        json!({"field": "tags", "operator": "eq", "value": ["tag-1"]})
    ));
    assert!(static_ok(
        "locale",
        json!({"field": "locale", "operator": "eq", "value": ["vi_VN"]})
    ));
    assert!(static_ok(
        "timezone",
        json!({"field": "timezone", "operator": "eq", "value": ["Asia/Ho_Chi_Minh"]})
    ));
    assert!(static_ok(
        "blocked",
        json!({"field": "blocked", "operator": "eq", "value": "true"})
    ));
    assert!(static_ok(
        "lastSeen",
        json!({"field": "lastSeen", "operator": "isBetween", "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"]})
    ));
}

#[test]
fn static_field_filter_accepts_has_contact_info_presence_operators_and_rejects_the_rest() {
    let f = "hasContactInfo";
    assert!(static_ok(
        f,
        json!({"field": f, "operator": "in", "value": ["phone", "email"]})
    ));
    assert!(static_ok(
        f,
        json!({"field": f, "operator": "notIn", "value": ["email"]})
    ));
    assert!(static_ok(
        f,
        json!({"field": f, "operator": "in", "value": ["phoneAndEmail"]})
    ));
    assert!(static_ok(f, json!({"field": f, "operator": "isEmpty"})));
    assert!(!static_ok(
        f,
        json!({"field": f, "operator": "eq", "value": ["phone"]})
    ));
    assert!(!static_ok(f, json!({"field": f, "operator": "in"})));
}

#[test]
fn static_field_filter_rejects_disabled_operators_and_missing_interval_values() {
    assert!(!static_ok(
        "tags",
        json!({"field": "tags", "operator": "contains", "value": "vip"})
    ));
    assert!(!static_ok(
        "timezone",
        json!({"field": "timezone", "operator": "contains", "value": "Asia"})
    ));
    assert!(!static_ok(
        "lastSeen",
        json!({"field": "lastSeen", "operator": "isBetween", "value": ["2026-05-01T00:00:00Z"]})
    ));
}

#[test]
fn static_field_filter_accepts_valueless_operators_without_value_and_applies_synthetic_fallback_only_at_factory_level()
 {
    assert!(static_ok(
        "fullName",
        json!({"field": "fullName", "operator": "isNotEmpty"})
    ));
    assert!(static_ok(
        "syntheticField",
        json!({"field": "syntheticField", "operator": "contains", "value": "vip"})
    ));
}

#[test]
fn static_field_filter_rejects_empty_operators_for_non_nullable_boolean_fields() {
    for field in ["emailWasVerified", "optedInForEmail"] {
        assert!(
            !static_ok(field, json!({"field": field, "operator": "isEmpty"})),
            "{field}"
        );
        assert!(
            static_ok(
                field,
                json!({"field": field, "operator": "eq", "value": "true"})
            ),
            "{field}"
        );
    }
    assert!(static_ok(
        "subscribedToBroadcast",
        json!({"field": "subscribedToBroadcast", "operator": "isEmpty"})
    ));
}

#[test]
fn static_field_filter_accepts_relation_set_operators_for_activated_field_s() {
    for field in [
        "broadcastSent",
        "broadcastDelivered",
        "broadcastSeen",
        "broadcastClicked",
        "broadcastFailed",
        "subscribedToDripCampaign",
        "entryPointsLinks",
        "conversationAssigned",
    ] {
        assert!(
            static_ok(
                field,
                json!({"field": field, "operator": "in", "value": ["option-1"]})
            ),
            "{field}"
        );
        assert!(
            !static_ok(
                field,
                json!({"field": field, "operator": "contains", "value": "option-1"})
            ),
            "{field}"
        );
    }
}

#[test]
fn static_field_filter_accepts_w1_2_activated_field_operators() {
    assert!(static_ok(
        "continent",
        json!({"field": "continent", "operator": "eq", "value": ["AS", "unknown"]})
    ));
    for field in ["existingContact", "unreplied", "unread"] {
        assert!(
            static_ok(
                field,
                json!({"field": field, "operator": "eq", "value": "true"})
            ),
            "{field}"
        );
        assert!(
            !static_ok(
                field,
                json!({"field": field, "operator": "contains", "value": "true"})
            ),
            "{field}"
        );
    }
}

#[test]
fn static_field_filter_accepts_w1_3_activated_date_and_number_field_operators() {
    assert!(static_ok(
        "lastSent",
        json!({"field": "lastSent", "operator": "isBetween", "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"]})
    ));
    for field in [
        "contactCreatedDateMinutesAgo",
        "lastSeenMinutesAgo",
        "lastInteractionMinutesAgo",
        "consecutiveAiFailures",
    ] {
        assert!(
            static_ok(
                field,
                json!({"field": field, "operator": "gte", "value": "30"})
            ),
            "{field}"
        );
        assert!(
            static_ok(
                field,
                json!({"field": field, "operator": "isBetween", "value": ["10", "30"]})
            ),
            "{field}"
        );
    }
}

// ---- couponTopicConditionSchema ---------------------------------------------

fn coupon_ok(condition: Value) -> bool {
    parse_coupon_topic_condition(&condition).is_ok()
}

#[test]
fn coupon_topic_condition_schema_accepts_is_not_empty_and_used_without_a_value() {
    assert!(coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "isNotEmpty"})
    ));
    assert!(coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "used"})
    ));
}

#[test]
fn coupon_topic_condition_schema_accepts_eq_with_a_coupon_code_value() {
    assert!(coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "eq", "value": "SAVE10"})
    ));
}

#[test]
fn coupon_topic_condition_schema_rejects_a_missing_topic_id() {
    assert!(!coupon_ok(
        json!({"field": "couponTopic", "topicId": "", "operator": "isNotEmpty"})
    ));
}

#[test]
fn coupon_topic_condition_schema_rejects_eq_without_a_value_and_unsupported_operators() {
    assert!(!coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "eq"})
    ));
    assert!(!coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "contains", "value": "SAVE10"})
    ));
    assert!(!coupon_ok(
        json!({"field": "couponTopic", "topicId": "topic-1", "operator": "ne", "value": "topic-1"})
    ));
}

// ---- customFieldConditionSchema / botFieldConditionSchema --------------------

fn typed_field_cases(type_key: &str) -> Vec<(&'static str, Value, bool)> {
    vec![
        (
            "short text search",
            json!({type_key: "shortText", "valueType": "text", "operator": "contains", "value": "vip"}),
            true,
        ),
        (
            "number range",
            json!({type_key: "number", "valueType": "number", "operator": "isBetween", "value": ["1", "10"]}),
            true,
        ),
        (
            "datetime range",
            json!({type_key: "datetime", "valueType": "datetime", "operator": "isBetween", "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"]}),
            true,
        ),
        (
            "boolean valueless",
            json!({type_key: "boolean", "valueType": "boolean", "operator": "isNotEmpty"}),
            true,
        ),
        (
            "long text search disabled",
            json!({type_key: "longText", "valueType": "text", "operator": "contains", "value": "vip"}),
            false,
        ),
    ]
}

fn with(base: Value, extra: &Value) -> Value {
    let mut out = base;
    if let (Some(out), Some(extra)) = (out.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

#[test]
fn custom_field_condition_schema_validates_s() {
    for (name, values, expected) in typed_field_cases("customFieldType") {
        let condition = with(
            json!({"field": "customField", "customFieldId": "cf-1"}),
            &values,
        );
        assert_eq!(
            parse_custom_field_condition(&condition).is_ok(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn custom_field_condition_schema_requires_custom_field_id_and_two_values_for_interval_operators() {
    assert!(
        parse_custom_field_condition(
            &json!({"field": "customField", "valueType": "text", "operator": "eq", "value": "vip"})
        )
        .is_err()
    );
    assert!(
        parse_custom_field_condition(&json!({"field": "customField", "customFieldId": "cf-1", "valueType": "number", "operator": "isBetween", "value": ["1"]}))
            .is_err()
    );
}

#[test]
fn bot_field_condition_schema_validates_s() {
    for (name, values, expected) in typed_field_cases("botFieldType") {
        let condition = with(json!({"field": "botField", "botFieldId": "bf-1"}), &values);
        assert_eq!(
            parse_bot_field_condition(&condition).is_ok(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn bot_field_condition_schema_requires_bot_field_id_and_two_values_for_interval_operators() {
    assert!(
        parse_bot_field_condition(
            &json!({"field": "botField", "valueType": "text", "operator": "eq", "value": "vip"})
        )
        .is_err()
    );
    assert!(
        parse_bot_field_condition(&json!({"field": "botField", "botFieldId": "bf-1", "valueType": "number", "operator": "isBetween", "value": ["1"]}))
            .is_err()
    );
}

// ---- contact filter union schemas -------------------------------------------

#[test]
fn contact_filter_union_schemas_accepts_static_and_custom_field_conditions() {
    assert!(single_ok(
        json!({"field": "email", "operator": "eq", "value": "a@example.com"})
    ));
    assert!(single_ok(
        json!({"field": "customField", "customFieldId": "cf-1", "valueType": "text", "operator": "eq", "value": "vip"})
    ));
}

#[test]
fn contact_filter_union_schemas_accepts_bot_field_conditions() {
    assert!(single_ok(
        json!({"field": "botField", "botFieldId": "bf-1", "valueType": "text", "operator": "eq", "value": "vip"})
    ));
}

#[test]
fn contact_filter_union_schemas_rejects_bogus_public_field_names() {
    let criteria = json!({
        "operator": "and",
        "conditions": [{"field": "syntheticField", "operator": "contains", "value": "vip"}],
    });
    assert!(parse_criteria(&criteria, Surface::Contacts).is_err());
}

// ---- CTWA fields ------------------------------------------------------------

#[test]
fn ctwa_fields_accepts_from_ctwa_ad_boolean_conditions() {
    assert!(single_ok(
        json!({"field": "fromCtwaAd", "operator": "eq", "value": "true"})
    ));
    assert!(single_ok(
        json!({"field": "fromCtwaAd", "operator": "isEmpty"})
    ));
    assert!(!single_ok(
        json!({"field": "fromCtwaAd", "operator": "contains", "value": "true"})
    ));
}

#[test]
fn ctwa_fields_accepts_from_google_ad_boolean_conditions_only() {
    assert!(single_ok(
        json!({"field": "fromGoogleAd", "operator": "eq", "value": "true"})
    ));
    assert!(single_ok(
        json!({"field": "fromGoogleAd", "operator": "isEmpty"})
    ));
    assert!(!single_ok(
        json!({"field": "fromGoogleAd", "operator": "contains", "value": "true"})
    ));
}

#[test]
fn ctwa_fields_accepts_ctwa_conversion_in_not_in_is_empty_conditions() {
    assert!(single_ok(
        json!({"field": "ctwaConversion", "operator": "in", "value": ["lead"]})
    ));
    assert!(single_ok(
        json!({"field": "ctwaConversion", "operator": "notIn", "value": ["purchase"]})
    ));
    assert!(single_ok(
        json!({"field": "ctwaConversion", "operator": "isEmpty"})
    ));
}

#[test]
fn ctwa_fields_rejects_disallowed_operators_for_ctwa_conversion() {
    assert!(!single_ok(
        json!({"field": "ctwaConversion", "operator": "contains", "value": "lead"})
    ));
    assert!(!single_ok(
        json!({"field": "ctwaConversion", "operator": "eq", "value": ["lead"]})
    ));
}

// ---- listContactsRequest contactFilter preprocess ---------------------------

fn criteria() -> Value {
    json!({"operator": "and", "conditions": [{"field": "inbox", "operator": "eq", "value": ["123"]}]})
}

#[test]
fn list_contacts_request_contact_filter_preprocess_parses_json_string_contact_filter_values() {
    let raw = Value::String(criteria().to_string());
    let parsed = parse_lenient_param(Some(&raw)).expect("parse");
    assert_eq!(parsed, Some(criteria()));
}

#[test]
fn list_contacts_request_contact_filter_preprocess_passes_through_already_object_contact_filter_values()
 {
    let parsed = parse_lenient_param(Some(&criteria())).expect("parse");
    assert_eq!(parsed, Some(criteria()));
}

#[test]
fn list_contacts_request_contact_filter_preprocess_drops_invalid_json_contact_filter_values_without_breaking_pagination_parsing()
 {
    let parsed = parse_lenient_param(Some(&json!("{invalid"))).expect("parse");
    assert_eq!(parsed, None);
}

// ---- leaderboardScoreConditionSchema ----------------------------------------

#[test]
fn leaderboard_score_condition_schema_accepts_a_numeric_comparison_and_coerces_numbers_to_strings()
{
    let parsed = parse_leaderboard_score_condition(
        &json!({"field": "leaderboardScore", "leaderboardId": "77", "operator": "gte", "value": 100}),
    )
    .expect("parse");
    assert_eq!(parsed["value"], json!("100"));
}

#[test]
fn leaderboard_score_condition_schema_rejects_other_operators_and_non_numeric_values() {
    assert!(
        parse_leaderboard_score_condition(&json!({"field": "leaderboardScore", "leaderboardId": "77", "operator": "contains", "value": "1"}))
            .is_err()
    );
    assert!(
        parse_leaderboard_score_condition(&json!({"field": "leaderboardScore", "leaderboardId": "77", "operator": "gt", "value": "abc"}))
            .is_err()
    );
}

#[test]
fn leaderboard_score_condition_schema_is_part_of_the_single_condition_union_flow_condition_step() {
    assert!(single_ok(
        json!({"field": "leaderboardScore", "leaderboardId": "77", "operator": "lt", "value": "5"})
    ));
}

// ---- flow-only contact filter fields ----------------------------------------

fn working_hours_criteria() -> Value {
    json!({
        "operator": "and",
        "conditions": [{"field": "isWithinWorkingHours", "operator": "in", "value": ["assignedAgent"]}],
    })
}

#[test]
fn flow_only_contact_filter_fields_are_rejected_by_the_list_audience_criteria_schema() {
    assert!(parse_criteria(&working_hours_criteria(), Surface::Contacts).is_err());
}

#[test]
fn flow_only_contact_filter_fields_are_accepted_by_the_flow_condition_criteria_schema() {
    assert!(parse_criteria(&working_hours_criteria(), Surface::FlowCondition).is_ok());
}

#[test]
fn flow_only_contact_filter_fields_only_allow_in_not_in_with_a_value() {
    let with_condition = |condition: Value| {
        let condition = with(json!({"field": "isWithinWorkingHours"}), &condition);
        parse_criteria(
            &json!({"operator": "and", "conditions": [condition]}),
            Surface::FlowCondition,
        )
        .is_ok()
    };
    assert!(with_condition(json!({"operator": "notIn", "value": ["1"]})));
    assert!(!with_condition(json!({"operator": "eq", "value": "1"})));
    assert!(!with_condition(json!({"operator": "in"})));
}

// ---- lastInteraction filter -------------------------------------------------

fn last_interaction_ok(operator: &str, value: Option<&str>) -> bool {
    let mut condition = json!({"field": "lastInteraction", "operator": operator});
    if let Some(value) = value {
        condition["value"] = json!(value);
    }
    parse_static_field_condition("lastInteraction", &condition).is_ok()
}

#[test]
fn last_interaction_filter_accepts_is_empty_never_messaged_and_is_not_empty_has_messaged_without_a_value()
 {
    assert!(last_interaction_ok("isEmpty", None));
    assert!(last_interaction_ok("isNotEmpty", None));
}

#[test]
fn last_interaction_filter_still_accepts_a_date_comparison() {
    assert!(last_interaction_ok("gt", Some("2026-05-19T10:00:00Z")));
}
