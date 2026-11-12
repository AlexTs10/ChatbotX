//! Port of `packages/flow-config/src/import-export/schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const FLOW_EXPORT_FORMAT_VERSION: usize = 2;

schema!(pub fn flow_export_custom_field_schema = z::object(vec![("name", z::string().trim().min(1)),
("type", deps::custom_field_types())]));

schema!(pub fn flow_export_bot_field_schema = z::object(vec![("name", z::string().trim().min(1).max(255).refine(flow_export_bot_field_schema_name_refine, Some("Name must not start with the reserved \"bot_field:\" prefix."), None)),
("type", deps::custom_field_types())]));

schema!(pub fn flow_exported_flow_schema = z::object(vec![("name", z::string().trim().min(1).max(255)),
("active", z::boolean()),
("enableInInbox", z::boolean()),
("startNodeId", z::string()),
("nodes", z::array(flow_version_schema()).super_refine(refine_steps_by_channel)),
("edges", z::array(edge_schema()))]));

schema!(pub fn flow_export_schema = z::object(vec![("formatVersion", z::literal(json!(2))),
("exportedAt", z::string()),
("source", z::object(vec![("workspaceId", z::string()),
("flowId", z::string()),
("appVersion", z::string().optional())])),
("flows", z::array(flow_exported_flow_schema()).length(1)),
("customFields", z::record(z::string(), flow_export_custom_field_schema()).default(json!({}))),
("botFields", z::record(z::string(), flow_export_bot_field_schema()).default(json!({})))]));

// HAND-PORT parseFlowExport:
// raw=>{const preParsed=z.object({formatVersion:z.unknown()}).safeParse(raw);if(preParsed.success&&preParsed.data.formatVersion!==FLOW_EXPORT_FORMAT_VERSION){const reason=`Unsupported export format version: ${String(preParsed.data.formatVersion)}`;return{ok:false,reason,errors:[{code:"invalidGraph",message:reason,path:"formatVersion"}]}}const result=flowExportSchema.safeParse(raw);if(!result.success){const errors=zodErrorToFlowAuthoringErrors(result.error,"invalidGraph");return{ok:false,reason:errors[0]?.message??"The export file is invalid.",errors}}return{ok:true,data:result.data}}

// name=>!isReservedFieldName(name)
fn flow_export_bot_field_schema_name_refine(value: &Value) -> bool {
    todo!("P3.4")
}
