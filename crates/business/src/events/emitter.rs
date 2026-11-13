//! `TriggerEventEmitter` / `WebhookEventEmitter` (`trigger/emitter.ts`,
//! `webhook/emitter.ts`, `BaseEventEmitter.emit`): bus subscribers that
//! queue `evaluate-triggers` / `evaluate-webhooks` jobs when an active
//! trigger / webhook listens for the event.

use std::sync::Arc;

use chatbotx_jobs::{Enqueuer, JobsError};

use super::cache::ActiveConditions;
use super::{BoxFuture, Event, Subscriber, SubscriberError, TriggerEvent};

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
        let _ = (&self.jobs, &self.active, event);
        todo!("P3.6-X1")
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
