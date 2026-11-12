//! Port of `packages/flow-config/__tests__/condition-schema.test.ts`.

mod flow_config_support;
use flow_config_support::*;

// The condition node stores the precise custom-field type (`date` | `datetime`)
// so the runtime filter can pick wall-clock vs zone-aware comparison. Zod strips
// unknown keys, so a missing `customFieldType` in the schema silently drops it
// on save and every date field is mis-evaluated on the datetime path.

#[test]
fn conditionfilterconditionschema_preserves_customfieldtype_so_date_fields_keep_wall_clock_semantics()
 {
    let parsed = fc::condition_filter_condition_schema()
        .parse(&json!({
            "field": "customField",
            "operator": "eq",
            "customFieldId": "42",
            "valueType": "datetime",
            "customFieldType": "date",
            "value": "2026-07-20 09:30",
        }))
        .unwrap();

    assert_eq!(parsed["customFieldType"], "date");
    assert_eq!(parsed["valueType"], "datetime");
}

#[test]
fn conditionfilterconditionschema_stays_optional_for_non_custom_field_conditions() {
    let parsed = fc::condition_filter_condition_schema()
        .parse(&json!({ "field": "lastSeen", "operator": "gt", "value": "2026-07-20" }))
        .unwrap();

    assert!(parsed.get("customFieldType").is_none());
}

#[test]
fn conditionfilterconditionschema_preserves_topicid_so_coupon_topic_conditions_keep_their_selected_topic()
 {
    let parsed = fc::condition_filter_condition_schema()
        .parse(
            &json!({ "field": "couponTopic", "operator": "used", "topicId": "11619011544072192" }),
        )
        .unwrap();

    assert_eq!(parsed["topicId"], "11619011544072192");
}

#[test]
fn setcustomfieldstepschema_preserves_the_editor_timezone_so_worker_temporal_writes_keep_their_source_zone()
 {
    let parsed = fc::set_custom_field_step_schema()
        .parse(&json!({
            "id": "1",
            "stepType": "setCustomField",
            "inputFieldId": "cf-birthday",
            "operation": "O01",
            "value": "23/07/2026 09:30",
            "timezone": "Asia/Ho_Chi_Minh",
        }))
        .unwrap();

    assert_eq!(parsed["timezone"], "Asia/Ho_Chi_Minh");
}

fn pipeline_condition() -> Value {
    json!({ "field": "pipelineItem", "operator": "in", "value": ["open"] })
}

#[test]
fn pipelineitem_in_the_condition_step_is_rejected_on_a_condition() {
    let parsed = fc::condition_filter_condition_schema().safe_parse(&pipeline_condition());

    assert!(!parsed.success);
    assert_eq!(parsed.issues()[0].path_json(), json!(["field"]));
}

#[test]
fn pipelineitem_in_the_condition_step_fails_the_whole_step_so_save_and_publish_stop() {
    let parsed = fc::condition_step_schema().safe_parse(&json!({
        "id": "1",
        "stepType": "condition",
        "otherwiseId": "2",
        "cases": [{ "id": "3", "operator": "and", "conditions": [pipeline_condition()] }],
    }));

    assert!(!parsed.success);
}

#[test]
fn pipelineitem_in_the_condition_step_other_fields_are_unaffected() {
    assert!(
        fc::condition_step_schema()
            .safe_parse(&json!({
                "id": "1",
                "stepType": "condition",
                "otherwiseId": "2",
                "cases": [{
                    "id": "3",
                    "operator": "and",
                    "conditions": [{ "field": "email", "operator": "eq", "value": "a@b.c" }],
                }],
            }))
            .success
    );
}
