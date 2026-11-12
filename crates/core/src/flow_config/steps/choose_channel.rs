//! Port of `packages/flow-config/src/steps/choose-channel.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn choose_channel_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("chooseChannel")).describe("Step type discriminator: \"chooseChannel\".")),
("channel", z::string().trim().min(1).describe("Channel this message node targets, e.g. `omnichannel` (any channel) or a specific channel type such as `messenger`, `whatsapp`, `instagram`, `telegram`. Channel-specific steps are validated against it on publish."))]));

// HAND-PORT chooseChannelStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.chooseChannel,channel:"omnichannel",...props})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn choose_channel_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
