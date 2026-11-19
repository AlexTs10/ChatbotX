//! The `whatsappVoipSignaling` queue (`handlers/whatsapp-voip-signaling.ts`):
//! `handleConnect` (ring the eligible agents), `expireIfUnanswered`,
//! `handleOutboundAnswer`, `expireOutboundDial`; and the VoIP crons
//! `sweepStaleWhatsappCalls` / `purgeExpiredCallRecordings`.
use chatbotx_business::whatsapp::GRAPH_API_URL;
use chatbotx_business::whatsapp_voip::{
    self as voip, EndAction, GraphAuth, Phase, VoipStore, is_call_ended,
};
use chatbotx_core::utils::whatsapp_call_hours::is_within_call_hours;
use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::partials::whatsapp_call::WhatsappCallTerminalStatus;
use chatbotx_db::repo::whatsapp_call::{self as calls, FinalizeInput};
use chatbotx_db::repo::{integration_whatsapp, now_ms};
use chatbotx_jobs::{JobCtx, JobError};
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Value, json};

use crate::{Deps, db_fail, fail, str_of};

fn biz(error: chatbotx_business::BusinessError) -> JobError {
    fail(error.message())
}

async fn read<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::read(&deps.db, f).await.map_err(db_fail)
}

pub(crate) fn store(deps: &Deps) -> VoipStore {
    VoipStore::new(deps.db.clone())
}

async fn by_wacid(deps: &Deps, wacid: &str) -> Result<Option<WhatsappCall>, JobError> {
    let wacid = wacid.to_owned();
    read(deps, move |c| calls::find_by_wacid(c, &wacid)).await
}

/// `getOutboundCallRowOrThrow`.
async fn outbound_row(
    deps: &Deps,
    attempt_id: &str,
    wacid: Option<&str>,
) -> Result<WhatsappCall, JobError> {
    let attempt = attempt_id.to_owned();
    if let Some(row) = read(deps, move |c| calls::find_by_attempt_id(c, &attempt)).await? {
        return Ok(row);
    }
    if let Some(wacid) = wacid.filter(|w| !w.is_empty())
        && let Some(row) = by_wacid(deps, wacid).await?
    {
        return Ok(row);
    }
    Err(fail(format!(
        "whatsapp-voip-call-row-not-ready: {}",
        wacid.unwrap_or(attempt_id)
    )))
}

/// The queue handler.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    handle_with(&deps, &job, GRAPH_API_URL).await?;
    Ok(Value::Null)
}

pub async fn handle_with(deps: &Deps, job: &JobCtx, graph_url: &str) -> Result<(), JobError> {
    let data = &job.data["data"];
    match str_of(&job.data, "type") {
        Some(voip::HANDLE_CONNECT) => {
            let result = handle_connect(deps, data, graph_url).await;
            // The queue's `failed` listener (`finalizeExhaustedHandleConnect`).
            if result.is_err() && job.is_final_attempt() {
                finalize_exhausted_handle_connect(deps, data, graph_url).await;
            }
            result
        }
        Some(voip::EXPIRE_IF_UNANSWERED) => handle_expire(deps, data, graph_url).await,
        Some(voip::HANDLE_OUTBOUND_ANSWER) => handle_outbound_answer(deps, data).await,
        Some(voip::EXPIRE_OUTBOUND_DIAL) => {
            handle_expire_outbound_dial(deps, data, graph_url).await
        }
        other => {
            tracing::warn!(?other, "Unhandled whatsapp voip signaling job");
            Ok(())
        }
    }
}

/// `resolveVoipIntegration`.
async fn integration_by_phone(
    deps: &Deps,
    phone_number_id: &str,
) -> Result<chatbotx_db::repo::models::IntegrationWhatsapp, JobError> {
    let phone = phone_number_id.to_owned();
    read(deps, move |c| {
        integration_whatsapp::find_by_phone_number_id(c, &phone, None)
    })
    .await?
    .ok_or_else(|| {
        fail(format!(
            "Integration not found for whatsapp: {phone_number_id}"
        ))
    })
}

