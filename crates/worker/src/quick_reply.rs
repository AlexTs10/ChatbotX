//! `quick-reply-settings.ts` + `quick-reply-resume.ts`: a node's quick-reply
//! retry (a typed reply re-asks, then routes) and follow-up (a timer that
//! routes when the contact does not answer).

use chatbotx_business::conversation_challenge::{
    ClearQuickReply, arm_quick_reply_challenge, clear_quick_reply_challenge,
    set_quick_reply_challenge_attempts,
};
use chatbotx_core::flow_config::steps::{
    compute_quick_reply_follow_up_trigger_at, resolve_active_quick_reply_settings,
};
use chatbotx_db::repo::models::{ContactInbox, ContactOnSmartDelay, Conversation, FlowVersion};
use chatbotx_db::repo::{Json, iso, new_id, now_ms};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::flow::Run;
use crate::steps::delay::SmartDelayRow;
use crate::{Deps, fail, id_of, str_of};

fn node_details<'a>(flow_version: &'a FlowVersion, node_id: &str) -> Option<&'a Value> {
    flow_version
        .nodes
        .0
        .as_array()?
        .iter()
        .find(|n| str_of(n, "id") == Some(node_id))
        .map(|n| &n["data"]["details"])
}

/// `smartDelayService.cancelQuickReplyFollowUps` for one contact inbox.
async fn cancel_follow_ups(
    deps: &Deps,
    workspace_id: i64,
    contact_inbox_id: i64,
    except_node_id: Option<String>,
    except_flow_id: Option<i64>,
) -> chatbotx_business::errors::Result<usize> {
    chatbotx_business::tx::write(&deps.db, move |conn| {
        chatbotx_business::smart_delay::cancel_quick_reply_follow_ups_in(
            conn,
            workspace_id,
            Some(contact_inbox_id),
            &[],
            except_node_id.as_deref(),
            except_flow_id,
        )
    })
    .await
}

async fn clear_node(
    deps: &Deps,
    conversation: &Conversation,
    node_id: &str,
    attempts: Option<i64>,
) -> Result<bool, JobError> {
    let filter = ClearQuickReply {
        node_id: Some(node_id.to_owned()),
        attempts,
        ..ClearQuickReply::default()
    };
    clear_quick_reply_challenge(&deps.db, conversation.workspace_id, conversation.id, filter)
        .await
        .map_err(|e| fail(e.message()))
}

