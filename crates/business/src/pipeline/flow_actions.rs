//! `pipeline/flow-actions.ts`: the Pipelines flow step / trigger actions
//! (`runPipelineAction`, `runPipelineFlowAction`,
//! `PIPELINE_FLOW_ACTION_HANDLERS`). The step is the flow-config JSON
//! (`pipelineStepSchema` / a trigger action); ids in it are strings.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chatbotx_db::Db;
use serde_json::Value;

use super::events::PipelineItemChanged;
use super::mutation::PipelineActor;
use crate::errors::Result;
use crate::events::TriggerEvent;

/// The future an [`Interpolate`] returns.
pub type InterpolateFuture = Pin<Box<dyn Future<Output = String> + Send>>;

/// `interpolate`: resolves `{{variables}}` for the contact.
pub type Interpolate = Arc<dyn Fn(String) -> InterpolateFuture + Send + Sync>;

/// An [`Interpolate`] from a synchronous function.
pub fn interpolate_fn(f: impl Fn(&str) -> String + Send + Sync + 'static) -> Interpolate {
    Arc::new(move |text: String| {
        let out = f(&text);
        Box::pin(async move { out }) as InterpolateFuture
    })
}

/// `PipelineFlowActionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFlowActionStatus {
    Success,
    Error,
    Skip,
}

/// `PipelineFlowActionResult`, plus the realtime events and flow-trigger
/// events of the write (the caller publishes them; the business crate has no
/// realtime handle or event bus).
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineFlowActionResult {
    pub status: PipelineFlowActionStatus,
    /// Why the step errored or skipped; written to logs, never shown to the contact.
    pub message: Option<String>,
    pub events: Vec<PipelineItemChanged>,
    pub triggers: Vec<TriggerEvent>,
}

/// `PipelineFlowActionContext`.
#[derive(Clone)]
pub struct PipelineFlowActionContext {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub flow_id: i64,
    /// Stable per flow run; present only when the executor supplied one.
    pub flow_execution_key: Option<String>,
    pub interpolate: Interpolate,
}

/// `PipelineActionContext`: what runs a pipeline action (a flow step, a trigger, ...).
#[derive(Clone)]
pub struct PipelineActionContext {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub actor: PipelineActor,
    /// Durable key of this occurrence of the action, identical on every retry.
    pub base_idempotency_key: Option<String>,
    pub interpolate: Interpolate,
}

/// `PIPELINE_FLOW_ACTION_HANDLERS`: the actions that have a handler.
pub const PIPELINE_FLOW_ACTION_HANDLERS: &[&str] = &[];

/// `runPipelineAction`: never fails for caller-fixable problems (a missing
/// target is `skip`, an exception is `error`); a repository error is `Err`.
pub async fn run_pipeline_action(
    _db: &Db,
    _action: &Value,
    _ctx: &PipelineActionContext,
) -> Result<PipelineFlowActionResult> {
    todo!("P3.6-D1p flow actions")
}

/// `runPipelineFlowAction`: one Pipelines flow step, attributed to its flow.
pub async fn run_pipeline_flow_action(
    _db: &Db,
    _step: &Value,
    _ctx: &PipelineFlowActionContext,
) -> Result<PipelineFlowActionResult> {
    todo!("P3.6-D1p flow actions")
}