/// `resolveVoipAuthByInboxId`.
async fn auth_by_inbox(deps: &Deps, call: &WhatsappCall) -> Result<Value, JobError> {
    let (ws, inbox) = (call.workspace_id, call.inbox_id);
    read(deps, move |c| {
        integration_whatsapp::find_auth_by_inbox_id_for_workspace(c, ws, inbox)
    })
    .await?
    .map(|auth| auth.0)
    .ok_or_else(|| {
        fail(format!(
            "Whatsapp VoIP: no IntegrationWhatsapp row for inboxId {inbox}"
        ))
    })
}

/// `inboundCallRefusal`.
pub fn inbound_call_refusal(
    integration: &chatbotx_db::repo::models::IntegrationWhatsapp,
    at: i64,
) -> Option<&'static str> {
    if integration.calling_enabled != Some(true) {
        return Some("callingDisabled");
    }
    if !integration.inbound_calls_enabled {
        return Some("inboundMuted");
    }
    let hours = integration
        .call_hours
        .as_ref()
        .map(|h| &h.0)
        .filter(|h| h.is_object());
    (!is_within_call_hours(hours, at)).then_some("outsideCallHours")
}

/// `finalizeEndedCall` (signalling side): the call card via
/// `finalizeCallSideEffects`.
async fn finalize_ended(deps: &Deps, wacid: &str, status: &str) -> Result<(), JobError> {
    let Some(call) = by_wacid(deps, wacid).await? else {
        tracing::warn!(
            wacid,
            status,
            "Whatsapp VoIP: no call row to finalize on end"
        );
        return Ok(());
    };
    let entity = json!({ "type": "whatsapp_call", "direction": call.direction, "status": status });
    crate::whatsapp_call::finalize_card(deps, call, entity).await
}

/// `graphEndThenFinalize`.
async fn graph_end_then_finalize(
    deps: &Deps,
    graph_url: &str,
    auth: &Value,
    wacid: &str,
    action: EndAction,
    status: &str,
) -> Result<(), JobError> {
    let graph = GraphAuth {
        base_url: graph_url,
        auth,
    };
    if let Err(error) = voip::end_call_at_meta(&graph, wacid, action).await {
        tracing::warn!(
            wacid,
            action = action.as_str(),
            status = error.status,
            "Whatsapp VoIP: Graph end action failed"
        );
    }
    finalize_ended(deps, wacid, status).await
}

/// `endReservedCall`: `true` when this call ended it.
pub async fn end_reserved_call(
    deps: &Deps,
    graph_url: &str,
    auth: &Value,
    wacid: &str,
) -> Result<bool, JobError> {
    let Some(ended) = store(deps).end_call(wacid, false).await.map_err(biz)? else {
        return Ok(false);
    };
    let status = match ended.terminal_status {
        WhatsappCallTerminalStatus::Rejected => "rejected",
        WhatsappCallTerminalStatus::Failed => "failed",
        WhatsappCallTerminalStatus::Completed => {
            tracing::warn!(
                wacid,
                "Whatsapp VoIP: unexpected terminalStatus from endCall on the expiry/reject path"
            );
            return Ok(false);
        }
    };
    graph_end_then_finalize(deps, graph_url, auth, wacid, ended.graph_action, status).await?;
    Ok(true)
}

/// `refuseIncomingCall`.
async fn refuse_incoming_call(
    deps: &Deps,
    graph_url: &str,
    auth: &Value,
    wacid: &str,
    deadline_at: i64,
) -> Result<(), JobError> {
    if store(deps)
        .claim_unreachable(wacid, deadline_at)
        .await
        .map_err(biz)?
    {
        return graph_end_then_finalize(
            deps,
            graph_url,
            auth,
            wacid,
            EndAction::Reject,
            "rejected",
        )
        .await;
    }
    end_reserved_call(deps, graph_url, auth, wacid)
        .await
        .map(|_| ())
}