/// `armQuickReplySettings`: after the node's quick replies went out, arm
/// the retry challenge and the follow-up timer (or clear a stale one).
/// Never fails the run.
pub(crate) async fn arm(run: &Run<'_>, details: &Value) {
    let deps = run.deps;
    let (ws, conversation_id) = (run.conversation.workspace_id, run.conversation.id);
    let node_id = run.target_id.clone();
    let active = resolve_active_quick_reply_settings(Some(details));
    let (retry, follow_up) = (active.get("retry"), active.get("followUp"));
    let send_from = str_of(run.data, "sendFrom");
    let can_arm = (retry.is_some() || follow_up.is_some())
        && send_from != Some("inbox")
        && crate::inbound::ensure_active(deps, &run.conversation).await;
    let flow_id = run.flow_version.flow_id;
    let armed = match retry.filter(|_| can_arm) {
        Some(retry) => {
            let mut data = json!({
                "flowId": flow_id.to_string(),
                "nodeId": node_id,
                "attempts": 0,
                "maxRetries": retry["maxRetries"],
                "sentAt": iso(now_ms()),
            });
            if let Some(version) = run.version_id() {
                data["flowVersionId"] = json!(version);
            }
            // Only while the bot is on: a pause that lands first wins.
            let challenge = json!({ "type": "quickReply", "data": data });
            arm_quick_reply_challenge(&deps.db, ws, conversation_id, challenge).await
        }
        None => {
            clear_quick_reply_challenge(&deps.db, ws, conversation_id, ClearQuickReply::default())
                .await
                .map(|_| true)
        }
    };
    let canceled =
        cancel_follow_ups(deps, ws, run.contact_inbox.id, Some(node_id.clone()), None).await;
    let armed = match (armed, canceled) {
        (Ok(armed), Ok(_)) => armed,
        (Err(err), _) | (_, Err(err)) => {
            tracing::warn!(err = %err, node_id, conversation_id, "armQuickReplySettings: failed to arm quick reply settings");
            return;
        }
    };
    let Some(follow_up) = follow_up.filter(|_| can_arm && armed) else {
        return;
    };
    let trigger_at =
        compute_quick_reply_follow_up_trigger_at(follow_up, &chatbotx_core::SystemClock)
            .timestamp_millis();
    let row = ContactOnSmartDelay {
        id: new_id(),
        workspace_id: ws,
        flow_id,
        flow_version_id: run.version_id().and_then(|v| v.parse().ok()),
        contact_inbox_id: run.contact_inbox.id,
        conversation_id,
        node_id: Some(node_id.clone()),
        step_id: str_of(follow_up, "id").map(str::to_owned),
        metadata: run.metadata().cloned().map(Json),
        r#type: "quickReplyFollowUp".into(),
        created_at: now_ms(),
        trigger_at,
        status: "pending".into(),
        appointment_id: None,
    };
    let extras = crate::smart_delay::ResumeExtras {
        metadata: run.metadata().cloned(),
        send_from: send_from.map(str::to_owned),
        ..Default::default()
    };
    if let Err(err) = crate::smart_delay::schedule_smart_delay_resume(deps, row, extras).await {
        tracing::warn!(err = %err, node_id, conversation_id, "armQuickReplySettings: failed to arm quick reply settings");
    }
}

/// `clearQuickReplyPendingOnFlowEntry`: entering a node of another flow
/// ends that flow's pending retry and follow-ups. Never fails the run.
pub(crate) async fn clear_on_entry(run: &Run<'_>) {
    let (ws, flow_id) = (run.conversation.workspace_id, run.flow_version.flow_id);
    let filter = ClearQuickReply {
        except_flow_id: Some(flow_id.to_string()),
        ..ClearQuickReply::default()
    };
    let cleared = clear_quick_reply_challenge(&run.deps.db, ws, run.conversation.id, filter).await;
    let canceled = cancel_follow_ups(run.deps, ws, run.contact_inbox.id, None, Some(flow_id)).await;
    if let Err(err) = cleared.map(|_| ()).and(canceled.map(|_| ())) {
        tracing::warn!(err = %err, conversation_id = run.conversation.id, "clearQuickReplyPendingOnFlowEntry: failed to clear pending state");
    }
}

/// `routeQuickReplyTarget`: run a retry / follow-up section's target as a
/// quick reply of `source_node_id`.
#[allow(clippy::too_many_arguments)]
async fn route_target(
    deps: &Deps,
    conversation: Conversation,
    contact_inbox: ContactInbox,
    flow_version: FlowVersion,
    use_latest: bool,
    source_node_id: &str,
    section: &Value,
    metadata: Option<Value>,
) -> Result<(), JobError> {
    let mut details = json!({ "id": section["id"], "label": "" });
    if let Some(target) = section["target"].as_object() {
        for (key, value) in target {
            details[key] = value.clone();
        }
    }
    details["steps"] = json!([]);
    let mut data = json!({
        "conversationId": conversation.id.to_string(),
        "contactInboxId": contact_inbox.id.to_string(),
        "flowId": flow_version.flow_id.to_string(),
    });
    if let Some(metadata) = metadata {
        data["metadata"] = metadata;
    }
    let run = Run {
        deps,
        data: &data,
        conversation,
        contact_inbox,
        flow_version,
        use_latest_flow_version: use_latest,
        target_type: "quickReply",
        target_id: str_of(section, "id").unwrap_or_default().to_owned(),
        target_node_id: source_node_id.to_owned(),
        flow_execution_key: crate::integration::mint_random_flow_execution_key("flow-inline-"),
        is_bulk_broadcast: None,
    };
    crate::flow::run_details(&run, &details).await
}

