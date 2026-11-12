//! Port of `packages/flow-config/src/steps/coupon.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn set_up_coupon_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("setUpCoupon")).describe("Step type discriminator: \"setUpCoupon\".")),
("topicId", z::string().regex(r#"^\d+$"#, "")),
("states", z::preprocess(set_up_coupon_step_schema_states_preprocess, z::undefined()).optional())]));

schema!(pub fn mark_coupon_used_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("markCouponUsed")).describe("Step type discriminator: \"markCouponUsed\".")),
("topicId", z::string().regex(r#"^\d+$"#, "")),
("states", z::preprocess(mark_coupon_used_step_schema_states_preprocess, z::undefined()).optional())]));

default_fn!(pub fn set_up_coupon_step_default_fn = json!({"id": create_id(), "stepType": "setUpCoupon", "topicId": ""}));

default_fn!(pub fn mark_coupon_used_step_default_fn = json!({"id": create_id(), "stepType": "markCouponUsed", "topicId": ""}));

fn set_up_coupon_step_schema_states_preprocess(_value: Option<Value>) -> Option<Value> {
    None
}

fn mark_coupon_used_step_schema_states_preprocess(_value: Option<Value>) -> Option<Value> {
    None
}
