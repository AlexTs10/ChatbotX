//! `integration/handlers/{challenge,step-challenge-target,
//! step-input-resolution}.ts`: a pending step challenge re-enters its step,
//! with a typed reply (`runChallenge`), or without one: the Skip tap
//! (`resolveStepInput`) or the step's timer (`resumeStepInputTimeout`).
//!
//! The runtime variables of `buildStepChallengeVariables` ride on the run's
//! job data as `stepChallenge` (`{ attempts, lastAttemptAt, startedAt?,
//! challengeId?, resolution? }`). Continuations copy only the carried keys, so
//! only the resumed step reads them.

use chatbotx_business::conversation_challenge::consume_challenge;
use chatbotx_business::smart_delay as service;
use chatbotx_core::flow_config::step_input_resolution::STEP_INPUT_RESOLUTION_PAYLOAD_TYPE;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::JobError;
use serde_json::{Map, Value, json};

use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

pub const RUN_CHALLENGE: &str = "runChallenge";
pub const RESOLVE_STEP_INPUT: &str = "resolveStepInput";
pub const RESUME_STEP_INPUT_TIMEOUT: &str = "resumeStepInputTimeout";

/// The data key of the challenge variables.
pub(crate) const STEP_CHALLENGE: &str = "stepChallenge";

/// A JSON id (`"12"` or `12`) as text.
pub(crate) fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The step challenge of `additionalAttributes`, if one is pending.
pub(crate) fn pending_step_challenge(conversation: &Conversation) -> Option<Value> {
    conversation
        .additional_attributes
        .as_ref()
        .and_then(|a| a.0.get("challenge"))
        .filter(|c| c["type"] == "step")
        .cloned()
}

async fn load(deps: &Deps, data: &Value) -> Result<(Conversation, ContactInbox), JobError> {
    let conversation = match data.get("conversationId").and_then(id_of) {
        Some(id) => load_conversation(&deps.db, id).await?,
        None => None,
    }
    .ok_or_else(|| fail("Conversation not found"))?;
    let contact_inbox = match data.get("contactInboxId").and_then(id_of) {
        Some(id) => load_contact_inbox(&deps.db, id, conversation.contact_id).await?,
        None => None,
    }
    .ok_or_else(|| fail("Contact inbox not found"))?;
    Ok((conversation, contact_inbox))
}

