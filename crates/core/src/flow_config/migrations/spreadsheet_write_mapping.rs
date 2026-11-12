//! Port of `packages/flow-config/src/migrations/spreadsheet-write-mapping.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT mapNodeSteps:
// (details,transform)=>{if(!isRecord(details)){return details}const detailsRecord=details;const nextDetails={...detailsRecord};if("beforeStep"in detailsRecord){nextDetails.beforeStep=mapStepRecord(detailsRecord.beforeStep,transform)}if("steps"in detailsRecord){nextDetails.steps=mapStepArray(detailsRecord.steps,transform)}if("quickReplies"in detailsRecord){nextDetails.quickReplies=Array.isArray(detailsRecord.quickReplies)?detailsRecord.quickReplies.map(button=>mapButtonSteps(button,transform)):detailsRecord.quickReplies}return nextDetails}

// HAND-PORT toCustomFieldToken:
// field=>`{{raw:${field.name}}}`

// HAND-PORT upgradeSpreadsheetWriteStep:
// (step,lookup)=>{if(!isRecord(step)){return step}if(toSpreadsheetStepVersion(step.version)===spreadsheetStepVersions.enum.v2){return step}if(!Array.isArray(step.map)||step.map.length===0){return step}const convertedMap=step.map.map(item=>convertLegacyMapItem(item,lookup));if(convertedMap.some(item=>item===null)){return step}return{...step,version:spreadsheetStepVersions.enum.v2,map:convertedMap}}

// HAND-PORT tagSpreadsheetWriteStepVersion:
// step=>{if(!isRecord(step)||"version"in step){return step}return{...step,version:spreadsheetStepVersions.enum.v1}}

// HAND-PORT upgradeNodeSteps:
// (details,lookup)=>mapNodeSteps(details,step=>{const stepType=toStepType(step.stepType);const upgrader=stepType?stepUpgraders[stepType]:void 0;return upgrader?upgrader(step,lookup):step})

// HAND-PORT tagNodeSpreadsheetWriteStepVersions:
// details=>mapNodeSteps(details,step=>{const stepType=toStepType(step.stepType);return stepType&&spreadsheetWriteStepTypes.some(type=>type===stepType)?tagSpreadsheetWriteStepVersion(step):step})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn tag_node_spreadsheet_write_step_versions(details: &Value) -> Value {
    todo!("P3.4")
}

pub fn tag_spreadsheet_write_step_version(step: &Value) -> Value {
    todo!("P3.4")
}

pub fn to_custom_field_token(field: &Value) -> String {
    todo!("P3.4")
}

pub fn upgrade_node_steps(details: &Value, lookup: &dyn Fn(&str) -> Option<Value>) -> Value {
    todo!("P3.4")
}

pub fn upgrade_spreadsheet_write_step(
    step: &Value,
    lookup: &dyn Fn(&str) -> Option<Value>,
) -> Value {
    todo!("P3.4")
}
