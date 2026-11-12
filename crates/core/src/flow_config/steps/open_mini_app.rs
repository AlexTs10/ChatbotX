//! Port of `packages/flow-config/src/steps/open-mini-app.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn open_mini_app_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("openMiniApp")).describe("Step type discriminator: \"openMiniApp\".")),
("miniAppId", z::string().regex(r#"^\d+$"#, "").describe("Id of the Mini App the button opens. Get Mini App ids from `miniApps.list`."))]));

// HAND-PORT openMiniAppStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.openMiniApp,miniAppId:"",...props})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn open_mini_app_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
