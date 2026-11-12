//! Port of `packages/flow-config/src/steps/whatsapp-option-list.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WHATSAPP_OPTION_LIST_MIN_OPTIONS: usize = 2;

pub const WHATSAPP_OPTION_LIST_MAX_OPTIONS: usize = 10;

pub const WHATSAPP_OPTION_LIST_BUTTON_MAX: usize = 20;

pub const WHATSAPP_OPTION_LIST_TITLE_MAX: usize = 24;

pub const WHATSAPP_OPTION_LIST_DESCRIPTION_MAX: usize = 72;

pub const WHATSAPP_OPTION_LIST_BODY_MAX: usize = 1024;

schema!(pub fn whatsapp_option_list_item_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("title", z::string().trim().min(1).max(24)),
("description", z::string().trim().max(72).optional())]));

schema!(pub fn whatsapp_option_list_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("whatsappOptionList")).describe("Step type discriminator: \"whatsappOptionList\".")),
("text", z::string().trim().min(1).max(1024)),
("buttonId", z::string().regex(r#"^\d+$"#, "")),
("buttonLabel", z::string().trim().min(1).max(20)),
("options", z::array(whatsapp_option_list_item_schema()).max(10))]));

schema!(pub fn whatsapp_option_list_button_label_form_schema = z::object(vec![("buttonLabel", z::string().trim().min(1).max(20))]));

schema!(pub fn whatsapp_option_list_option_form_schema = z::object(vec![("title", z::string().trim().min(1).max(24)),
("description", z::union(vec![z::string().trim().max(72).optional(),
z::literal(json!(""))]))]));

// HAND-PORT whatsappOptionListItemDefaultFn:
// (props={})=>({title:"",...props,id:createId()})

// HAND-PORT whatsappOptionListStepDefaultFn:
// (props={})=>({text:"",buttonLabel:"button #1",options:[whatsappOptionListItemDefaultFn({title:"Title #1"}),whatsappOptionListItemDefaultFn({title:"Title #2"})],...props,id:createId(),buttonId:createId(),stepType:stepTypes.enum.whatsappOptionList})

// ---- stage 1 API (hand-ported in stage 2) ----

/// `whatsappOptionListItemDefaultFn(props)`.
pub fn whatsapp_option_list_item_default_fn(props: &Value) -> Value {
    spread3(json!({"title": ""}), props, json!({"id": create_id()}))
}

/// `whatsappOptionListStepDefaultFn(props)`.
pub fn whatsapp_option_list_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({"text": "", "buttonLabel": "button #1", "options": [{"title": "Title #1", "id": create_id()}, {"title": "Title #2", "id": create_id()}]}),
        props,
        json!({"id": create_id(), "buttonId": create_id(), "stepType": "whatsappOptionList"}),
    )
}
