//! Port of `packages/flow-config/src/steps/external-request.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn external_request_methods = z::enum_(&["GET", "POST", "PUT", "PATCH", "DELETE"]));

pub const METHODS_WITH_BODY: &[&str] = &["POST", "PUT", "PATCH"];

schema!(pub fn external_request_body_types = z::enum_(&["allContactData", "json", "formEncoded"]));

schema!(pub fn external_request_header_schema = z::object(vec![("key", z::string().trim().min(1)),
("value", z::string())]));

schema!(pub fn external_request_fields_schema = z::object(vec![("method", external_request_methods().default(json!("GET"))),
("url", z::string().trim().min(1)),
("headers", z::array(external_request_header_schema())),
("body", z::discriminated_union("bodyType", vec![z::object(vec![("bodyType", z::literal(json!("allContactData")))]),
z::object(vec![("bodyType", z::literal(json!("json"))),
("jsonBody", z::string().trim().min(1))]),
z::object(vec![("bodyType", z::literal(json!("formEncoded"))),
("formFields", z::array(external_request_header_schema()))])]).optional())]).super_refine(validate_request_fields));

schema!(pub fn external_request_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("callApi")).describe("Step type discriminator: \"callApi\".")),
("method", external_request_methods().default(json!("GET"))),
("url", z::string().trim().min(1)),
("headers", z::array(external_request_header_schema())),
("body", z::discriminated_union("bodyType", vec![z::object(vec![("bodyType", z::literal(json!("allContactData")))]),
z::object(vec![("bodyType", z::literal(json!("json"))),
("jsonBody", z::string().trim().min(1))]),
z::object(vec![("bodyType", z::literal(json!("formEncoded"))),
("formFields", z::array(external_request_header_schema()))])]).optional()),
("mapping", z::array(z::object(vec![("jsonPath", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1))]))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(validate_request_fields));

default_fn!(pub fn external_request_step_default_fn = json!({"id": create_id(), "stepType": "callApi", "method": "GET", "url": "", "headers": [], "mapping": [{"jsonPath": "", "outputFieldId": ""}], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

fn has_duplicate_keys(items: &[Value]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let normalized = crate::js::to_lower(crate::js::trim(str_field(item, "key")));
        if normalized.is_empty() {
            continue;
        }
        if !seen.insert(normalized) {
            return true;
        }
    }
    false
}

fn validate_request_fields(data: &Value, ctx: &mut RefineCtx) {
    let method = str_field(data, "method");
    let method_allows_body = METHODS_WITH_BODY.contains(&method);
    let has_body = truthy(data.get("body"));
    if method_allows_body && !has_body {
        ctx.issue(path!["body"], "Body is required for this HTTP method");
    }
    if !method_allows_body && has_body {
        ctx.issue(path!["body"], "Body is not supported for this HTTP method");
    }
    if has_duplicate_keys(arr_field(data, "headers")) {
        ctx.issue(path!["headers"], "Header keys must be unique");
    }
    if data["body"]["bodyType"] == "formEncoded"
        && has_duplicate_keys(arr_field(&data["body"], "formFields"))
    {
        ctx.issue(path!["body", "formFields"], "Field keys must be unique");
    }
}
