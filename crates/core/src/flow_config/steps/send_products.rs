//! Port of `packages/flow-config/src/steps/send-products.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_products_displays = z::enum_(&["products", "carousel"]));

pub const SEND_PRODUCTS_TEXT_MAX: usize = 4096;

schema!(pub fn send_products_section_schema = z::object(vec![("title", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Section title. Required by some channels once there are 2+ sections.")),
("productIds", z::array(z::string().regex(r#"^\d+$"#, "")).describe("Product ids in this section. Get them from `products.listSendable`."))]));

schema!(pub fn send_products_content_schema = z::object(vec![("display", send_products_displays().describe("\"products\": one product, or a list of products grouped in sections. \"carousel\": 2+ products as scrollable cards.")),
("header", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Header text. Only for the \"products\" display with 2+ products.")),
("body", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Message text. Required for most displays, see the channel capabilities.")),
("footer", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Optional footer text.")),
("sections", z::array(send_products_section_schema()).describe("Product sections. \"products\": 1+ sections; \"carousel\": exactly one untitled section."))]));

schema!(pub fn send_products_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendProducts")).describe("Step type discriminator: \"sendProducts\".")),
("display", send_products_displays().describe("\"products\": one product, or a list of products grouped in sections. \"carousel\": 2+ products as scrollable cards.")),
("header", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Header text. Only for the \"products\" display with 2+ products.")),
("body", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Message text. Required for most displays, see the channel capabilities.")),
("footer", z::string().trim().refine(send_products_section_schema_title_refine, Some("sendProductsFieldTooLong"), None).optional().describe("Optional footer text.")),
("sections", z::array(send_products_section_schema()).describe("Product sections. \"products\": 1+ sections; \"carousel\": exactly one untitled section."))]));

// HAND-PORT refineSendProductsStructure:
// (content,ctx)=>{const seen=new Set;content.sections.forEach((section,sectionIndex)=>{if(section.productIds.length===0){ctx.addIssue({code:"custom",path:["sections",sectionIndex,"productIds"],message:flowValidationCodes.sendProductsProductCountOutOfRange})}for(const productId of section.productIds){if(seen.has(productId)){ctx.addIssue({code:"custom",path:["sections",sectionIndex,"productIds"],message:flowValidationCodes.sendProductsDuplicateProduct})}seen.add(productId)}})}

// HAND-PORT sendProductsStepDefaultFn:
// (props={})=>({display:"products",sections:[{productIds:[]}],...props,id:createId(),stepType:stepTypes.enum.sendProducts})

// value=>countMessageCharacters(value)<=SEND_PRODUCTS_TEXT_MAX
fn send_products_section_schema_title_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_products_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
