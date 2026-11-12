//! Port of `packages/flow-config/src/steps/whatsapp-flow.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WHATSAPP_FLOW_BODY_MAX: usize = 1024;

pub const WHATSAPP_FLOW_BUTTON_MAX: usize = 20;

pub const WHATSAPP_FLOW_PARAM_KEY_MAX: usize = 64;

schema!(pub fn whatsapp_flow_file_formats = z::enum_(&["allUrls", "firstUrl"]));

pub const DEFAULT_WHATSAPP_FLOW_FILE_FORMAT: &str = "allUrls";

pub const WHATSAPP_FLOW_FILE_ANSWER_FIELD_TYPES: &[&str] = &["shortText", "longText"];

schema!(pub fn whatsapp_flow_field_mapping_schema = z::object(vec![("paramKey", z::string().trim().min(1).max(64)),
("paramLabel", z::string().optional()),
("customFieldId", z::string().regex(r#"^\d+$"#, "").nullable()),
("fileFormat", whatsapp_flow_file_formats().optional().describe("Only for file answers: \"allUrls\" stores every file URL, comma-separated (default); \"firstUrl\" stores the first one."))]));

schema!(pub fn whatsapp_flow_data_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").nullable()),
("sourceId", z::string()),
("startScreenId", z::string().nullable()),
("fieldMappings", z::array(whatsapp_flow_field_mapping_schema()))]));

schema!(pub fn whatsapp_flow_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("whatsappFlow")).describe("Step type discriminator: \"whatsappFlow\".")),
("text", z::string().trim().min(1).max(1024)),
("buttons", z::array(button_step_schema()).min(1).max(1).super_refine(refine_reply_only_buttons)),
("inboxId", z::string().regex(r#"^\d+$"#, "").nullable()),
("flow", z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").nullable()),
("sourceId", z::string()),
("startScreenId", z::string().nullable()),
("fieldMappings", z::array(whatsapp_flow_field_mapping_schema()))]).super_refine(whatsapp_flow_step_schema_flow_super_refine))]));

schema!(pub fn whatsapp_flow_dialog_form_schema = z::object(vec![("buttonLabel", z::string().trim().min(1).max(20)),
("flow", z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("sourceId", z::string().trim().min(1)),
("startScreenId", z::string().trim().min(1)),
("fieldMappings", z::array(whatsapp_flow_field_mapping_schema()))]))]));

/// `whatsappFlowStepDefaultFn(props)`.
pub fn whatsapp_flow_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({
            "text": "",
            "buttons": [whatsapp_flow_default_button(None)],
            "inboxId": null,
            "flow": { "id": null, "sourceId": "", "startScreenId": null, "fieldMappings": [] },
        }),
        props,
        json!({ "id": create_id(), "stepType": "whatsappFlow" }),
    )
}

fn whatsapp_flow_step_schema_flow_super_refine(flow: &Value, ctx: &mut RefineCtx) {
    if flow["id"].is_null() || trimmed(&flow["startScreenId"]).unwrap_or("").is_empty() {
        ctx.message("whatsappFlowIncomplete");
    }
}

/// `whatsappFlowDefaultButton(label = "button #1")`.
pub fn whatsapp_flow_default_button(label: Option<&str>) -> Value {
    json!({ "id": create_id(), "label": label.unwrap_or("button #1"), "buttonType": null, "beforeStep": null, "steps": [] })
}