/// `finalizeExhaustedHandleConnect` (never fails).
async fn finalize_exhausted_handle_connect(deps: &Deps, data: &Value, graph_url: &str) {
    let wacid = str_of(data, "wacid").unwrap_or_default();
    let run = async {
        let integration =
            integration_by_phone(deps, str_of(data, "phoneNumberId").unwrap_or_default()).await?;
        let deadline = data["deadlineAt"].as_i64().unwrap_or_else(now_ms);
        refuse_incoming_call(deps, graph_url, &integration.auth.0, wacid, deadline).await
    };
    if let Err(error) = run.await {
        tracing::error!(%error, wacid, "Whatsapp VoIP: unable to finalize a call whose handleConnect retries were exhausted; the 5-minute stale-call sweep remains the backstop");
    }
}

/// Publishes to each member; the failure count.
fn publish_to_members(
    deps: &Deps,
    workspace_id: i64,
    targets: &[i64],
    event: &Envelope,
    message: &str,
) -> usize {
    let Some(realtime) = deps.realtime.as_ref() else {
        return targets.len();
    };
    let ws = workspace_id.to_string();
    targets
        .iter()
        .filter(|user| {
            let failed = realtime
                .publish_member_event(&ws, &user.to_string(), event.clone())
                .is_err();
            if failed {
                tracing::warn!(user_id = **user, workspace_id, "{message}");
            }
            failed
        })
        .count()
}

/// The online members of the workspace (`listOnlineMembers`).
fn online_members(deps: &Deps, workspace_id: i64) -> Vec<i64> {
    deps.realtime
        .as_ref()
        .map(|r| r.online_members(&workspace_id.to_string()))
        .unwrap_or_default()
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect()
}

async fn handle_connect(deps: &Deps, data: &Value, graph_url: &str) -> Result<(), JobError> {
    let wacid = str_of(data, "wacid").unwrap_or_default().to_owned();
    let deadline_at = data["deadlineAt"].as_i64().unwrap_or_else(now_ms);
    let received_at = data["receivedAt"].as_i64().unwrap_or_else(now_ms);
    let store = store(deps);

    let existing = by_wacid(deps, &wacid).await?;
    if let Some(call) = existing.as_ref().filter(|c| is_call_ended(c)) {
        store.delete_offer(&wacid).await.map_err(biz)?;
        tracing::info!(
            wacid,
            status = call.status,
            "Whatsapp VoIP: connect for a call that already ended; not ringing"
        );
        return Ok(());
    }
    if let Some(control) = store.read_control(&wacid).await.map_err(biz)?
        && control.phase != Phase::Reserved
    {
        tracing::info!(wacid, phase = ?control.phase, "Whatsapp VoIP: connect for a call that is no longer ringing; ignoring");
        return Ok(());
    }
    let integration =
        integration_by_phone(deps, str_of(data, "phoneNumberId").unwrap_or_default()).await?;
    let (workspace_id, auth) = (integration.workspace_id, integration.auth.0.clone());
    if let Some(refusal) = inbound_call_refusal(&integration, received_at) {
        tracing::info!(
            wacid,
            workspace_id,
            refusal,
            "Whatsapp VoIP: inbound call refused by this number's calling settings"
        );
        return refuse_incoming_call(deps, graph_url, &auth, &wacid, deadline_at).await;
    }
    let Some(offer) = store.read_offer(&wacid).await.map_err(biz)? else {
        return refuse_incoming_call(deps, graph_url, &auth, &wacid, deadline_at).await;
    };
    if !store
        .reserve_incoming(&wacid, deadline_at)
        .await
        .map_err(biz)?
    {
        return Ok(());
    }
    let conversation_id = match &existing {
        Some(call) => call.conversation_id,
        None => row_or_throw(deps, &wacid).await?.conversation_id,
    };
    let targets = voip::select_ring_targets_for_call(
        &deps.db,
        workspace_id,
        conversation_id,
        online_members(deps, workspace_id),
    )
    .await
    .map_err(biz)?;
    if targets.is_empty() {
        end_reserved_call(deps, graph_url, &auth, &wacid).await?;
        return Ok(());
    }
    let call = row_or_throw(deps, &wacid).await?;
    if is_call_ended(&call) {
        store.end_call(&wacid, false).await.map_err(biz)?;
        store.delete_offer(&wacid).await.map_err(biz)?;
        return Ok(());
    }
    let event = Envelope::new(
        EventType::WhatsappCallTransportIncoming,
        json!({
            "transport": "voip",
            "whatsappCallId": call.id.to_string(),
            "wacid": wacid,
            "direction": call.direction,
            "conversationId": call.conversation_id.to_string(),
            "contactInboxId": call.contact_inbox_id.to_string(),
            "contactName": voip::caller_name(&deps.db, &call).await,
            "offer": { "sdpType": "offer", "sdp": offer },
            "deadlineAt": chatbotx_db::repo::iso(deadline_at),
        }),
    );
    let failures = publish_to_members(
        deps,
        workspace_id,
        &targets,
        &event,
        "Whatsapp VoIP: unable to publish the offer realtime event",
    );
    if failures == targets.len() {
        tracing::error!(
            target_count = targets.len(),
            wacid,
            "Whatsapp VoIP: unable to publish the offer realtime event to any target"
        );
        return Err(fail(
            "Whatsapp VoIP: unable to publish the offer realtime event to any target",
        ));
    }
    // `notifyRungAgentsIfEnded`: a hangup may have raced the ring.
    if let Some(latest) = by_wacid(deps, &wacid).await?.filter(is_call_ended) {
        let ended = ended_event(&latest, &wacid);
        publish_to_members(
            deps,
            workspace_id,
            &targets,
            &ended,
            "Whatsapp VoIP: unable to publish the ended event to a previously rung agent",
        );
    }
    Ok(())
}

