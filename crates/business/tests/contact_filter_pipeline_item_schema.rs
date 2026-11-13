//! Port of the `pipelineItem condition schema` part of
//! `apps/builder/__tests__/contact-filter-pipeline-item.test.ts` (P3.6-D2
//! slice contact filter schema). Ported: `safeParse(..).success` becomes
//! `is_ok()`. The UI config, form draft and `collectFilterValueIds` parts
//! are builder UI code (not-applicable).

use chatbotx_business::contact_filter::{Surface, parse_condition, parse_criteria};
use serde_json::{Value, json};

const SALES: &str = "11";

fn parse_ok(condition: Value) -> bool {
    let mut full = json!({"field": "pipelineItem"});
    if let (Some(out), Some(extra)) = (full.as_object_mut(), condition.as_object()) {
        for (k, v) in extra {
            out.insert(k.clone(), v.clone());
        }
    }
    parse_condition(&full, Surface::Contacts).is_ok()
}

#[test]
fn pipeline_item_condition_schema_accepts_s() {
    let cases = [
        (
            "exists in any pipeline",
            json!({"attribute": "exists", "pipelineId": null, "operator": "isNotEmpty", "value": "any"}),
        ),
        (
            "has no open item",
            json!({"attribute": "exists", "pipelineId": SALES, "operator": "isEmpty", "value": "open"}),
        ),
        (
            "status in a list",
            json!({"attribute": "status", "pipelineId": null, "operator": "in", "value": ["open", "lost"]}),
        ),
        (
            "stage in a pipeline",
            json!({"attribute": "stage", "pipelineId": SALES, "operator": "notIn", "value": ["21"]}),
        ),
        (
            "priority in a pipeline",
            json!({"attribute": "priority", "pipelineId": SALES, "operator": "in", "value": ["high"]}),
        ),
    ];
    for (label, condition) in cases {
        assert!(parse_ok(condition), "{label}");
    }
}

#[test]
fn pipeline_item_condition_schema_rejects_s() {
    let cases = [
        (
            "stage without a pipeline",
            json!({"attribute": "stage", "pipelineId": null, "operator": "in", "value": ["21"]}),
        ),
        (
            "stage with an empty list",
            json!({"attribute": "stage", "pipelineId": SALES, "operator": "in", "value": []}),
        ),
        (
            "stage id that is not numeric",
            json!({"attribute": "stage", "pipelineId": SALES, "operator": "in", "value": ["lead"]}),
        ),
        (
            "pipeline id that is not numeric",
            json!({"attribute": "status", "pipelineId": "abc", "operator": "in", "value": ["open"]}),
        ),
        (
            "status that does not exist",
            json!({"attribute": "status", "pipelineId": null, "operator": "in", "value": ["archived"]}),
        ),
        (
            "status with a single string",
            json!({"attribute": "status", "pipelineId": null, "operator": "in", "value": "open"}),
        ),
        (
            "status with the exists operators",
            json!({"attribute": "status", "pipelineId": null, "operator": "isEmpty", "value": ["open"]}),
        ),
        (
            "exists with the membership operators",
            json!({"attribute": "exists", "pipelineId": null, "operator": "in", "value": "any"}),
        ),
        (
            "exists with a list",
            json!({"attribute": "exists", "pipelineId": null, "operator": "isNotEmpty", "value": ["any"]}),
        ),
        (
            "exists with an unknown status",
            json!({"attribute": "exists", "pipelineId": null, "operator": "isNotEmpty", "value": "closed"}),
        ),
        (
            "unknown attribute",
            json!({"attribute": "title", "pipelineId": null, "operator": "in", "value": ["a"]}),
        ),
        (
            "missing pipelineId",
            json!({"attribute": "priority", "operator": "in", "value": ["low"]}),
        ),
    ];
    for (label, condition) in cases {
        assert!(!parse_ok(condition), "{label}");
    }
}

#[test]
fn pipeline_item_condition_schema_is_accepted_inside_a_criteria_object_with_other_conditions() {
    let criteria = json!({
        "operator": "and",
        "conditions": [
            {"field": "email", "operator": "contains", "value": "@x.io"},
            {"field": "pipelineItem", "attribute": "priority", "pipelineId": null, "operator": "in", "value": ["low"]},
        ],
    });
    assert!(parse_criteria(&criteria, Surface::Contacts).is_ok());
}