/// `runQuickReplyChallenge`: a typed reply while quick replies wait. Under
/// the retry limit the question is asked again; at it, the retry target runs.
pub(crate) async fn run_challenge(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    challenge: &Value,
) -> Result<(), JobError> {
    let data = &challenge["data"];
    let node_id = str_of(data, "nodeId").unwrap_or_default().to_owned();
    if !crate::inbound::ensure_active(deps, conversation).await {
        clear_node(deps, conversation, &node_id, None).await?;
        tracing::info!(
            conversation_id = conversation.id,
            node_id,
            "runQuickReplyChallenge: bot inactive, cleared challenge"
        );
        return Ok(());
    }
    let resolved = match data.get("flowId").and_then(id_of) {
        Some(flow_id) => {
            crate::flow::find_flow_version(
                deps,
                flow_id,
                data.get("flowVersionId").and_then(id_of),
                conversation.workspace_id,
            )
            .await?
        }
        None => None,
    };
    let Some((flow_version, use_latest)) = resolved else {
        clear_node(deps, conversation, &node_id, None).await?;
        tracing::warn!(
            conversation_id = conversation.id,
            node_id,
            "runQuickReplyChallenge: flow version unavailable, cleared challenge"
        );
        return Ok(());
    };
    let details = node_details(&flow_version, &node_id).cloned();
    let active = resolve_active_quick_reply_settings(details.as_ref());
    let Some(retry) = active.get("retry") else {
        clear_node(deps, conversation, &node_id, None).await?;
        tracing::info!(
            conversation_id = conversation.id,
            node_id,
            "runQuickReplyChallenge: retry no longer configured, cleared"
        );
        return Ok(());
    };
    let attempts = data["attempts"].as_i64().unwrap_or(0);
    let max_retries = retry["maxRetries"].as_f64().unwrap_or(0.0) as i64;
    let ws = conversation.workspace_id;
    if attempts < max_retries {
        let set = |from, to| {
            set_quick_reply_challenge_attempts(&deps.db, ws, conversation.id, &node_id, from, to)
        };
        if !set(attempts, attempts + 1)
            .await
            .map_err(|e| fail(e.message()))?
        {
            return Ok(());
        }
        let mut job = json!({
            "conversationId": conversation.id.to_string(),
            "contactInboxId": contact_inbox.id.to_string(),
            "flowId": flow_version.flow_id.to_string(),
            "executedFlowVersionId": flow_version.id.to_string(),
            "step": { "id": retry["id"], "nodeId": node_id, "stepType": "sendText",
                "text": retry["message"], "buttons": [] },
        });
        if !use_latest {
            job["flowVersionId"] = json!(flow_version.id.to_string());
        }
        if let Some(quick_replies) = details
            .as_ref()
            .and_then(|d| d.get("quickReplies"))
            .filter(|q| !q.is_null())
        {
            job["quickReplies"] = quick_replies.clone();
        }
        let sent = deps
            .jobs
            .add(
                QueueName::Chat,
                crate::chat::SEND_FLOW_MESSAGE,
                json!({ "type": crate::chat::SEND_FLOW_MESSAGE, "data": job }),
                JobOptions::default(),
            )
            .await;
        if let Err(err) = sent {
            tracing::warn!(err = %err, conversation_id = conversation.id, node_id, "runQuickReplyChallenge: failed to send retry message");
            if let Err(err) = set(attempts + 1, attempts).await {
                tracing::warn!(err = %err, conversation_id = conversation.id, node_id, "runQuickReplyChallenge: failed to roll back retry attempt");
            }
        }
        return Ok(());
    }
    if !clear_node(deps, conversation, &node_id, Some(attempts)).await? {
        return Ok(());
    }
    let retry = retry.clone();
    route_target(
        deps,
        conversation.clone(),
        contact_inbox.clone(),
        flow_version,
        use_latest,
        &node_id,
        &retry,
        None,
    )
    .await
}

