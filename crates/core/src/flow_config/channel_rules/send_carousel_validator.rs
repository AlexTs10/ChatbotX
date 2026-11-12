//! Port of `packages/flow-config/src/channel-rules/send-carousel-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `sendCarouselValidator`.
pub fn send_carousel_validator() -> StepValidator {
    static V: LazyLock<StepValidator> = LazyLock::new(|| {
        StepValidator::per_channel(
            send_carousel_step_schema(),
            vec![(
                "whatsapp",
                send_carousel_step_schema().super_refine(refine_whatsapp_carousel_step),
            )],
        )
    });
    V.clone()
}
