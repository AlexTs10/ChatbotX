//! Port of `packages/flow-config/src/steps/whatsapp-call-button.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WHATSAPP_CALL_BUTTON_BODY_MAX: usize = 1024;

pub const WHATSAPP_CALL_BUTTON_LABEL_MAX: usize = 20;

schema!(pub fn whatsapp_call_button_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("whatsappCallButton")).describe("Step type discriminator: \"whatsappCallButton\".")),
("text", z::string().trim().min(1).max(1024)),
("buttonLabel", z::string().trim().min(1).max(20))]));

schema!(pub fn whatsapp_call_button_label_form_schema = z::object(vec![("buttonLabel", z::string().trim().min(1).max(20))]));

// HAND-PORT whatsappCallButtonStepDefaultFn:
// (props={})=>({text:"",buttonLabel:"Call Now",...props,id:createId(),stepType:stepTypes.enum.whatsappCallButton})
