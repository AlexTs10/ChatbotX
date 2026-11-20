//! `facebook-custom-audience-handler.ts`: adds the contact to (or removes it
//! from) a Facebook custom audience. Fire-and-forget: a failure is logged as
//! a `facebook-ads` provider error and the flow goes on (success).

use chatbotx_business::ads::retarget::sync_contact_to_audience;
use chatbotx_business::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use chatbotx_jobs::JobError;

use super::StepCtx;
use crate::flow::StepStatus;

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    let ci = &run.contact_inbox;
    let messenger = (ci.channel == "messenger").then(|| (ci.source_id.clone(), ci.inbox_id));
    let audience = ctx.str("customAudienceId").unwrap_or_default();
    let operation = ctx.str("operation").unwrap_or("add");
    let outcome = sync_contact_to_audience(
        &run.deps.db,
        ctx.workspace_id(),
        ctx.contact_id(),
        messenger,
        audience,
        operation,
    )
    .await;
    if let Err(message) = outcome {
        tracing::error!(
            workspace_id = ctx.workspace_id(),
            custom_audience_id = audience,
            operation,
            err = %message,
            "Facebook custom audience step failed"
        );
        let mut input = ProviderErrorInput::new(
            "facebook-ads",
            &ctx.workspace_id().to_string(),
            Thrown::Error {
                message,
                stack: None,
                status: None,
            },
        );
        input.contact_id = Some(ctx.contact_id().to_string());
        input.source_id = Some(ci.source_id.clone());
        log_provider_error(run.deps.events.as_ref(), input).await;
    }
    Ok(StepStatus::Success)
}
