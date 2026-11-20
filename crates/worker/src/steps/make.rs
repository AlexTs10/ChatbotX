//! `make-handler.ts` / `trigger-n8n-handler.ts`: POST the contact to every
//! external webhook registered for the step's events (any provider).
//! ponytail: the transport is `deps.webhook_http` (SSRF guard at every
//! redirect hop) instead of `externalRequestService.execute`; same checks.

use chatbotx_business::external_webhook;
use chatbotx_jobs::JobError;
use serde_json::Value;

use super::contact::biz_fail;
use super::{StepCtx, step_error};
use crate::flow::StepStatus;

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let db = &ctx.run.deps.db;
    let events: Vec<String> = ctx
        .step
        .get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let registrations = external_webhook::list_by_events(db, ctx.workspace_id(), events)
        .await
        .map_err(biz_fail)?;
    if registrations.is_empty() {
        return Ok(StepStatus::Success);
    }
    let body = external_webhook::delivery_body(db, ctx.workspace_id(), ctx.contact_id())
        .await
        .map_err(biz_fail)?;
    for registration in registrations {
        let mut payload = serde_json::Map::new();
        payload.insert("event".into(), Value::from(registration.event.clone()));
        payload.extend(body.clone());
        let text = Value::Object(payload).to_string();
        match ctx
            .run
            .deps
            .webhook_http
            .post(&registration.url, &text)
            .await
        {
            Ok(status) if status >= 400 => {
                let message = format!(
                    "Webhook for event \"{}\" failed with status {status}",
                    registration.event
                );
                return Ok(step_error(ctx, &message));
            }
            Ok(_) => {}
            Err(error) => return Ok(step_error(ctx, &format!("{error:?}"))),
        }
    }
    Ok(StepStatus::Success)
}
