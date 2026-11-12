//! Port of `packages/flow-config/src/steps/send-collection.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const COLLECTION_MAX_PRODUCTS: usize = 10;

schema!(pub fn collection_types = z::enum_(&["automated", "manual"]));

schema!(pub fn send_collection_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendCollection")).describe("Step type discriminator: \"sendCollection\".")),
("dataType", z::literal(json!("products")).describe("What the collection lists; only \"products\" for now.")),
("collectionType", collection_types().describe("`automated`: the products matching `conditions` at send time. `manual`: the chosen `productIds`.")),
("conditions", z::object(vec![("name", z::string().trim().max(255).describe("Part of the product name; empty for any. Only searchable products match a name.")),
("categoryId", z::union(vec![z::string().regex(r#"^\d+$"#, ""),
z::literal(json!(""))]).nullable().optional().transform_opt(optional_id_transform).describe("Category id (from `productCategories.list`); null for all. Its sub-categories count too.")),
("subcategoryId", z::union(vec![z::string().regex(r#"^\d+$"#, ""),
z::literal(json!(""))]).nullable().optional().transform_opt(optional_id_transform).describe("Sub-category id; null for all. Narrows `categoryId` when both are set.")),
("vendorId", z::union(vec![z::string().regex(r#"^\d+$"#, ""),
z::literal(json!(""))]).nullable().optional().transform_opt(optional_id_transform).describe("Vendor id (from `productVendors.list`); null for all."))]).describe("With `automated`: the filters a product must match.")),
("productIds", z::array(z::string().regex(r#"^\d+$"#, "")).max(10).describe("With `manual`: product ids from `products.list`, in order (max 10)."))]).refine(send_collection_step_schema_refine, Some("Choose at least one product."), Some(&["productIds"])));

default_fn!(pub fn send_collection_step_default_fn = json!({"id": create_id(), "stepType": "sendCollection", "dataType": "products", "collectionType": "automated", "conditions": {"name": "", "categoryId": null, "subcategoryId": null, "vendorId": null}, "productIds": []}));

fn optional_id_transform(value: Option<Value>, _ctx: &mut RefineCtx) -> Option<Value> {
    // `(id) => id || null`
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(Value::String(s)),
        _ => Some(Value::Null),
    }
}

fn send_collection_step_schema_refine(step: &Value) -> bool {
    step["collectionType"] == "automated" || !arr_field(step, "productIds").is_empty()
}
