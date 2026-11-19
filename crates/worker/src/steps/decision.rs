//! `handlers/evaluate-decision.ts` (`evaluateDecision`): the profile's
//! answer is mapped to custom fields; the outcome picks the success, skip
//! or error state.

use std::sync::LazyLock;

use chatbotx_business::decision::{EvalDeps, FlowEval, Outcome, evaluate_flow};
use chatbotx_business::events::Event;
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_jobs::JobError;
use chatbotx_kv::lock::LockMap;
use serde_json::Value;

use super::StepCtx;
use crate::flow::StepStatus;

/// `distributedLock` of `decision:eval:*` (one process).
static EVAL_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    let key = if run.flow_execution_key.is_empty() {
        format!("flow-inline-{}", chatbotx_db::repo::new_id())
    } else {
        run.flow_execution_key.clone()
    };
    let step_id = ctx.str("id").unwrap_or("");
    let deps = EvalDeps {
        db: &run.deps.db,
        encryption: run.deps.variables.as_ref().map(|v| &v.encryption),
        locks: Some(&EVAL_LOCKS),
        doh: &HttpDoh,
    };
    let outcome = evaluate_flow(
        &deps,
        FlowEval {
            workspace_id: ctx.workspace_id(),
            contact_id: ctx.contact_id(),
            flow_id: run.flow_version.flow_id,
            execution_key: format!("decision-{}", sha256_hex(&format!("{key}:{step_id}"))),
            step: ctx.step,
        },
    )
    .await;
    Ok(match outcome {
        Outcome::Success(changes) => {
            emit_changes(ctx, changes).await;
            StepStatus::Success
        }
        Outcome::Skip(_) => StepStatus::Skip,
        Outcome::Error(reason) => super::step_error(ctx, reason),
    })
}

/// `emitCustomFieldChanges` (`customFieldChanged` triggers).
async fn emit_changes(
    ctx: &StepCtx<'_, '_>,
    changes: Vec<chatbotx_business::contact_custom_field::FieldChange>,
) {
    let ws = ctx.workspace_id().to_string();
    let inbox = ctx.run.contact_inbox.id.to_string();
    let bus = crate::order_payment::bus(ctx.run.deps);
    for change in changes {
        let event = chatbotx_business::events::trigger::custom_field_changed(
            &ws,
            &change.contact_id.to_string(),
            &change.custom_field_id.to_string(),
            &change.custom_field_name,
            change.old_value.map_or(Value::Null, Value::String),
            change.new_value.map_or(Value::Null, Value::String),
            Some(&inbox),
        );
        bus.publish(Event::Trigger(event)).await;
    }
}