/// `resolveStepChallengeTarget`: the flow, node and step still exist.
async fn target_exists(
    deps: &Deps,
    workspace_id: i64,
    data: &Value,
) -> Result<Result<(), &'static str>, JobError> {
    let Some(flow_id) = data.get("flowId").and_then(id_of) else {
        return Ok(Err("FlowVersion not found"));
    };
    let version_id = data.get("flowVersionId").and_then(id_of);
    let nodes: Option<String> = deps
        .db
        .read(move |conn| {
            use chatbotx_db::rusqlite::OptionalExtension as _;
            Ok(conn
                .query_row(
                    r#"SELECT "nodes" FROM "FlowVersion" WHERE "workspaceId" = ?1 AND "id" = COALESCE(?2,
                         (SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?3 AND "workspaceId" = ?1 AND "active" = 1))"#,
                    chatbotx_db::rusqlite::params![workspace_id, version_id, flow_id],
                    |r| r.get(0),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some(nodes) = nodes.and_then(|n| serde_json::from_str::<Value>(&n).ok()) else {
        return Ok(Err("FlowVersion not found"));
    };
    let Some(node) = nodes
        .as_array()
        .and_then(|n| n.iter().find(|n| n["id"] == data["nodeId"]))
    else {
        return Ok(Err("Target node not found"));
    };
    let Some(steps) = node["data"]["details"]["steps"].as_array() else {
        return Ok(Err("Target node does not have steps"));
    };
    if !steps
        .iter()
        .any(|s| id_text(&s["id"]) == id_text(&data["stepId"]))
    {
        return Ok(Err("Target step not found"));
    }
    Ok(Ok(()))
}

/// How a step challenge run re-enters its step.
pub(crate) struct Reentry<'a> {
    pub conversation: &'a Conversation,
    pub contact_inbox: &'a ContactInbox,
    pub challenge: &'a Value,
    pub message_id: Option<&'a str>,
    pub message_created_at: Option<&'a Value>,
    pub started_by_contact_message: bool,
    pub resolution: Option<&'a str>,
    /// Drop the challenge (instead of failing the job) when its target is gone.
    pub drop_when_unavailable: bool,
}

/// `runStepChallenge`: runs the challenge's node from its step, inline.
pub(crate) async fn run_step_challenge(deps: &Deps, r: Reentry<'_>) -> Result<(), JobError> {
    let data = &r.challenge["data"];
    let workspace_id = r.conversation.workspace_id;
    if let Err(reason) = target_exists(deps, workspace_id, data).await? {
        if !r.drop_when_unavailable {
            return Err(fail(reason));
        }
        // The flow was edited or republished since the question was asked:
        // drop the challenge, or it would block keywords and AI.
        if let (Some(step_id), Some(challenge_id)) =
            (id_text(&data["stepId"]), id_text(&data["challengeId"]))
        {
            consume_challenge(
                &deps.db,
                workspace_id,
                r.conversation.id,
                &step_id,
                &challenge_id,
            )
            .await
            .map_err(|e| fail(e.to_string()))?;
        }
        tracing::warn!(
            conversation_id = r.conversation.id,
            reason,
            "runStepInputResolution: step no longer exists, cleared the challenge"
        );
        return Ok(());
    }
    let mut variables = Map::new();
    variables.insert("attempts".into(), data["attempts"].clone());
    variables.insert("lastAttemptAt".into(), data["lastAttemptAt"].clone());
    for key in ["startedAt", "challengeId"] {
        if let Some(v) = data.get(key).filter(|v| !v.is_null()) {
            variables.insert(key.into(), v.clone());
        }
    }
    if let Some(reason) = r.resolution {
        variables.insert("resolution".into(), json!(reason));
    }
    let mut run = Map::new();
    run.insert(
        "conversationId".into(),
        json!(r.conversation.id.to_string()),
    );
    run.insert(
        "contactInboxId".into(),
        json!(r.contact_inbox.id.to_string()),
    );
    for (to, from) in [
        ("flowId", "flowId"),
        ("flowVersionId", "flowVersionId"),
        ("nodeId", "nodeId"),
        ("startFromStepId", "stepId"),
        ("appointmentId", "appointmentId"),
    ] {
        if let Some(v) = data.get(from).filter(|v| !v.is_null()) {
            run.insert(to.into(), v.clone());
        }
    }
    if let Some(id) = r.message_id {
        run.insert("triggerMessageId".into(), json!(id));
    }
    if let Some(at) = r.message_created_at.filter(|v| !v.is_null()) {
        run.insert("triggerMessageCreatedAt".into(), at.clone());
    }
    run.insert(
        "startedByContactMessage".into(),
        json!(r.started_by_contact_message),
    );
    run.insert(STEP_CHALLENGE.into(), Value::Object(variables));
    // ponytail: the `message:bot_received` dashboard events of the original
    // come with the analytics producers (the inbound path emits none either).
    let key = crate::integration::mint_random_flow_execution_key("flow-inline-");
    crate::flow::run_flow_node(deps, &Value::Object(run), key).await
}

/// `runChallenge`.
pub async fn run_challenge(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (conversation, contact_inbox) = load(deps, data).await?;
    let challenge = &data["challenge"];
    match challenge["type"].as_str() {
        // ponytail: the quick-reply retry challenge is never written in Rust
        // yet (the quick-reply unit arms it); a stray one is left alone.
        Some("quickReply") => {
            tracing::warn!(
                conversation_id = conversation.id,
                "Quick reply challenge resume is not ported; ignored"
            );
            Ok(())
        }
        Some("step") => {
            run_step_challenge(
                deps,
                Reentry {
                    conversation: &conversation,
                    contact_inbox: &contact_inbox,
                    challenge,
                    message_id: str_of(data, "messageId"),
                    message_created_at: data.get("messageCreatedAt"),
                    started_by_contact_message: true,
                    resolution: None,
                    drop_when_unavailable: false,
                },
            )
            .await
        }
        _ => Ok(()),
    }
}

/// `runStepInputResolution`: ends a pending step input without a typed
/// answer; the step claims the challenge, so a duplicate resolves once.
pub async fn resolve_step_input(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (conversation, contact_inbox) = load(deps, data).await?;
    let challenge_id = id_text(&data["challengeId"]);
    let Some(challenge) = pending_step_challenge(&conversation)
        .filter(|c| challenge_id.is_some() && id_text(&c["data"]["challengeId"]) == challenge_id)
    else {
        tracing::info!(
            conversation_id = conversation.id,
            "runStepInputResolution: challenge already resolved or replaced, ignoring"
        );
        return Ok(());
    };
    // Re-checked: the job may have waited since the webhook gate ran.
    if !(crate::inbound::workspace_is_active(deps, conversation.workspace_id).await?
        && crate::inbound::ensure_active(deps, &conversation).await)
    {
        tracing::info!(
            conversation_id = conversation.id,
            "runStepInputResolution: workspace or bot inactive, leaving the challenge"
        );
        return Ok(());
    }
    let message_id = str_of(data, "messageId");
    run_step_challenge(
        deps,
        Reentry {
            conversation: &conversation,
            contact_inbox: &contact_inbox,
            challenge: &challenge,
            message_id,
            message_created_at: data.get("messageCreatedAt"),
            // Only the contact's own tap counts as contact-initiated.
            started_by_contact_message: message_id.is_some(),
            resolution: str_of(data, "reason"),
            drop_when_unavailable: true,
        },
    )
    .await
}

/// `runStepInputTimeoutResume`.
pub async fn resume_step_input_timeout(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(id) = data.get("smartDelayId").and_then(id_of) else {
        return Ok(());
    };
    let Some(row) = service::find_by_id(&deps.db, id).await.map_err(db_fail)? else {
        return Ok(());
    };
    // A restarted timer supersedes this delayed job; the new job fires later.
    if row.r#type != "stepInputTimeout" || row.status != "scheduled" || row.trigger_at > now_ms() {
        return Ok(());
    }
    let payload = row
        .metadata
        .as_ref()
        .map(|m| m.0.clone())
        .unwrap_or_default();
    let reason = str_of(&payload, "reason").filter(|r| matches!(*r, "timeout" | "user_skipped"));
    let challenge_id = id_text(&payload["challengeId"]).filter(|c| !c.is_empty());
    let (Some(reason), Some(challenge_id), true) = (
        reason,
        challenge_id,
        str_of(&payload, "type") == Some(STEP_INPUT_RESOLUTION_PAYLOAD_TYPE)
            && str_of(&payload, "stepId").is_some(),
    ) else {
        service::claim_for_run(&deps.db, row.id, "canceled", Some(row.trigger_at))
            .await
            .map_err(db_fail)?;
        tracing::warn!(
            smart_delay_id = row.id,
            "Step input timeout canceled: unreadable payload"
        );
        return Ok(());
    };
    // Tied to the deadline read above: a re-armed row is not consumed here.
    if !service::claim_for_run(&deps.db, row.id, "completed", Some(row.trigger_at))
        .await
        .map_err(db_fail)?
    {
        return Ok(());
    }
    let resolve = json!({
        "conversationId": row.conversation_id.to_string(),
        "contactInboxId": row.contact_inbox_id.to_string(),
        "challengeId": challenge_id,
        "reason": reason,
    });
    if let Err(err) = resolve_step_input(deps, &resolve).await {
        crate::smart_delay::requeue_claimed_run_or_log(
            deps,
            row.id,
            "Failed to requeue a claimed step input timeout after failure",
        )
        .await;
        return Err(err);
    }
    Ok(())
}