fn ended_event(call: &WhatsappCall, wacid: &str) -> Envelope {
    Envelope::new(
        EventType::WhatsappCallTransportEnded,
        json!({ "transport": "voip", "whatsappCallId": call.id.to_string(), "wacid": wacid, "status": call.status }),
    )
}

/// `getCallRowOrThrow`.
async fn row_or_throw(deps: &Deps, wacid: &str) -> Result<WhatsappCall, JobError> {
    by_wacid(deps, wacid)
        .await?
        .ok_or_else(|| fail(format!("whatsapp-voip-call-row-not-ready: {wacid}")))
}

/// `handleExpire`.
async fn handle_expire(deps: &Deps, data: &Value, graph_url: &str) -> Result<(), JobError> {
    let wacid = str_of(data, "wacid").unwrap_or_default();
    let control = store(deps).read_control(wacid).await.map_err(biz)?;
    if control.is_none_or(|c| matches!(c.phase, Phase::Accepted | Phase::Terminated)) {
        return Ok(());
    }
    let integration =
        integration_by_phone(deps, str_of(data, "phoneNumberId").unwrap_or_default()).await?;
    end_reserved_call(deps, graph_url, &integration.auth.0, wacid)
        .await
        .map(|_| ())
}

/// `handleOutboundAnswer`: the answer SDP to the initiating agent only.
async fn handle_outbound_answer(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let attempt_id = str_of(data, "attemptId").unwrap_or_default();
    let wacid = str_of(data, "wacid");
    let call = outbound_row(deps, attempt_id, wacid).await?;
    let Some(sdp) = store(deps)
        .read_outbound_answer(attempt_id)
        .await
        .map_err(biz)?
    else {
        tracing::warn!(
            attempt_id,
            whatsapp_call_id = call.id,
            "Whatsapp VoIP outbound answer: no stored answer found (expired or never stored)"
        );
        return Ok(());
    };
    let Some(initiator) = call.answered_by_user_id else {
        tracing::warn!(
            attempt_id,
            whatsapp_call_id = call.id,
            "Whatsapp VoIP outbound answer: call row has no initiator; dropping"
        );
        return Ok(());
    };
    let event = Envelope::new(
        EventType::WhatsappCallOutboundAnswer,
        json!({
            "whatsappCallId": call.id.to_string(),
            "wacid": call.wacid.clone().or(wacid.map(str::to_owned)).unwrap_or_default(),
            "attemptId": attempt_id,
            "session": { "sdpType": "answer", "sdp": sdp },
        }),
    );
    let workspace = str_of(data, "workspaceId")
        .map(str::to_owned)
        .unwrap_or_else(|| call.workspace_id.to_string());
    if let Some(realtime) = deps.realtime.as_ref()
        && let Err(error) = realtime.publish_member_event(&workspace, &initiator.to_string(), event)
    {
        tracing::warn!(attempt_id, %error, user_id = initiator, "Whatsapp VoIP: unable to publish the outbound answer realtime event");
        return Err(fail(error.to_string()));
    }
    Ok(())
}

