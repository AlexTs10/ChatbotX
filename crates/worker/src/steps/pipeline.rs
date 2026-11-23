//! `pipeline.ts` (`handlePipelineStep`): the business rules live in
//! `runPipelineFlowAction`; this handler gives it the contact's variable
//! resolver, publishes the realtime events and flow-trigger events, runs the
//! `assignContact` effects and maps the outcome.

use std::sync::Arc;

use chatbotx_business::pipeline::flow_actions::{
    Interpolate, InterpolateFuture, PipelineFlowActionContext, PipelineFlowActionStatus,
    run_pipeline_flow_action,
};
use chatbotx_jobs::JobError;
use serde_json::{Map, Value};

use super::StepCtx;
use super::contact::biz_fail;
use crate::flow::StepStatus;

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    let (workspace_id, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    let variables = run.deps.variables.clone();
    let (conversation, contact_inbox) = (run.conversation.clone(), run.contact_inbox.clone());
    // `contactVariableService.replaceAll` (no spintax). ponytail: a resolver
    // failure keeps the text as written (the original rethrows and retries).
    let interpolate: Interpolate = Arc::new(move |text: String| {
        let (variables, conversation, contact_inbox) = (
            variables.clone(),
            conversation.clone(),
            contact_inbox.clone(),
        );
        Box::pin(async move {
            let Some(variables) = variables.filter(|_| text.contains("{{")) else {
                return text;
            };
            match variables
                .resolve(
                    &conversation,
                    &contact_inbox,
                    &Value::String(text.clone()),
                    None,
                    None,
                    false,
                )
                .await
            {
                Ok(Value::String(out)) => out,
                Ok(_) => text,
                Err(err) => {
                    tracing::warn!(err = %err, "Pipeline step variables failed; text kept as written");
                    text
                }
            }
        }) as InterpolateFuture
    });
    let outcome = run_pipeline_flow_action(
        &run.deps.db,
        ctx.step,
        &PipelineFlowActionContext {
            workspace_id,
            contact_id,
            flow_id: run.flow_version.flow_id,
            flow_execution_key: Some(run.flow_execution_key.clone()),
            interpolate,
        },
    )
    .await
    .map_err(|err| {
        tracing::error!(err = %err.message(), workspace_id, contact_id, step_id = ctx.str("id"), "Pipeline step failed");
        biz_fail(err)
    })?;
    if let Some(realtime) = &run.deps.realtime {
        for change in &outcome.events {
            let mut data = Map::new();
            data.insert("pipelineId".into(), change.pipeline_id.to_string().into());
            if let Some(item) = change.item_id {
                data.insert("itemId".into(), item.to_string().into());
            }
            if let Some(contact) = change.contact_id {
                data.insert("contactId".into(), contact.to_string().into());
            }
            data.insert("change".into(), change.change.into());
            let event = chatbotx_rt::Envelope::new(
                chatbotx_rt::EventType::PipelineItemChanged,
                Value::Object(data),
            );
            realtime
                .queue_workspace_event(&change.workspace_id.to_string(), event)
                .await;
        }
    }
    let bus = crate::order_payment::bus(run.deps);
    for trigger in outcome.triggers.iter().cloned() {
        bus.publish(chatbotx_business::events::Event::Trigger(trigger))
            .await;
    }
    match chatbotx_business::pipeline::assignees::apply_contact_assignments(
        &run.deps.db,
        &outcome.assignments,
    )
    .await
    {
        Ok(done) => {
            for (ws, assigned) in done {
                if let Err(err) =
                    crate::rich_response::publish_assigned(run.deps, ws, assigned).await
                {
                    tracing::warn!(%err, "Pipeline assignContact publish failed");
                }
            }
        }
        Err(err) => tracing::warn!(err = %err.message(), "Pipeline assignContact failed"),
    }
    let status = match outcome.status {
        PipelineFlowActionStatus::Success => return Ok(StepStatus::Success),
        PipelineFlowActionStatus::Error => StepStatus::Error,
        PipelineFlowActionStatus::Skip => StepStatus::Skip,
    };
    tracing::info!(
        workspace_id,
        contact_id,
        step_id = ctx.str("id"),
        action = ctx.str("action"),
        reason = outcome.message.as_deref(),
        "Pipeline step did not apply"
    );
    Ok(status)
}