/// `resumeQuickReplyFollowUp`: the claimed follow-up row runs its target.
pub(crate) async fn resume_follow_up(
    deps: &Deps,
    row: &SmartDelayRow,
    conversation: Conversation,
    contact_inbox: ContactInbox,
) -> Result<(), JobError> {
    let node_id = row.node_id.clone().unwrap_or_default();
    let Some((flow_version, use_latest)) =
        crate::flow::find_flow_version(deps, row.flow_id, row.flow_version_id, row.workspace_id)
            .await?
    else {
        tracing::warn!(
            smart_delay_id = row.id,
            node_id,
            "Quick reply follow-up skipped: flow version unavailable"
        );
        return Ok(());
    };
    let active = resolve_active_quick_reply_settings(node_details(&flow_version, &node_id));
    let Some(follow_up) = active.get("followUp").cloned() else {
        tracing::warn!(
            smart_delay_id = row.id,
            node_id,
            "Quick reply follow-up skipped: follow-up no longer configured"
        );
        return Ok(());
    };
    clear_node(deps, &conversation, &node_id, None).await?;
    let metadata = row
        .metadata
        .clone()
        .filter(|m| m.get("type").is_some_and(Value::is_string));
    route_target(
        deps,
        conversation,
        contact_inbox,
        flow_version,
        use_latest,
        &node_id,
        &follow_up,
        metadata,
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::steps::test_support::*;

    async fn challenge(fx: &Fixture) -> serde_json::Value {
        fx.text(r#"SELECT json_extract("additionalAttributes", '$.challenge') FROM "Conversation" WHERE "id" = 6"#)
            .await
            .map(|c| serde_json::from_str(&c).unwrap())
            .unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn a_typed_reply_re_asks_then_routes_the_retry_target() {
        let fx = Fixture::new().await;
        fx.exec(r#"UPDATE "Conversation" SET "botEnabled" = 1 WHERE "id" = 6"#)
            .await;
        fx.flow(
            json!([
                { "id": "n1", "data": { "isStartNode": true, "details": {
                    "steps": [{ "id": "s1", "stepType": "sendText", "text": "Pick", "buttons": [] }],
                    "quickReplies": [{ "id": "q1", "label": "A", "buttonType": null, "beforeStep": null, "steps": [] }],
                    "quickReplySettings": { "retry": { "id": "r1", "enabled": true, "message": "Pick one",
                        "maxRetries": 1, "target": { "buttonType": "startAnotherNode",
                            "beforeStep": { "id": "b1", "stepType": "startAnotherNode" } } } },
                } } },
                { "id": "n2", "data": { "details": { "steps": [] } } },
            ]),
            json!([{ "id": "e1", "source": "n1", "sourceHandle": "r1", "target": "n2" }]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        let armed = challenge(&fx).await;
        assert_eq!(armed["type"], "quickReply");
        assert_eq!(armed["data"]["attempts"], 0);

        let reply = || json!({ "conversationId": "6", "contactInboxId": "5" });
        let mut data = reply();
        data["challenge"] = armed;
        crate::challenge::run_challenge(&fx.deps, &data)
            .await
            .unwrap();
        assert_eq!(challenge(&fx).await["data"]["attempts"], 1);
        let retry = fx.jobs.named("sendFlowMessage").pop().unwrap();
        assert_eq!(retry.data["data"]["step"]["text"], "Pick one");
        assert_eq!(retry.data["data"]["quickReplies"][0]["id"], "q1");

        let mut data = reply();
        data["challenge"] = challenge(&fx).await;
        crate::challenge::run_challenge(&fx.deps, &data)
            .await
            .unwrap();
        assert!(challenge(&fx).await.is_null());
        assert!(
            fx.jobs.send_flows().iter().any(|d| d["nodeId"] == "n2"),
            "{:?}",
            fx.jobs.send_flows()
        );
    }
}
