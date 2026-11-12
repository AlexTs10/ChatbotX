//! Port of `packages/flow-config/src/steps/open-mini-app.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn open_mini_app_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("openMiniApp")).describe("Step type discriminator: \"openMiniApp\".")),
("miniAppId", z::string().regex(r#"^\d+$"#, "").describe("Id of the Mini App the button opens. Get Mini App ids from `miniApps.list`."))]));

/// `openMiniAppStepDefaultFn(props)`.
pub fn open_mini_app_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({ "id": create_id(), "stepType": "openMiniApp", "miniAppId": "" }),
        props,
    )
}
