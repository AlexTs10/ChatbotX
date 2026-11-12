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

/// `sendProductsStepDefaultFn(props)`.
pub fn send_products_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({"display": "products", "sections": [{"productIds": []}]}),
        props,
        json!({"id": create_id(), "stepType": "sendProducts"}),
    )
}

fn send_products_section_schema_title_refine(value: &Value) -> bool {
    count_message_characters(value.as_str()) <= SEND_PRODUCTS_TEXT_MAX
}

/// `refineSendProductsStructure(content, ctx)`: non-empty sections, no
/// duplicate products.
pub fn refine_send_products_structure(content: &Value, ctx: &mut RefineCtx) {
    let mut seen = std::collections::HashSet::new();
    for (section_index, section) in arr_field(content, "sections").iter().enumerate() {
        let ids = arr_field(section, "productIds");
        if ids.is_empty() {
            ctx.issue(
                path!["sections", section_index, "productIds"],
                "sendProductsProductCountOutOfRange",
            );
        }
        for id in ids {
            let key = key_of(id).unwrap_or_default();
            if seen.contains(&key) {
                ctx.issue(
                    path!["sections", section_index, "productIds"],
                    "sendProductsDuplicateProduct",
                );
            }
            seen.insert(key);
        }
    }
}
