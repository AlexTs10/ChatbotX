//! The two Meta CAPI producers (`send-meta-capi-event-step-handler.ts`,
//! the `sendMetaCapiEvent` trigger action, `capi-input-error.ts`): the
//! `{{variable}}` fields are resolved, then `metaConversionsService.
//! enqueueEvent` runs; a field the schema rejects lands in the Error Log.

use chatbotx_business::meta_capi_send::{
    self as capi, EnqueueError, EnqueueInput, build_source_key, capi_channel,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use serde_json::{Value, json};

use crate::Deps;

/// One producer call.
pub(crate) struct Produce<'a> {
    pub deps: &'a Deps,
    pub conversation: &'a Conversation,
    pub contact_inbox: &'a ContactInbox,
    /// `flowStep` | `triggerAction`.
    pub source: &'static str,
    /// `flow` | `trigger`.
    pub scope: &'static str,
    pub scope_id: String,
    /// The step / the parsed trigger action.
    pub fields: &'a Value,
    pub sent_by_user_id: Option<String>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// `resolveContactVariablesDeep({ value, currency, contentIds })` +
/// `enqueueCapiEvent`. `Err` is the sanitized message.
pub(crate) async fn produce(p: Produce<'_>) -> Result<(), String> {
    let channel_name = p.contact_inbox.channel.clone();
    let Some(channel) = capi_channel(&channel_name) else {
        return Err(format!("Unsupported Meta CAPI channel: {channel_name}"));
    };
    let mut templates = serde_json::Map::new();
    for field in ["value", "currency", "contentIds"] {
        if let Some(v) = text(p.fields, field) {
            templates.insert(field.to_owned(), json!(v));
        }
    }
    let templates = Value::Object(templates);
    let resolved = match &p.deps.variables {
        Some(variables) => {
            variables
                .resolve(
                    p.conversation,
                    p.contact_inbox,
                    &templates,
                    None,
                    p.sent_by_user_id.clone(),
                    false,
                )
                .await?
        }
        None => templates,
    };
    let action_source = text(p.fields, "actionSource");
    let input = EnqueueInput {
        workspace_id: p.conversation.workspace_id,
        channel: channel_name,
        contact_inbox_id: p.contact_inbox.id,
        inbox_id: p.contact_inbox.inbox_id,
        source: p.source.to_owned(),
        source_key: build_source_key(
            p.scope,
            &p.scope_id,
            p.contact_inbox.id,
            channel,
            action_source.as_deref(),
        ),
        event_name: text(p.fields, "eventName"),
        action_source,
        content_type: text(p.fields, "contentType"),
        content_ids: text(&resolved, "contentIds"),
        value: text(&resolved, "value"),
        currency: text(&resolved, "currency"),
        content_category: text(p.fields, "contentCategory"),
        content_name: text(p.fields, "contentName"),
    };
    match capi::enqueue_event(&p.deps.db, p.deps.jobs.as_ref(), input).await {
        Ok(_) => Ok(()),
        Err(EnqueueError::Invalid(issues)) => {
            let shown: Vec<(&str, Option<String>)> = ["value", "currency", "contentIds"]
                .iter()
                .map(|f| (*f, text(&resolved, f)))
                .collect();
            let shown: Vec<(&str, Option<&str>)> =
                shown.iter().map(|(f, v)| (*f, v.as_deref())).collect();
            capi::report_input_failure(
                p.deps.events.as_ref(),
                p.conversation.workspace_id,
                p.conversation.contact_id,
                &p.contact_inbox.source_id,
                capi::describe_input_failure(&issues, &shown),
            )
            .await;
            Err(issues)
        }
        Err(EnqueueError::Failed(message)) => Err(message),
    }
}

/// `handleSendMetaCapiEventStep`: `success`, else `error` with the message.
pub(crate) async fn step(ctx: &crate::steps::StepCtx<'_, '_>) -> crate::flow::StepStatus {
    let run = ctx.run;
    let step_id = ctx.str("id").unwrap_or_default().to_owned();
    let produced = produce(Produce {
        deps: run.deps,
        conversation: &run.conversation,
        contact_inbox: &run.contact_inbox,
        source: "flowStep",
        scope: "flow",
        scope_id: step_id.clone(),
        fields: ctx.step,
        sent_by_user_id: crate::str_of(run.data, "sentByUserId").map(str::to_owned),
    })
    .await;
    match produced {
        Ok(()) => crate::flow::StepStatus::Success,
        Err(message) => {
            tracing::warn!(
                err = message,
                workspace_id = run.conversation.workspace_id,
                conversation_id = run.conversation.id,
                contact_inbox_id = run.contact_inbox.id,
                step_id,
                "Failed to enqueue Meta CAPI flow step event"
            );
            crate::steps::step_error(ctx, &message)
        }
    }
}
