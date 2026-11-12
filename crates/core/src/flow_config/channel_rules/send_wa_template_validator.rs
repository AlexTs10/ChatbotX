//! Port of `packages/flow-config/src/channel-rules/send-wa-template-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `sendWaTemplateMessageValidator`.
pub fn send_wa_template_message_validator() -> StepValidator {
    static V: LazyLock<StepValidator> = LazyLock::new(|| {
        StepValidator::per_channel(
            send_wa_template_message_step_schema().super_refine(refine_wa_template_send_step),
            vec![],
        )
    });
    V.clone()
}