/// `handleExpireOutboundDial` (+ `forceEndNoAnswerOutboundDial`).
async fn handle_expire_outbound_dial(
    deps: &Deps,
    data: &Value,
    graph_url: &str,
) -> Result<(), JobError> {
    let attempt_id = str_of(data, "attemptId").unwrap_or_default();
    let call = outbound_row(deps, attempt_id, str_of(data, "wacid")).await?;
    let Some(wacid) = call
        .wacid
        .clone()
        .or_else(|| str_of(data, "wacid").map(str::to_owned))
        .filter(|w| !w.is_empty())
    else {
        tracing::warn!(
            attempt_id,
            whatsapp_call_id = call.id,
            "Whatsapp VoIP outbound expiry: call row has no wacid yet; cannot terminate"
        );
        return Ok(());
    };
    let auth = auth_by_inbox(deps, &call).await?;
    if end_reserved_call(deps, graph_url, &auth, &wacid).await? {
        return Ok(());
    }
    if by_wacid(deps, &wacid)
        .await?
        .is_some_and(|c| c.status == "ringing")
    {
        graph_end_then_finalize(
            deps,
            graph_url,
            &auth,
            &wacid,
            EndAction::Terminate,
            "failed",
        )
        .await?;
    }
    Ok(())
}

// ---- VoIP hooks of the call event worker -----------------------------------------

/// `emitCallEndedToAgent`: ends the control, drops the offer and tells the
/// agent (or every rung agent) the call is over.
pub(crate) async fn emit_call_ended_to_agent(
    deps: &Deps,
    call: &WhatsappCall,
    status: &str,
) -> Result<(), JobError> {
    let Some(wacid) = call.wacid.as_deref().filter(|w| !w.is_empty()) else {
        return Ok(());
    };
    let store = store(deps);
    let control = store.read_control(wacid).await.map_err(biz)?;
    store.end_call(wacid, true).await.map_err(biz)?;
    store.delete_offer(wacid).await.map_err(biz)?;
    let (Some(control), Some(realtime)) = (control, deps.realtime.as_ref()) else {
        return Ok(());
    };
    let event = Envelope::new(
        EventType::WhatsappCallTransportEnded,
        json!({ "transport": "voip", "whatsappCallId": call.id.to_string(), "wacid": wacid, "status": status }),
    );
    let ws = call.workspace_id.to_string();
    if control.reserved_user_id.is_empty() {
        if let Err(error) = realtime.publish_workspace_event(&ws, event).await {
            tracing::warn!(%error, call_id = call.id, "Whatsapp call: unable to emit whatsappCallTransportEnded");
        }
    } else if let Err(error) = realtime.publish_member_event(&ws, &control.reserved_user_id, event)
    {
        tracing::warn!(%error, call_id = call.id, "Whatsapp call: unable to emit whatsappCallTransportEnded");
    }
    Ok(())
}

