//! Port of `packages/flow-config/src/steps/evaluate-decision.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn evaluate_decision_field_mapping_schema = z::union(vec![z::object(vec![("customFieldId", z::string().regex(r#"^\d+$"#, "")),
("customFieldType", z::enum_(&["longText", "shortText"])),
("questionKey", z::string().trim().min(1).max(80)),
("value", z::literal(json!("choice")))]).strict(),
z::object(vec![("customFieldId", z::string().regex(r#"^\d+$"#, "")),
("customFieldType", z::literal(json!("number"))),
("questionKey", z::string().trim().min(1).max(80)),
("value", z::enum_(&["confidence", "probabilityTrue", "score", "selectedProbability"]))]).strict()]));

schema!(pub fn evaluate_decision_step_schema = z::object(vec![("contextCustomFieldIds", z::array(z::string().regex(r#"^\d+$"#, "")).min(1).max(8)),
("fieldMappings", z::array(evaluate_decision_field_mapping_schema()).max(60).super_refine(evaluate_decision_step_schema_field_mappings_super_refine)),
("id", z::string().regex(r#"^\d+$"#, "")),
("profileId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
skip_state_schema(),
error_state_schema()])),
("stepType", z::literal(json!("evaluateDecision")))]).strict().super_refine(evaluate_decision_step_schema_super_refine));

default_fn!(pub fn evaluate_decision_step_default_fn = json!({"fieldMappings": [], "contextCustomFieldIds": [], "id": create_id(), "profileId": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "skip"}, {"id": create_id(), "stateType": "error"}], "stepType": "evaluateDecision"}));

fn evaluate_decision_step_schema_field_mappings_super_refine(
    mappings: &Value,
    ctx: &mut RefineCtx,
) {
    report_duplicates(
        ctx,
        mappings.as_array().into_iter().flatten(),
        |m| key_of(&m["customFieldId"]),
        |i, _| {
            (
                path![i, "customFieldId"],
                "A custom field can only be mapped once in an Evaluate Decision step".to_owned(),
            )
        },
    );
}

fn evaluate_decision_step_schema_super_refine(step: &Value, ctx: &mut RefineCtx) {
    let mut context_ids = std::collections::HashSet::new();
    for (index, id) in arr_field(step, "contextCustomFieldIds").iter().enumerate() {
        let key = key_of(id).unwrap_or_default();
        if context_ids.contains(&key) {
            ctx.issue(
                path!["contextCustomFieldIds", index],
                "A context custom field can only be selected once",
            );
        }
        context_ids.insert(key);
    }
    for (index, mapping) in arr_field(step, "fieldMappings").iter().enumerate() {
        if context_ids.contains(&key_of(&mapping["customFieldId"]).unwrap_or_default()) {
            ctx.issue(
                path!["fieldMappings", index, "customFieldId"],
                "A custom field cannot be both Decision context and an output destination",
            );
        }
    }
}

/// `normalizeEvaluateDecisionMappingValue`: legacy `noul` / `probability`.
pub fn normalize_evaluate_decision_mapping_value(value: &Value) -> Value {
    match value.as_str() {
        Some("noul") => json!("probabilityTrue"),
        Some("probability") => json!("selectedProbability"),
        _ => value.clone(),
    }
}

/// `normalizeEvaluateDecisionStep`: a missing `contextCustomFieldIds` is `[]`.
pub fn normalize_evaluate_decision_step(raw: &Value) -> Value {
    let Value::Object(step) = raw else {
        return raw.clone();
    };
    let mut out = step.clone();
    let ids = step
        .get("contextCustomFieldIds")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    out.insert("contextCustomFieldIds".into(), ids);
    Value::Object(out)
}
