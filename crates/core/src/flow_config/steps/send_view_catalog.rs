//! Port of `packages/flow-config/src/steps/send-view-catalog.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_view_catalog_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendViewCatalog")).describe("Step type discriminator: \"sendViewCatalog\" (shares the product catalog with a button to open it).")),
("text", z::string().trim().min(1).refine(send_view_catalog_step_schema_text_refine, Some("sendViewCatalogBodyTooLong"), None).describe("Message text shown above the catalog button (max 1024 characters, counted in UTF-16 units so an emoji may count as 2). May include `{{variable}}` placeholders.").meta(json!({"maxLength":1024}))),
("footer", z::string().trim().refine(send_view_catalog_step_schema_footer_refine, Some("sendViewCatalogFooterTooLong"), None).default(json!("")).describe("Optional small text under the message (max 60 characters, UTF-16 units; use an empty string for none). May include `{{variable}}` placeholders.").meta(json!({"maxLength":60}))),
("thumbnailProductId", z::string().trim().default(json!("")).describe("Optional catalog product whose image is shown on the message: its Meta catalog `retailerId` (the Content ID in Commerce Manager), as returned by `products.searchMetaCatalogItems`. Use an empty string to let the channel pick one."))]));

schema!(pub fn send_view_catalog_details_form_schema = z::object(vec![("footer", z::string().trim().refine(send_view_catalog_step_schema_footer_refine, Some("sendViewCatalogFooterTooLong"), None).default(json!("")).describe("Optional small text under the message (max 60 characters, UTF-16 units; use an empty string for none). May include `{{variable}}` placeholders.").meta(json!({"maxLength":60}))),
("thumbnailProductId", z::string().trim().default(json!("")).describe("Optional catalog product whose image is shown on the message: its Meta catalog `retailerId` (the Content ID in Commerce Manager), as returned by `products.searchMetaCatalogItems`. Use an empty string to let the channel pick one."))]));

// HAND-PORT sendViewCatalogStepDefaultFn:
// (props={})=>({text:"",footer:"",thumbnailProductId:"",...props,id:createId(),stepType:stepTypes.enum.sendViewCatalog})

// value=>value.length<=SEND_VIEW_CATALOG_TEXT_MAX
fn send_view_catalog_step_schema_text_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>value.length<=SEND_VIEW_CATALOG_FOOTER_MAX
fn send_view_catalog_step_schema_footer_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_view_catalog_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
