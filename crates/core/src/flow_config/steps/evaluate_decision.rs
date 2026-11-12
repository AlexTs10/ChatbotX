//! Port of `packages/flow-config/src/steps/evaluate-decision.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT normalizeEvaluateDecisionMappingValue:
// value=>typeof value==="string"&&value in legacyMappingValueByValue?legacyMappingValueByValue[value]:value

schema!(pub fn evaluate_decision_field_mapping_schema = z::union(vec![z::object(vec![("customFieldId", z::string().regex(r#"^\d+$"#, "")),
("customFieldType", z::enum_(&["longText", "shortText"])),
("questionKey", z::string().trim().min(1).max(80)),
("value", z::literal(json!("choice")))]).strict(),
z::object(vec![("customFieldId", z::string().regex(r#"^\d+$"#, "")),
("customFieldType", z::literal(json!("number"))),
("questionKey", z::string().trim().min(1).max(80)),
("value", z::enum_(&["confidence", "probabilityTrue", "score", "selectedProbability"]))]).strict()]));

// HAND-PORT normalizeEvaluateDecisionStep:
// raw=>{if(!raw||typeof raw!=="object"||Array.isArray(raw)){return raw}const step=raw;return{...step,contextCustomFieldIds:step.contextCustomFieldIds??[]}}

schema!(pub fn evaluate_decision_step_schema = z::object(vec![("contextCustomFieldIds", z::array(z::string().regex(r#"^\d+$"#, "")).min(1).max(8)),
("fieldMappings", z::array(evaluate_decision_field_mapping_schema()).max(60).super_refine(evaluate_decision_step_schema_field_mappings_super_refine)),
("id", z::string().regex(r#"^\d+$"#, "")),
("profileId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
skip_state_schema(),
error_state_schema()])),
("stepType", z::literal(json!("evaluateDecision")))]).strict().super_refine(evaluate_decision_step_schema_super_refine));

default_fn!(pub fn evaluate_decision_step_default_fn = json!({"fieldMappings": [], "contextCustomFieldIds": [], "id": create_id(), "profileId": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "skip"}, {"id": create_id(), "stateType": "error"}], "stepType": "evaluateDecision"}));

// (mappings,ctx)=>{const seen=new Set;for(const[index,mapping]of mappings.entries()){if(seen.has(mapping.customFieldId)){ctx.addIssue({code:"custom",message:"A custom field can only be mapped once in an Evaluate Decision step",path:[index,"customFieldId"]})}seen.add(mapping.customFieldId)}}
fn evaluate_decision_step_schema_field_mappings_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// (step,ctx)=>{const contextIds=new Set;for(const[index,id]of step.contextCustomFieldIds.entries()){if(contextIds.has(id)){ctx.addIssue({code:"custom",message:"A context custom field can only be selected once",path:["contextCustomFieldIds",index]})}contextIds.add(id)}for(const[index,mapping]of step.fieldMappings.entries()){if(contextIds.has(mapping.customFieldId)){ctx.addIssue({code:"custom",message:"A custom field cannot be both Decision context and an output destination",path:["fieldMappings",index,"customFieldId"]})}}}
fn evaluate_decision_step_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
