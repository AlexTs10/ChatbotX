//! Q.W3: the `flowStepHandlers` of `integration/handlers/step.ts` that are
//! not message sends (those stay in `flow.rs`). One handler per step type;
//! each returns the routing status (`ExecuteStepResult.status`), `success`
//! for the TypeScript `void` handlers.

pub(crate) mod ai_step;
mod appointment;
mod condition;
mod contact;
mod conversation;
mod coupon;
mod decision;
pub mod delay;
pub(crate) mod email_marketing;
mod external_request;
mod facebook_audience;
mod make;
mod messenger;
mod pipeline;
mod places;
mod reply;
mod tools;
pub(crate) mod working_hours;

use chatbotx_jobs::JobError;
use serde_json::{Map, Value, json};

use crate::flow::{Run, StepStatus};
use crate::str_of;

/// What a handler gets (`ExecuteStepProps`): the run, the step (with its
/// `nodeId`), the run's node visits and the comment anchor (only for a
/// message-producing step).
pub(crate) struct StepCtx<'a, 'r> {
    pub run: &'a Run<'r>,
    pub step: &'a Value,
    pub node_visits: Option<&'a Value>,
    pub anchor: Option<Value>,
}

impl StepCtx<'_, '_> {
    pub fn workspace_id(&self) -> i64 {
        self.run.conversation.workspace_id
    }

    pub fn contact_id(&self) -> i64 {
        self.run.conversation.contact_id
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        str_of(self.step, key)
    }

    /// A `sendFlow` job to `node_id` carrying only `keys` of the run
    /// (each handler names what it forwards, as the TypeScript literals do).
    pub fn send_flow_job(&self, node_id: &str, keys: &[&str]) -> Map<String, Value> {
        let run = self.run;
        let mut data = Map::new();
        data.insert(
            "conversationId".into(),
            json!(run.conversation.id.to_string()),
        );
        data.insert(
            "contactInboxId".into(),
            json!(run.contact_inbox.id.to_string()),
        );
        data.insert("flowId".into(), json!(run.flow_version.flow_id.to_string()));
        if let Some(version) = run.version_id() {
            data.insert("flowVersionId".into(), json!(version));
        }
        data.insert("nodeId".into(), json!(node_id));
        for key in keys {
            let value = match *key {
                "nodeVisits" => self.node_visits.cloned(),
                "commentAnchor" => self.anchor.clone(),
                "isBulkBroadcast" => run.is_bulk_broadcast.map(Value::Bool),
                other => run.data.get(other).cloned(),
            };
            if let Some(value) = value.filter(|v| !v.is_null()) {
                data.insert((*key).into(), value);
            }
        }
        data.insert("origin".into(), json!("webhook"));
        data
    }

    /// `resolveContactVariablesDeep(contactId, value, { contactInbox,
    /// conversation, appointmentId, sentByUserId })`; without variable deps
    /// (no `ENCRYPTION_KEY`) the value is returned unchanged.
    pub async fn resolve_variables(&self, value: &Value, spintax: bool) -> Result<Value, String> {
        let run = self.run;
        let Some(variables) = &run.deps.variables else {
            return Ok(value.clone());
        };
        variables
            .resolve(
                &run.conversation,
                &run.contact_inbox,
                value,
                str_of(run.data, "appointmentId").map(str::to_owned),
                str_of(run.data, "sentByUserId").map(str::to_owned),
                spintax,
            )
            .await
    }

    pub async fn enqueue(&self, data: Map<String, Value>) -> Result<(), JobError> {
        self.run.enqueue_send_flow(data, None).await
    }
}

/// An `{ status: "error", errorMessage }` result: the message is only logged
/// (the runner routes on the status alone).
pub(crate) fn step_error(ctx: &StepCtx<'_, '_>, message: &str) -> StepStatus {
    tracing::warn!(
        step_id = ctx.str("id").unwrap_or(""),
        step_type = ctx.str("stepType").unwrap_or(""),
        conversation_id = ctx.run.conversation.id,
        error_message = message,
        "Flow step returned an error"
    );
    StepStatus::Error
}

/// `flowStepHandlers[step.stepType]` for the non-message steps.
pub(crate) async fn run(
    run: &Run<'_>,
    step: &Value,
    node_visits: Option<&Value>,
    anchor: Option<Value>,
) -> Result<StepStatus, JobError> {
    let ctx = StepCtx {
        run,
        step,
        node_visits,
        anchor,
    };
    let step_type = ctx.str("stepType").unwrap_or("");
    if let Some(status) = contact::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = coupon::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = conversation::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = delay::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = tools::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = reply::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = ai_step::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = messenger::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(status) = email_marketing::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if let Some(action) = crate::heavy::action_of(step_type) {
        return crate::heavy::run_step(&ctx, action).await;
    }
    if let Some(status) = appointment::run(&ctx, step_type).await? {
        return Ok(status);
    }
    if crate::spreadsheet::STEP_TYPES.contains(&step_type) {
        let sheets = crate::spreadsheet::SheetRun {
            deps: run.deps,
            http: chatbotx_business::email_marketing::safe_http(),
            conversation: &run.conversation,
            contact_inbox: &run.contact_inbox,
            sent_by_user_id: crate::str_of(run.data, "sentByUserId").map(str::to_owned),
        };
        return Ok(
            match crate::spreadsheet::execute(&sheets, step_type, step).await {
                Ok(()) => StepStatus::Success,
                Err(failure) => {
                    crate::spreadsheet::report(&sheets, step_type, &failure).await;
                    step_error(&ctx, failure.message)
                }
            },
        );
    }
    match step_type {
        "make" | "triggerN8n" => make::handle(&ctx).await,
        "callApi" => external_request::handle(&ctx).await,
        "facebookCustomAudience" => facebook_audience::handle(&ctx).await,
        "condition" => condition::handle(&ctx).await,
        "pipeline" => pipeline::handle(&ctx).await,
        "findPlacesNearMe" => places::handle(&ctx).await,
        "sendGoogleAdsConversion" => Ok(crate::google_ads_producer::step(&ctx).await),
        "evaluateDecision" => decision::handle(&ctx).await,
        "changeGamificationScore" => crate::leaderboard::change_score_step(&ctx).await,
        // The dropped Execute JavaScript step (DECISIONS 2026-11-11) has no
        // handler: like any unknown step type it is an implicit success.
        "executeJavascript" => {
            tracing::warn!(
                flow_id = run.flow_version.flow_id,
                "Execute JavaScript step is not supported; skipped"
            );
            Ok(StepStatus::Success)
        }
        other => {
            // ponytail: steps of unported domains (ads, ecommerce, ...)
            // are implicit
            // successes until their units; see specs/Q.W3.md.
            tracing::warn!(
                step_type = other,
                flow_id = run.flow_version.flow_id,
                "Flow step handler is not ported yet; treated as success"
            );
            Ok(StepStatus::Success)
        }
    }
}

#[cfg(test)]
mod ai_tests;
#[cfg(test)]
mod send_flow_step_tests;
#[cfg(test)]
mod send_message_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod u4_tests;
