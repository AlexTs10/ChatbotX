//! Port of `packages/flow-config/src/states/index.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn state_types() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"success":"success","error":"error","skip":"skip"}"#)
            .expect("valid JSON")
    });
    &V
}

schema!(pub fn success_state_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("stateType", z::literal(json!("success")))]));

default_fn!(pub fn success_state_default_fn = json!({"id": create_id(), "stateType": "success"}));

schema!(pub fn error_state_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("stateType", z::literal(json!("error")))]));

default_fn!(pub fn error_state_default_fn = json!({"id": create_id(), "stateType": "error"}));

schema!(pub fn skip_state_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("stateType", z::literal(json!("skip")))]));

default_fn!(pub fn skip_state_default_fn = json!({"id": create_id(), "stateType": "skip"}));
