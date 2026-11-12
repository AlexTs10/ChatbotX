//! Port of `packages/flow-config/src/steps/wa-template-send-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `validateWaTemplateSendParams(params, ctx, basePath)`.
pub fn validate_wa_template_send_params(
    params: &Value,
    ctx: &mut RefineCtx,
    base_path: &[PathSegment],
) {
    validate_mpm_params(params, ctx, base_path);
    validate_limited_time_offer_params(params, ctx, base_path);
    validate_payment_request_params(params, ctx, base_path);
}

/// `refineWaTemplateSendStep(step, ctx)`.
pub fn refine_wa_template_send_step(step: &Value, ctx: &mut RefineCtx) {
    validate_wa_template_send_params(
        &step["template"]["params"],
        ctx,
        &[PathSegment::from("template"), PathSegment::from("params")],
    );
}
