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

// (data,ctx)=>{const methodAllowsBody=methodsWithBody.includes(data.method);if(methodAllowsBody&&!data.body){ctx.addIssue({code:"custom",path:["body"],message:"Body is required for this HTTP method"})}if(!methodAllowsBody&&data.body){ctx.addIssue({code:"custom",path:["body"],message:"Body is not supported for this HTTP method"})}if(hasDuplicateKeys(data.headers)){ctx.addIssue({code:"custom",path:["headers"],message:"Header keys must be unique"})}if(data.body?.bodyType===externalRequestBodyTypes.enum.formEncoded&&hasDuplicateKeys(data.body.formFields)){ctx.addIssue({code:"custom",path:["body","formFields"],message:"Field keys must be unique"})}}
fn validate_request_fields(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
