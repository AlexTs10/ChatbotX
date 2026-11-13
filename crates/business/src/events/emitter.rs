//! `TriggerEventEmitter` / `WebhookEventEmitter` (`trigger/emitter.ts`,
//! `webhook/emitter.ts`, `BaseEventEmitter.emit`): bus subscribers that
//! queue `evaluate-triggers` / `evaluate-webhooks` jobs when an active
//! trigger / webhook listens for the event.

use std::sync::Arc;

use chatbotx_jobs::{Enqueuer, JobOptions, JobsError, QueueName, Retention};
use serde_json::{Map, Value, json};

use super::cache::ActiveConditions;
use super::{BoxFuture, Event, Subscriber, SubscriberError, TriggerEvent};
use super::{context, registry};

/// Which emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitterKind {
    /// `trigger` queue, job `evaluate-triggers` (`evaluateTriggers`).
    Trigger,
    /// `webhook` queue, job `evaluate-webhooks` (`evaluateWebhooks`); only
    /// inside a webhook context or for a context-free type.
    Webhook,
}

/// One queue emitter.
pub struct QueueEmitter {
    kind: EmitterKind,
    jobs: Arc<dyn Enqueuer>,
    active: Arc<dyn ActiveConditions>,
}

impl QueueEmitter {
    /// `TriggerEventEmitter`.
    pub fn trigger(jobs: Arc<dyn Enqueuer>, active: Arc<dyn ActiveConditions>) -> QueueEmitter {
        QueueEmitter {
            kind: EmitterKind::Trigger,
            jobs,
            active,
        }
    }

    /// `WebhookEventEmitter`.
    pub fn webhook(jobs: Arc<dyn Enqueuer>, active: Arc<dyn ActiveConditions>) -> QueueEmitter {
        QueueEmitter {
            kind: EmitterKind::Webhook,
            jobs,
            active,
        }
    }

    pub fn kind(&self) -> EmitterKind {
        self.kind
    }

    /// `emit`: no workspace or contact id, or an unsupported type, does
    /// nothing; else `shouldEmitEvent` (the active-condition check, with the
    /// `metadata.sourceId`), then the queue add.
    pub async fn emit(&self, event: &TriggerEvent) -> Result<(), JobsError> {
        if event.workspace_id.is_empty() || event.contact_id.is_empty() {
            return Ok(());
        }
        if !registry::is_emitted(&event.event_type) {
            return Ok(());
        }
        if !self.should_emit(event).await? {
            return Ok(());
        }
        let (queue, name, job_type) = match self.kind {
            EmitterKind::Trigger => (QueueName::Trigger, "evaluate-triggers", "evaluateTriggers"),
            EmitterKind::Webhook => (QueueName::Webhook, "evaluate-webhooks", "evaluateWebhooks"),
        };
        let mut data = Map::new();
        data.insert(
            "workspaceId".into(),
            Value::from(event.workspace_id.as_str()),
        );
        data.insert("contactId".into(), Value::from(event.contact_id.as_str()));
        data.insert("eventType".into(), Value::from(event.event_type.as_str()));
        data.insert("eventData".into(), Value::Object(event.metadata.clone()));
        data.insert(
            "timestamp".into(),
            Value::from(chatbotx_db::repo::iso(now_ms())),
        );
        if self.kind == EmitterKind::Trigger {
            data.insert(
                "channelOriginated".into(),
                Value::Bool(context::webhook_channel_origin() == Some("channel")),
            );
        }
        let opts = JobOptions {
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        };
        self.jobs
            .add(queue, name, json!({ "type": job_type, "data": data }), opts)
            .await?;
        Ok(())
    }

    /// `shouldEmitEvent`.
    async fn should_emit(&self, event: &TriggerEvent) -> Result<bool, JobsError> {
        let event_type = event.event_type.as_str();
        if self.kind == EmitterKind::Webhook
            && !(context::is_webhook_context() || registry::is_context_free(event_type))
        {
            tracing::debug!(
                event_type,
                workspace_id = %event.workspace_id,
                source_id = ?event.source_id(),
                "Skipping webhook event outside channel-originated context"
            );
            return Ok(false);
        }
        let active = self
            .active
            .has_active(&event.workspace_id, &[event_type], event.source_id())
            .await
            .map_err(JobsError::Other)?;
        if !active && self.kind == EmitterKind::Webhook {
            tracing::debug!(
                event_type,
                workspace_id = %event.workspace_id,
                source_id = ?event.source_id(),
                "Skipping webhook event because no active webhooks match"
            );
        }
        Ok(active)
    }
}

impl Subscriber for QueueEmitter {
    fn name(&self) -> &str {
        match self.kind {
            EmitterKind::Trigger => "TriggerEventEmitter",
            EmitterKind::Webhook => "WebhookEventEmitter",
        }
    }

    fn handle<'a>(&'a self, event: &'a Event) -> BoxFuture<'a, Result<(), SubscriberError>> {
        Box::pin(async move {
            match event {
                Event::Trigger(trigger) => self.emit(trigger).await.map_err(|err| err.to_string()),
                Event::Bus { .. } => Ok(()),
            }
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
