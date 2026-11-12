//! Port of `packages/flow-config/src/steps/mailchimp-add-member.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn mailchimp_merge_field_mapping_schema = z::object(vec![("tag", z::string().min(1)),
("name", z::string().optional()),
("type", z::string().optional()),
("customFieldId", z::string())]));

schema!(pub fn mailchimp_add_member_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("mailchimpAddMember")).describe("Step type discriminator: \"mailchimpAddMember\".")),
("listId", z::string().min(1)),
("email", z::string().min(1)),
("doubleOptIn", z::boolean()),
("tags", z::array(z::string())),
("mergeFields", z::array(mailchimp_merge_field_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

default_fn!(pub fn mailchimp_add_member_default_fn = json!({"id": create_id(), "stepType": "mailchimpAddMember", "listId": "", "email": "email", "doubleOptIn": false, "tags": [], "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));
