//! Port of `packages/flow-config/src/authoring/errors.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn flow_capability_schema = z::discriminated_union("code", vec![z::object(vec![("block", z::string()),
("channel", z::string()),
("code", z::literal(json!("unsupportedBlock"))),
("policyVersion", z::number())]),
z::object(vec![("actual", z::number().optional()),
("allowed", z::number().optional()),
("block", z::string()),
("channel", z::string()),
("code", z::literal(json!("constraintExceeded"))),
("constraintId", z::string().optional()),
("policyVersion", z::number()),
("unit", z::string().optional())])]));

// HAND-PORT FlowAuthoringException:
// class FlowAuthoringException extends Error{static{__name(this,"FlowAuthoringException")}errors;constructor(errors){super(errors.length>0?errors.map(error=>`${error.path}: ${formatFlowAuthoringErrorMessage(error)}`).join("; "):"Flow authoring failed");this.name="FlowAuthoringException";this.errors=errors}}

// HAND-PORT formatZodPathSegment:
// (acc,segment)=>{if(typeof segment==="number"){return`${acc}[${segment}]`}const key=String(segment);return acc.length===0?key:`${acc}.${key}`}

// HAND-PORT zodErrorToFlowAuthoringErrors:
// function zodErrorToFlowAuthoringErrors(error,code,mapPath){return error.issues.map(issue=>{const issueParams="params"in issue&&typeof issue.params==="object"?issue.params:void 0;const capability=issueParams&&"capability"in issueParams&&isFlowCapability(issueParams.capability)?issueParams.capability:void 0;const capabilityCode=capability?.code;return{capability,path:mapPath?.(issue.path)??issue.path.reduce(formatZodPathSegment,""),code:capabilityCode??code,message:issue.message}})}

// HAND-PORT closestNames:
// function closestNames(target,available){const targetLower=target.toLowerCase();return[...available].map(name=>({name,score:nameSimilarity(targetLower,name.toLowerCase())})).filter(({score})=>score>0).sort((a,b)=>b.score-a.score).slice(0,MAX_CANDIDATES).map(({name})=>name)}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `FlowAuthoringError`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowAuthoringError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Value>,
    pub path: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<String>>,
}

/// `FlowAuthoringException`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowAuthoringException {
    pub errors: Vec<FlowAuthoringError>,
}

impl FlowAuthoringException {
    pub fn new(errors: Vec<FlowAuthoringError>) -> FlowAuthoringException {
        FlowAuthoringException { errors }
    }

    /// `error.message`.
    pub fn message(&self) -> String {
        todo!("P3.4")
    }
}

pub fn closest_names(target: &str, available: &[&str]) -> Vec<String> {
    todo!("P3.4")
}

pub fn format_zod_path_segment(acc: &str, segment: &PathSegment) -> String {
    todo!("P3.4")
}

pub fn zod_error_to_flow_authoring_errors(
    error: &z::ZodError,
    code: &str,
    map_path: Option<&dyn Fn(&[PathSegment]) -> Option<String>>,
) -> Vec<FlowAuthoringError> {
    todo!("P3.4")
}
