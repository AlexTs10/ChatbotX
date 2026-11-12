//! Port of `packages/flow-config/src/steps/open-website.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn open_website_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("openWebsite")).describe("Step type discriminator: \"openWebsite\".")),
("url", z::string().refine(open_website_step_schema_url_refine, Some("Invalid URL"), None)),
("browserSize", z::literals(vec![json!(40), json!(70), json!(100)]))]));

default_fn!(pub fn open_website_step_default_fn = json!({"id": create_id(), "stepType": "openWebsite", "url": "", "browserSize": 100}));

fn open_website_step_schema_url_refine(value: &Value) -> bool {
    deps::url_with_variables_ok(value)
}
