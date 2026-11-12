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
("nodes", z::array(flow_version_schema()).super_refine(|nodes, ctx| refine_steps_by_channel(nodes.as_array().map_or(&[][..], Vec::as_slice), ctx))),
("edges", z::array(edge_schema()))]));

schema!(pub fn flow_export_schema = z::object(vec![("formatVersion", z::literal(json!(2))),
("exportedAt", z::string()),
("source", z::object(vec![("workspaceId", z::string()),
("flowId", z::string()),
("appVersion", z::string().optional())])),
("flows", z::array(flow_exported_flow_schema()).length(1)),
("customFields", z::record(z::string(), flow_export_custom_field_schema()).default(json!({}))),
("botFields", z::record(z::string(), flow_export_bot_field_schema()).default(json!({})))]));

fn flow_export_bot_field_schema_name_refine(name: &Value) -> bool {
    !is_reserved_field_name(name.as_str().unwrap_or(""))
}

/// The failure of `parseFlowExport`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowExportParseError {
    pub reason: String,
    pub errors: Vec<FlowAuthoringError>,
}

/// `parseFlowExport(raw)`: never throws.
pub fn parse_flow_export(raw: &Value) -> Result<Value, FlowExportParseError> {
    if let Value::Object(map) = raw {
        let version = map.get("formatVersion");
        if version.and_then(Value::as_f64) != Some(FLOW_EXPORT_FORMAT_VERSION as f64) {
            let shown = match version {
                None => "undefined".to_owned(),
                Some(v) => crate::js::string_of(v),
            };
            let reason = format!("Unsupported export format version: {shown}");
            return Err(FlowExportParseError {
                errors: vec![FlowAuthoringError {
                    code: "invalidGraph".into(),
                    message: reason.clone(),
                    path: "formatVersion".into(),
                    ..FlowAuthoringError::default()
                }],
                reason,
            });
        }
    }
    match flow_export_schema().parse(raw) {
        Ok(data) => Ok(data),
        Err(error) => {
            let errors = zod_error_to_flow_authoring_errors(&error, "invalidGraph", None);
            let reason = errors.first().map_or_else(
                || "The export file is invalid.".to_owned(),
                |e| e.message.clone(),
            );
            Err(FlowExportParseError { reason, errors })
        }
    }
}
