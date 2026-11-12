//! Port of `packages/flow-config/src/steps/facebook-custom-audience.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn facebook_custom_audience_operations = z::enum_(&["add", "remove"]));

schema!(pub fn facebook_custom_audience_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("facebookCustomAudience")).describe("Step type discriminator: \"facebookCustomAudience\".")),
("operation", facebook_custom_audience_operations().default(json!("add"))),
("adAccountId", z::string().trim().min(1)),
("customAudienceId", z::string().trim().min(1))]));

default_fn!(pub fn facebook_custom_audience_default_fn = json!({"id": create_id(), "stepType": "facebookCustomAudience", "operation": "add", "adAccountId": "", "customAudienceId": ""}));
