//! `copy-text.ts` / `call-phone-number.ts` (`replyWithCopyText`,
//! `replyWithCallPhoneNumber`): a Copy Text / Call Phone Number button sent
//! as a reply answers the tap with the text (or number) as a text message.
//! The value is resolved again in the send-time context.

use chatbotx_core::flow_config::normalize_dialable_phone_number;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::StepCtx;
use crate::flow::{StepStatus, send_flow_message};

/// The reply steps; `None` = not a reply step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    let (key, reason) = match step_type {
        "copyText" => (
            "text",
            "Not replying to a Copy Text tap: the text resolved to nothing",
        ),
        "callPhoneNumber" => (
            "phoneNumber",
            "Not replying to a Call Phone Number tap: the number is not a valid international number",
        ),
        _ => return Ok(None),
    };
    let raw = ctx.step.get(key).cloned().unwrap_or(Value::Null);
    let resolved = ctx
        .resolve_variables(&raw, true)
        .await
        .map_err(JobError::retryable)?;
    let resolved = resolved.as_str().unwrap_or("");
    let text = if step_type == "copyText" {
        Some(resolved.trim().to_owned()).filter(|t| !t.is_empty())
    } else {
        normalize_dialable_phone_number(resolved)
    };
    let Some(text) = text else {
        tracing::warn!(
            workspace_id = ctx.workspace_id(),
            conversation_id = ctx.run.conversation.id,
            step_id = ctx.str("id"),
            "{reason}"
        );
        return Ok(Some(StepStatus::Success));
    };
    let mut reply =
        json!({ "id": ctx.step.get("id"), "stepType": "sendText", "text": text, "buttons": [] });
    if !ctx.run.target_node_id.is_empty() {
        reply["nodeId"] = json!(ctx.run.target_node_id);
    }
    send_flow_message(ctx.run, &reply, ctx.anchor.clone(), None).await?;
    Ok(Some(StepStatus::Success))
}