/// `notifyOutboundStatus` + the control move of `handleOutboundInterimStatus`.
pub(crate) async fn outbound_status_hook(
    deps: &Deps,
    call: &WhatsappCall,
    status: &str,
) -> Result<(), JobError> {
    let wacid = call.wacid.clone().unwrap_or_default();
    let store = store(deps);
    if status == "ringing" {
        store.mark_outbound_ringing(&wacid).await.map_err(biz)?;
    } else {
        store.mark_outbound_accepted(&wacid).await.map_err(biz)?;
    }
    let (Some(agent), Some(realtime)) = (call.answered_by_user_id, deps.realtime.as_ref()) else {
        return Ok(());
    };
    let event = Envelope::new(
        EventType::WhatsappCallOutboundStatus,
        json!({
            "whatsappCallId": call.id.to_string(),
            "wacid": wacid,
            "attemptId": call.attempt_id.clone().unwrap_or_default(),
            "status": status,
        }),
    );
    if let Err(error) =
        realtime.publish_member_event(&call.workspace_id.to_string(), &agent.to_string(), event)
    {
        tracing::warn!(%error, whatsapp_call_id = call.id, status, "Whatsapp VoIP: outbound status realtime send threw unexpectedly");
    }
    Ok(())
}

// ---- crons --------------------------------------------------------------------------

const STALE_RINGING_THRESHOLD_MS: i64 = 90_000;
const STALE_RINGING_SWEEP_LIMIT: i64 = 200;
const PURGE_BATCH_SIZE: i64 = 500;

/// `sweepStaleWhatsappCalls`.
pub async fn sweep_stale_whatsapp_calls(deps: &Deps) -> Result<(), JobError> {
    sweep_stale_with(deps, GRAPH_API_URL).await
}

pub async fn sweep_stale_with(deps: &Deps, graph_url: &str) -> Result<(), JobError> {
    let older_than = now_ms() - STALE_RINGING_THRESHOLD_MS;
    let stale = read(deps, move |c| {
        calls::sweep_stale_ringing(c, older_than, STALE_RINGING_SWEEP_LIMIT)
    })
    .await?;
    let mut finalized = 0;
    for call in stale {
        if let Some(wacid) = call.wacid.clone() {
            let ended = async {
                let auth = auth_by_inbox(deps, &call).await?;
                end_reserved_call(deps, graph_url, &auth, &wacid).await
            };
            match ended.await {
                Ok(true) => {
                    finalized += 1;
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%error, call_id = call.id, wacid, "Whatsapp call stale sweep: unable to resolve auth/end the call via its control record; falling back to a direct finalize");
                }
            }
        }
        let input = FinalizeInput {
            id: call.id,
            status: "failed".into(),
            outcome: "failed".into(),
            ended_at: Some(Some(now_ms())),
            last_error: Some(Some("stale-ringing-never-finalized".into())),
            current: Some(call.clone()),
            ..FinalizeInput::default()
        };
        let updated = chatbotx_db::repo::write(&deps.db, move |c| calls::finalize_by_id(c, &input))
            .await
            .map_err(db_fail)?;
        if updated.is_some() {
            finalized += 1;
        }
    }
    if finalized > 0 {
        tracing::info!(finalized, "Finalized stale ringing WhatsApp calls");
    }
    Ok(())
}

/// `purgeExpiredCallRecordings`: the object (best effort), then the columns.
pub async fn purge_expired_call_recordings(deps: &Deps) -> Result<(), JobError> {
    let mut total = 0;
    loop {
        let now = now_ms();
        let expired = read(deps, move |c| {
            calls::list_recordings_past_retention(c, PURGE_BATCH_SIZE, now)
        })
        .await?;
        let mut purged = 0;
        for call in expired {
            let Some(path) = call.recording_path.clone() else {
                continue;
            };
            if let Some(files) = deps.files.as_ref() {
                let _ = files.delete_object(&path).await;
            }
            let id = call.id;
            chatbotx_db::repo::write(&deps.db, move |c| calls::clear_recording(c, id))
                .await
                .map_err(db_fail)?;
            purged += 1;
        }
        total += purged;
        if purged < PURGE_BATCH_SIZE {
            break;
        }
    }
    if total > 0 {
        tracing::info!(
            total_purged = total,
            "Purged expired WhatsApp call recordings"
        );
    }
    Ok(())
}
