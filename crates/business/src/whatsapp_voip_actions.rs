//! The calling server actions of `features/integration-whatsapp/calling/actions/*`
//! (answer, outbound dial, hangup, TURN, outbound call mode) and
//! `lib/calling-operations.ts` (calling settings, call hours, the `calls`
//! subscription fix). The caller (API) ran the member / role gates.
use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{ContactInbox, IntegrationWhatsapp, WhatsappCall};
use chatbotx_db::repo::partials::whatsapp_call::WhatsappCallTerminalStatus;
use chatbotx_db::repo::whatsapp_call::{self as calls, PendingOutboundInput};
use chatbotx_db::repo::{integration_whatsapp, iso, now_ms, random_uuid};
use chatbotx_jobs::Enqueuer;
use chatbotx_rt::protocol::{AssignmentRoute, route_for_assignment};
use chatbotx_rt::{Envelope, EventType, Realtime};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;
use crate::whatsapp_voip::{
    self as voip, Announcements, EndAction, GraphAuth, GraphError, VoipStore,
};

/// `whatsapp.calls.errors.*` (en).
pub mod messages {
    pub const NOT_FOUND: &str = "WhatsApp integration not found";
    pub const NOT_WHATSAPP_CONVERSATION: &str = "This conversation is not on WhatsApp";
    pub const CALL_NOT_FOUND: &str = "Call not found";
    pub const VOIP_ANSWER_FAILED: &str = "Could not answer the WhatsApp call";
    pub const VOIP_NOT_RESERVED_AGENT: &str = "You are not the agent assigned to this call";
    pub const VOIP_CALL_ACCESS_DENIED: &str = "You don't have permission to handle this call";
    pub const VOIP_TURN_RATE_LIMITED: &str =
        "Too many call setup requests. Please wait a moment and try again.";
    pub const UPDATE_FAILED: &str = "Meta rejected the calling settings change";
    pub const SAVED_ON_META_ONLY: &str =
        "Meta saved the change but this workspace did not. Save again to finish.";
    pub const TRANSCRIPTION_REQUIRES_RECORDING: &str =
        "Turn on call recording before turning on transcription.";
    pub const CALLING_ACCESS_DENIED: &str = "You don't have access to calling in this workspace.";
    pub const CALL_HISTORY_ACCESS_DENIED: &str =
        "You don't have access to call history in this workspace.";
    pub const CALL_ARTIFACT_ACCESS_DENIED: &str = "You don't have permission to view this call.";
    pub const PERMISSION_REQUEST_LIMIT_REACHED: &str = "WhatsApp allows only 2 call permission requests per contact every 7 days. This contact has reached that limit \u{2014} try again later.";
    pub const PERMISSION_CHECK_FAILED: &str = "We could not check your call permission for this contact right now. Please try again in a moment.";
    pub const MANUAL_INTEGRATION_NOTICE: &str = "This number was connected manually and has no app credentials here. Subscribe the 'calls' webhook field for your app in the Meta App Dashboard.";
    pub const APP_SUBSCRIPTION_MISSING: &str = "Configure the WhatsApp webhook for this app in the Meta App Dashboard first, then try again.";
    pub const APP_SETTINGS_NOT_FOUND: &str = "WhatsApp app settings were not found.";
    pub const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";
}

/// The `public` (English) messages of `lib/calling-operations.ts`
/// (`ENGLISH_CALLING_MESSAGES`), or the builder's translations.
#[derive(Debug, Clone, Copy)]
pub struct CallingMessages {
    pub not_found: &'static str,
    pub transcription_requires_recording: &'static str,
    pub update_failed: &'static str,
    pub saved_on_meta_only: &'static str,
}

/// `ENGLISH_CALLING_MESSAGES` (the public API).
pub const ENGLISH_CALLING_MESSAGES: CallingMessages = CallingMessages {
    not_found: "WhatsApp channel not found",
    transcription_requires_recording: "Turn on call recording before turning on transcription.",
    update_failed: "Meta did not accept the calling settings.",
    saved_on_meta_only: "Meta accepted the change but it could not be saved here. Send the same request again to fix it.",
};

/// The builder's `whatsapp.calls.errors.*` for the same operations.
pub const BUILDER_CALLING_MESSAGES: CallingMessages = CallingMessages {
    not_found: messages::NOT_FOUND,
    transcription_requires_recording: messages::TRANSCRIPTION_REQUIRES_RECORDING,
    update_failed: messages::UPDATE_FAILED,
    saved_on_meta_only: messages::SAVED_ON_META_ONLY,
};

/// `WHATSAPP_CALLING_UPSTREAM_CODE`.
pub const WHATSAPP_CALLING_UPSTREAM_CODE: &str = "whatsappCallingUpstream";
/// `CALL_ACCESS_DENIED_CODE`.
pub const CALL_ACCESS_DENIED_CODE: &str = "callAccessDenied";

/// `new ChatbotXException(message)`.
pub fn system_error(message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

/// The services of a calling action.
#[derive(Clone, Copy)]
pub struct Calling<'a> {
    pub db: &'a Arc<Db>,
    pub graph_url: &'a str,
    pub jobs: Option<&'a Arc<dyn Enqueuer>>,
    pub realtime: Option<&'a Realtime>,
}

impl Calling<'_> {
    fn store(&self) -> VoipStore {
        VoipStore::new(self.db.clone())
    }
}

async fn call_for_workspace(db: &Db, ws: i64, id: i64) -> Result<Option<WhatsappCall>> {
    tx::read(db, move |c| Ok(calls::find_by_id_for_workspace(c, id, ws)?)).await
}

async fn integration_by_inbox(db: &Db, ws: i64, inbox: i64) -> Result<Option<IntegrationWhatsapp>> {
    tx::read(db, move |c| {
        Ok(integration_whatsapp::find_by_inbox_id_for_workspace(
            c, ws, inbox,
        )?)
    })
    .await
}

/// `resolveContactLocale`: the contact inbox language, else the contact locale.
async fn contact_locale(db: &Db, contact_inbox_id: i64) -> Result<Option<String>> {
    tx::read(db, move |c| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        Ok(c.query_row(
            r#"SELECT COALESCE(ci."language", ct."locale") FROM "ContactInbox" ci
               LEFT JOIN "Contact" ct ON ct."id" = ci."contactId" WHERE ci."id" = ?1"#,
            [contact_inbox_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
    })
    .await
}

/// `resolveContactInbox`: the contact's WhatsApp contact inbox (the given one,
/// else the first).
async fn whatsapp_contact_inbox(
    db: &Db,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
) -> Result<Option<ContactInbox>> {
    tx::read(db, move |c| {
        let sql = format!(
            r#"SELECT {} FROM "ContactInbox" WHERE "contactId" = ?1 AND "channel" = 'whatsapp'
               AND (?2 IS NULL OR "id" = ?2) ORDER BY "id" LIMIT 1"#,
            ContactInbox::columns_sql(None)
        );
        Ok(
            chatbotx_db::repo::query_models(c, &sql, &[&contact_id, &contact_inbox_id])?
                .into_iter()
                .next(),
        )
    })
    .await
}

fn browser_recording(integration: &IntegrationWhatsapp) -> bool {
    integration.call_recording_enabled && integration.call_recording_mode == "browserWhisper"
}

/// `claimConversationForCallAgent`: assigns the conversation to the agent
/// while unassigned (best effort; a support session never claims).
///
/// ponytail: like `assignOne`, the response-SLA window, the
/// `conversation_assigned` flow trigger (`whatsappCallAnswered` /
/// `whatsappCallDialed`) and the analytics attribution are not ported.
pub async fn claim_conversation_for_call_agent(
    c: Calling<'_>,
    workspace_id: i64,
    conversation_id: i64,
    user_id: i64,
) {
    let claimed = tx::write(c.db, move |conn| {
        Ok(chatbotx_db::repo::conversation::assign_user_if_unassigned(
            conn,
            workspace_id,
            conversation_id,
            user_id,
            None,
        )?)
    })
    .await;
    match claimed {
        Ok(rows) if !rows.is_empty() => {
            if let Some(realtime) = c.realtime {
                let user = user_id.to_string();
                let event = Envelope::routed(
                    EventType::ConversationAssigned,
                    json!({
                        "conversationIds": [conversation_id.to_string()],
                        "assignedUserId": user,
                        "assignedInboxTeamId": null,
                    }),
                    route_for_assignment(&AssignmentRoute {
                        assigned_inbox_team_id: None,
                        assigned_user_id: Some(user.clone()),
                        previous_assigned_inbox_team_ids: vec![None],
                        previous_assigned_user_ids: vec![None],
                    }),
                );
                realtime
                    .queue_workspace_event(&workspace_id.to_string(), event)
                    .await;
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(err = %error.message(), conversation_id, "WhatsApp call: auto-assign failed");
        }
    }
}

// ---- answer -------------------------------------------------------------------------

/// `acceptCallWithAnnouncementFallback`: `(announcement applied, error)`.
async fn accept_with_fallback(
    graph: &GraphAuth<'_>,
    wacid: &str,
    sdp: &str,
    options: &Announcements,
) -> std::result::Result<bool, GraphError> {
    match voip::accept_call(graph, wacid, sdp, options).await {
        Ok(()) => Ok(!options.is_empty()),
        Err(error) if !options.is_empty() && voip::is_announcement_validation_error(&error) => {
            tracing::warn!(
                wacid,
                status = error.status,
                "WhatsApp VoIP accept: retrying without recording/transcription announcement options after a Meta 4xx"
            );
            voip::accept_call(graph, wacid, sdp, &Announcements::default()).await?;
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// `answerWhatsappVoipCallAction`.
pub async fn answer_call(
    c: Calling<'_>,
    workspace_id: i64,
    user_id: i64,
    whatsapp_call_id: i64,
    sdp_answer: &str,
) -> Result<Value> {
    let cannot = || Ok(json!({ "outcome": "cannotAnswer" }));
    let call = call_for_workspace(c.db, workspace_id, whatsapp_call_id).await?;
    let Some((call, wacid)) = call.and_then(|call| call.wacid.clone().map(|w| (call, w))) else {
        return Err(system_error(messages::CALL_NOT_FOUND));
    };
    let integration = integration_by_inbox(c.db, workspace_id, call.inbox_id)
        .await?
        .ok_or_else(|| system_error(messages::NOT_FOUND))?;
    let locale = contact_locale(c.db, call.contact_inbox_id).await?;
    let options = voip::build_announcements(&integration, locale.as_deref());
    let browser_recording = browser_recording(&integration);
    let auth = integration.auth.0.clone();
    let graph = GraphAuth {
        base_url: c.graph_url,
        auth: &auth,
    };
    let conversation_id = call.conversation_id;
    if !voip::can_call_conversation(c.db, workspace_id, conversation_id, user_id).await? {
        return cannot();
    }
    let store = c.store();
    let Some(control) = store.read_control(&wacid).await? else {
        return cannot();
    };
    if voip::is_answer_deadline_expired(control.deadline_at) {
        return cannot();
    }
    let Some(fence) = store.claim_for_answer(&wacid, &user_id.to_string()).await? else {
        return cannot();
    };
    let release = |reason: &'static str| {
        let (store, wacid, fence) = (store.clone(), wacid.clone(), fence.clone());
        async move {
            if let Err(error) = store.release_claim(&wacid, &fence).await {
                tracing::warn!(err = %error.message(), whatsapp_call_id, wacid, "{reason}");
            }
        }
    };
    if voip::is_answer_deadline_expired(control.deadline_at)
        || !voip::can_call_conversation(c.db, workspace_id, conversation_id, user_id).await?
    {
        release("WhatsApp VoIP call: best-effort releaseClaim failed").await;
        return cannot();
    }
    let accepted = async {
        voip::pre_accept_call(&graph, &wacid, sdp_answer).await?;
        if voip::is_answer_deadline_expired(control.deadline_at) {
            return Ok(None);
        }
        accept_with_fallback(&graph, &wacid, sdp_answer, &options)
            .await
            .map(Some)
    }
    .await;
    let announcement_applied = match accepted {
        Ok(Some(applied)) => applied,
        Ok(None) => {
            release("WhatsApp VoIP call: best-effort releaseClaim failed").await;
            return cannot();
        }
        Err(error) => {
            if voip::is_answer_deadline_expired(control.deadline_at) {
                tracing::warn!(
                    whatsapp_call_id,
                    wacid,
                    status = error.status,
                    "WhatsApp VoIP call accept failed after the answer deadline"
                );
                release("WhatsApp VoIP call: best-effort releaseClaim failed").await;
                return cannot();
            }
            tracing::error!(
                whatsapp_call_id,
                wacid,
                status = error.status,
                message = error.message,
                "WhatsApp VoIP call accept failed"
            );
            release("WhatsApp VoIP call: releaseClaim after accept failure failed").await;
            return Err(system_error(messages::VOIP_ANSWER_FAILED));
        }
    };
    let compensate = || async {
        if let Err(error) = voip::end_call_at_meta(&graph, &wacid, EndAction::Terminate).await {
            tracing::error!(
                whatsapp_call_id,
                wacid,
                status = error.status,
                "WhatsApp VoIP compensating terminate failed"
            );
        }
        Ok(json!({ "outcome": "callEnded" }))
    };
    if !store.commit_accepted(&wacid, &fence).await? {
        return compensate().await;
    }
    let accepted_row = tx::write(c.db, move |conn| {
        Ok(calls::mark_accepted_if_active(
            conn,
            whatsapp_call_id,
            user_id,
        )?)
    })
    .await?;
    if accepted_row.is_none() {
        return compensate().await;
    }
    if let Some(realtime) = c.realtime {
        let event = Envelope::new(
            EventType::WhatsappCallClaimedElsewhere,
            json!({ "whatsappCallId": whatsapp_call_id.to_string(), "wacid": wacid, "answeredByUserId": user_id.to_string() }),
        );
        if let Err(error) = realtime
            .publish_workspace_event(&workspace_id.to_string(), event)
            .await
        {
            tracing::warn!(%error, whatsapp_call_id, wacid, "WhatsApp VoIP call: whatsappCallClaimedElsewhere broadcast failed");
        }
    }
    let recording_requested =
        (announcement_applied && options.recording.is_some()) || browser_recording;
    voip::record_recording_arrangement(
        c.db,
        whatsapp_call_id,
        recording_requested,
        options.recording.is_some(),
    )
    .await;
    claim_conversation_for_call_agent(c, workspace_id, conversation_id, user_id).await;
    Ok(json!({
        "outcome": "accepted",
        "browserRecordingEnabled": browser_recording,
        "recordingRequested": recording_requested,
    }))
}

// ---- outbound dial -------------------------------------------------------------------

/// `abandonOutboundDial`.
async fn abandon_outbound_dial(
    store: &VoipStore,
    graph: &GraphAuth<'_>,
    wacid: &str,
    attempt_id: &str,
) {
    if let Err(error) = store.end_call(wacid, true).await {
        tracing::error!(err = %error.message(), wacid, attempt_id, "WhatsApp outbound dial: failed to end the local call control");
    }
    if let Err(error) = voip::end_call_at_meta(graph, wacid, EndAction::Terminate).await {
        tracing::error!(
            status = error.status,
            wacid,
            attempt_id,
            "WhatsApp outbound dial: best-effort terminate at Meta failed"
        );
    }
}

fn outcome(name: &str) -> Value {
    json!({ "outcome": name })
}

/// `initiateOutboundVoipCallAction`.
pub async fn initiate_outbound_call(
    c: Calling<'_>,
    workspace_id: i64,
    user_id: i64,
    conversation_id: i64,
    contact_inbox_id: Option<i64>,
    sdp_offer: &str,
) -> Result<Value> {
    let conversation = tx::read(c.db, move |conn| {
        crate::conversation_list::find_in(conn, workspace_id, conversation_id)
    })
    .await?
    .ok_or_else(|| system_error(messages::CALL_NOT_FOUND))?;
    if !voip::can_call_conversation(c.db, workspace_id, conversation_id, user_id).await? {
        return Ok(outcome("callAccessDenied"));
    }
    let contact_inbox = whatsapp_contact_inbox(c.db, conversation.contact_id, contact_inbox_id)
        .await?
        .ok_or_else(|| system_error(messages::NOT_WHATSAPP_CONVERSATION))?;
    let integration = integration_by_inbox(c.db, workspace_id, contact_inbox.inbox_id)
        .await?
        .ok_or_else(|| system_error(messages::NOT_FOUND))?;
    if voip::is_blocked_business_calling_country(&integration.display_phone_number) {
        return Ok(outcome("ineligibleNumber"));
    }
    let (dial_field, dial_value, permission_field) = voip::dial_identity(&contact_inbox);
    let auth = integration.auth.0.clone();
    let graph = GraphAuth {
        base_url: c.graph_url,
        auth: &auth,
    };
    let locale = match contact_inbox.language.clone() {
        Some(language) => Some(language),
        None => contact_locale(c.db, contact_inbox.id).await?,
    };
    let options = voip::build_announcements(&integration, locale.as_deref());
    let permissions = match voip::get_call_permissions(&graph, permission_field, &dial_value).await
    {
        Ok(permissions) => permissions,
        Err(error) => {
            tracing::error!(
                status = error.status,
                integration_id = integration.id,
                "Failed to read WhatsApp call permissions before outbound dial"
            );
            return Ok(outcome("permissionCheckFailed"));
        }
    };
    if !voip::can_perform_call_action(&permissions, "start_call") {
        return Ok(outcome("needsPermission"));
    }
    let store = c.store();
    if !voip::no_active_call_for_contact(c.db, &store, contact_inbox.inbox_id, contact_inbox.id)
        .await?
    {
        return Ok(outcome("callAlreadyInProgress"));
    }
    let attempt_id = random_uuid();
    let pending = PendingOutboundInput {
        attempt_id: attempt_id.clone(),
        workspace_id,
        inbox_id: contact_inbox.inbox_id,
        contact_inbox_id: contact_inbox.id,
        conversation_id: conversation.id,
        answered_by_user_id: Some(user_id),
        initiated_by_user_id: Some(user_id),
    };
    let pending = match tx::write(c.db, move |conn| {
        Ok(calls::create_pending_outbound(conn, &pending)?)
    })
    .await
    {
        Ok(row) => row,
        Err(BusinessError::Repo(chatbotx_db::repo::Error::Coded {
            kind: "WhatsappCallPendingOutboundExistsError",
            ..
        })) => {
            return Ok(outcome("callAlreadyInProgress"));
        }
        Err(error) => return Err(error),
    };
    let connected = match voip::connect_call(
        &graph,
        sdp_offer,
        &attempt_id,
        (dial_field, &dial_value),
        &options,
    )
    .await
    {
        Ok(wacid) => Ok((wacid, !options.is_empty())),
        Err(error) if !options.is_empty() && voip::is_announcement_validation_error(&error) => {
            tracing::warn!(
                attempt_id,
                status = error.status,
                "WhatsApp outbound connect: retrying without recording/transcription announcement options after a Meta 4xx"
            );
            voip::connect_call(
                &graph,
                sdp_offer,
                &attempt_id,
                (dial_field, &dial_value),
                &Announcements::default(),
            )
            .await
            .map(|wacid| (wacid, false))
        }
        Err(error) => Err(error),
    };
    let (wacid, announcement_applied) = match connected {
        Ok(connected) => connected,
        Err(error) => {
            let code = match &error.code {
                Value::Null => "callFailed".to_owned(),
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            if let Err(finalize) = voip::finalize_row(
                c.db,
                pending.id,
                WhatsappCallTerminalStatus::Failed,
                false,
                Some(code),
            )
            .await
            {
                tracing::error!(err = %finalize.message(), whatsapp_call_id = pending.id, attempt_id, "WhatsApp outbound dial: failed to finalize pending row after connect failure");
            }
            tracing::error!(
                whatsapp_call_id = pending.id,
                attempt_id,
                status = error.status,
                "WhatsApp outbound connect call failed"
            );
            // A local failure (no response, bad auth) is `callFailed`.
            let meta_code = (error.status != 0).then(|| error.code_number()).flatten();
            return Ok(outcome(voip::outbound_outcome_for_code(meta_code)));
        }
    };
    let deadline_at = now_ms() + voip::OUTBOUND_DIAL_DEADLINE_MS;
    let setup = async {
        store
            .start_outbound_dial(c.db, &wacid, &user_id.to_string(), deadline_at)
            .await?;
        let (id, bind) = (pending.id, wacid.clone());
        let bound = tx::write(c.db, move |conn| Ok(calls::attach_wacid(conn, id, &bind)?)).await?;
        if bound.as_ref().is_none_or(voip::is_call_ended) {
            return Ok(false);
        }
        if let Some(jobs) = c.jobs {
            voip::enqueue_outbound_dial_expiry(
                jobs.as_ref(),
                &attempt_id,
                pending.id,
                &wacid,
                workspace_id,
                deadline_at,
            )
            .await?;
        }
        Ok::<bool, BusinessError>(true)
    }
    .await;
    match setup {
        Ok(true) => {}
        Ok(false) => {
            abandon_outbound_dial(&store, &graph, &wacid, &attempt_id).await;
            tracing::info!(
                whatsapp_call_id = pending.id,
                wacid,
                attempt_id,
                "WhatsApp outbound dial: ended before Meta connected; hung up the connected leg"
            );
            return Ok(outcome("callFailed"));
        }
        Err(error) => {
            tracing::error!(err = %error.message(), whatsapp_call_id = pending.id, wacid, attempt_id, "WhatsApp outbound dial: post-connect setup failed after Meta connect succeeded; compensating");
            abandon_outbound_dial(&store, &graph, &wacid, &attempt_id).await;
            if let Err(finalize) = voip::finalize_row(
                c.db,
                pending.id,
                WhatsappCallTerminalStatus::Failed,
                false,
                Some("outbound-setup-failed".into()),
            )
            .await
            {
                tracing::error!(err = %finalize.message(), whatsapp_call_id = pending.id, attempt_id, "WhatsApp outbound dial: failed to finalize pending row after post-connect failure");
            }
            return Ok(outcome("callFailed"));
        }
    }
    let browser_recording = browser_recording(&integration);
    let recording_requested =
        (announcement_applied && options.recording.is_some()) || browser_recording;
    voip::record_recording_arrangement(
        c.db,
        pending.id,
        recording_requested,
        options.recording.is_some(),
    )
    .await;
    claim_conversation_for_call_agent(c, workspace_id, conversation.id, user_id).await;
    Ok(json!({
        "outcome": "dialing",
        "whatsappCallId": pending.id.to_string(),
        "wacid": wacid,
        "attemptId": attempt_id,
        "deadlineAt": iso(deadline_at),
        "browserRecordingEnabled": browser_recording,
        "recordingRequested": recording_requested,
    }))
}

// ---- hangup ---------------------------------------------------------------------------

/// `endVoipCallAsAgent`: `false` = nothing left to end.
pub async fn end_call_as_agent(
    c: Calling<'_>,
    workspace_id: i64,
    user_id: i64,
    whatsapp_call_id: i64,
    graph_failure_log: &str,
) -> Result<bool> {
    let call = tx::read(c.db, move |conn| {
        Ok(calls::find_by_id(conn, whatsapp_call_id)?)
    })
    .await?;
    let Some(call) = call.filter(|call| call.workspace_id == workspace_id) else {
        return Err(system_error(messages::CALL_NOT_FOUND));
    };
    let business_cancel = call.direction == "businessInitiated";
    let owner =
        call.initiated_by_user_id == Some(user_id) || call.answered_by_user_id == Some(user_id);
    let canceled_error = || Some(voip::CALL_CANCELED_BY_BUSINESS_LAST_ERROR.to_owned());
    let Some(wacid) = call.wacid.clone() else {
        if !owner {
            return Err(system_error(messages::VOIP_NOT_RESERVED_AGENT));
        }
        voip::finalize_row(
            c.db,
            whatsapp_call_id,
            WhatsappCallTerminalStatus::Failed,
            business_cancel,
            business_cancel.then(canceled_error).flatten(),
        )
        .await?;
        return Ok(true);
    };
    let store = c.store();
    let control = store.read_control(&wacid).await?;
    let reserved_agent = match &control {
        Some(control) => control.reserved_user_id == user_id.to_string(),
        None => owner,
    };
    if !reserved_agent {
        return Err(system_error(messages::VOIP_NOT_RESERVED_AGENT));
    }
    let ended = match control {
        Some(_) => store.end_call(&wacid, true).await?,
        None => voip::end_outcome_without_control(&call),
    };
    let Some(ended) = ended else {
        return Ok(false);
    };
    let integration = integration_by_inbox(c.db, workspace_id, call.inbox_id)
        .await?
        .ok_or_else(|| system_error(messages::NOT_FOUND))?;
    let graph = GraphAuth {
        base_url: c.graph_url,
        auth: &integration.auth.0,
    };
    if let Err(error) = voip::end_call_at_meta(&graph, &wacid, ended.graph_action).await {
        tracing::warn!(
            whatsapp_call_id,
            wacid,
            action = ended.graph_action.as_str(),
            status = error.status,
            "{graph_failure_log}"
        );
    }
    let failed_cancel =
        business_cancel && ended.terminal_status == WhatsappCallTerminalStatus::Failed;
    voip::finalize_row(
        c.db,
        whatsapp_call_id,
        ended.terminal_status,
        failed_cancel,
        failed_cancel.then(canceled_error).flatten(),
    )
    .await?;
    store.delete_offer(&wacid).await?;
    Ok(true)
}

// ---- TURN -------------------------------------------------------------------------------

/// `getWhatsappVoipTurnCredentialsAction`.
pub async fn turn_credentials_for_call(
    c: Calling<'_>,
    workspace_id: i64,
    user_id: i64,
    whatsapp_call_id: i64,
) -> Result<Value> {
    let call = tx::read(c.db, move |conn| {
        Ok(calls::find_by_id(conn, whatsapp_call_id)?)
    })
    .await?;
    let Some((call, wacid)) = call
        .filter(|call| call.workspace_id == workspace_id)
        .and_then(|call| call.wacid.clone().map(|w| (call, w)))
    else {
        return Err(system_error(messages::CALL_NOT_FOUND));
    };
    let control = c.store().read_control(&wacid).await?;
    let allowed = control.is_some_and(|control| {
        control.reserved_user_id.is_empty() || control.reserved_user_id == user_id.to_string()
    });
    if !allowed {
        return Err(system_error(messages::VOIP_NOT_RESERVED_AGENT));
    }
    if !voip::can_call_conversation(c.db, workspace_id, call.conversation_id, user_id).await? {
        return Err(BusinessError::exception(
            messages::VOIP_CALL_ACCESS_DENIED,
            CALL_ACCESS_DENIED_CODE,
            403,
        ));
    }
    let (url, secret) = voip::turn_env();
    Ok(voip::issue_turn_credentials(
        user_id,
        &wacid,
        url.as_deref(),
        secret.as_deref(),
    ))
}

// ---- outbound call mode --------------------------------------------------------------------

/// `getAppWebhookSubscriptions`: the app's subscriptions (`appId|appSecret` token).
async fn app_subscriptions(
    graph_url: &str,
    app_id: &str,
    app_secret: &str,
) -> std::result::Result<Vec<Value>, GraphError> {
    let request = voip::graph_client()
        .get(format!(
            "{graph_url}/{}/{app_id}/subscriptions",
            crate::whatsapp::DEFAULT_API_VERSION
        ))
        .bearer_auth(format!("{app_id}|{app_secret}"));
    let body = voip::graph_send(request).await?;
    Ok(body["data"].as_array().cloned().unwrap_or_default())
}

const WABA_OBJECT: &str = "whatsapp_business_account";

/// The platform WhatsApp app credential of the workspace (`resolveForOwner`).
async fn app_credential(
    db: &Db,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
) -> Result<Option<(String, String, String)>> {
    let Some(encryption) = encryption.cloned() else {
        return Ok(None);
    };
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    let credential = tx::read(db, move |conn| {
        crate::platform_credential::resolve_for_owner(conn, &encryption, owner, "whatsapp", false)
    })
    .await
    .ok()
    .flatten();
    Ok(credential.and_then(|cred| {
        let text = |k: &str| cred.text(k).map(str::to_owned);
        Some((
            text("clientId")?,
            text("clientSecret")?,
            text("verifyToken").unwrap_or_default(),
        ))
    }))
}

/// `getWhatsappCallingPreflight(..).callsSubscribed` with `hasAppCredential`:
/// `None` = no app credential (manual number) or unknown.
async fn calls_subscribed(
    db: &Db,
    graph_url: &str,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
    auth: &Value,
) -> Option<bool> {
    if auth.pointer("/metadata/isManual") == Some(&Value::Bool(true)) {
        return None;
    }
    let (app_id, secret, _) = app_credential(db, encryption, workspace_id)
        .await
        .ok()
        .flatten()?;
    let subscriptions = app_subscriptions(graph_url, &app_id, &secret).await.ok()?;
    let waba = subscriptions.iter().find(|s| s["object"] == WABA_OBJECT);
    Some(waba.is_some_and(|s| {
        s["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|f| f["name"] == "calls")
    }))
}

/// `describeForAgent`: Meta's own sentence (title and message, once each) and its code.
fn describe_for_agent(error: &GraphError) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in [error.user_title.as_deref(), error.user_message.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
    {
        if !part.is_empty() && !parts.contains(&part) {
            parts.push(part);
        }
    }
    let sentence = if parts.is_empty() {
        error.message.trim().to_owned()
    } else {
        parts.join(": ")
    };
    if sentence.is_empty() {
        return None;
    }
    Some(match error.code_number() {
        Some(code) if !sentence.contains(&code.to_string()) => format!("{sentence} (code {code})"),
        _ => sentence,
    })
}

/// `getWhatsappCallingPreflight` (`probeEligibility: true`).
pub async fn calling_preflight(
    db: &Db,
    graph_url: &str,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
    auth: &Value,
    inbox_id: i64,
) -> Value {
    let graph = GraphAuth {
        base_url: graph_url,
        auth,
    };
    let probe = async {
        let contact_inbox = tx::read(db, move |c| {
            let sql = format!(
                r#"SELECT {} FROM "ContactInbox" WHERE "inboxId" = ?1 AND "channel" = 'whatsapp' ORDER BY "id" LIMIT 1"#,
                ContactInbox::columns_sql(None)
            );
            Ok(chatbotx_db::repo::query_models(c, &sql, &[&inbox_id])?.into_iter().next())
        })
        .await
        .ok()
        .flatten()?;
        let (_, value, field) = voip::dial_identity(&contact_inbox);
        match voip::get_call_permissions(&graph, field, &value).await {
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(
                    status = error.status,
                    inbox_id,
                    "Whatsapp calling: Meta refused the calling-eligibility probe"
                );
                describe_for_agent(&error)
            }
        }
    };
    let (detail, ineligible) =
        futures::future::join(voip::phone_number_detail(&graph), probe).await;
    let detail = detail.ok();
    let platform_type = detail
        .as_ref()
        .and_then(|d| d["platform_type"].as_str().map(str::to_owned));
    let manual = auth.pointer("/metadata/isManual") == Some(&Value::Bool(true));
    let credential = if manual {
        None
    } else {
        app_credential(db, encryption, workspace_id)
            .await
            .ok()
            .flatten()
    };
    let calls_subscribed =
        match &credential {
            Some((app_id, secret, _)) => app_subscriptions(graph_url, app_id, secret)
                .await
                .ok()
                .map(|subs| {
                    subs.iter()
                        .find(|s| s["object"] == WABA_OBJECT)
                        .is_some_and(|s| {
                            s["fields"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|f| f["name"] == "calls")
                        })
                }),
            None => None,
        };
    json!({
        "isManual": manual,
        "hasAppCredential": credential.is_some(),
        "callsSubscribed": calls_subscribed,
        "platformType": platform_type,
        "isCloudApiPlatform": platform_type.as_ref().map(|p| p == "CLOUD_API"),
        "messagingLimitTier": detail.as_ref().and_then(|d| d["whatsapp_business_manager_messaging_limit"].as_str()),
        "callingIneligibleReason": ineligible,
    })
}

/// The `whatsapps/[id]/calls` page data: `None` = no such channel.
pub async fn calls_settings_page(
    db: &Db,
    graph_url: &str,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<Value>> {
    let Some(integration) = tx::read(db, move |c| {
        Ok(integration_whatsapp::find_by_id_for_workspace(
            c,
            integration_id,
            workspace_id,
        )?)
    })
    .await?
    else {
        return Ok(None);
    };
    let workspace = crate::workspace::find_by_id(db, workspace_id).await?;
    let auth = &integration.auth.0;
    let graph = GraphAuth {
        base_url: graph_url,
        auth,
    };
    // `resolveEffectiveCallingSettings`: on only when Meta and the workspace are on.
    let (settings, load_error) = match voip::get_calling_settings(&graph).await {
        Ok(mut settings) => {
            if integration.calling_enabled != Some(true) {
                settings["status"] = json!("DISABLED");
            }
            (settings, None)
        }
        Err(error) => (Value::Null, Some(error.message)),
    };
    let preflight = calling_preflight(
        db,
        graph_url,
        encryption,
        workspace_id,
        auth,
        integration.inbox_id,
    )
    .await;
    Ok(Some(json!({
        "inboundCallsEnabled": integration.inbound_calls_enabled,
        "recordingEnabled": integration.call_recording_enabled,
        "recordingRetentionDays": integration.call_recording_retention_days,
        "transcriptionEnabled": integration.call_transcription_enabled,
        "settings": settings,
        "loadError": load_error,
        "preflight": preflight,
        "workspaceTimezone": workspace.timezone,
    })))
}

/// `resolveOutboundCallModeAction`.
pub async fn resolve_outbound_call_mode(
    c: Calling<'_>,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
    user_id: i64,
    conversation_id: i64,
    contact_inbox_id: Option<i64>,
) -> Result<Value> {
    let none = |reason: &str| Ok(json!({ "mode": "none", "reason": reason }));
    let conversation = tx::read(c.db, move |conn| {
        crate::conversation_list::find_in(conn, workspace_id, conversation_id)
    })
    .await?
    .ok_or_else(|| system_error(messages::CALL_NOT_FOUND))?;
    if !voip::can_call_conversation(c.db, workspace_id, conversation.id, user_id).await? {
        return none("callAccessDenied");
    }
    let Some(contact_inbox) =
        whatsapp_contact_inbox(c.db, conversation.contact_id, contact_inbox_id).await?
    else {
        return none("notWhatsappConversation");
    };
    let Some(integration) =
        integration_by_inbox(c.db, workspace_id, contact_inbox.inbox_id).await?
    else {
        return none("notWhatsappConversation");
    };
    let auth = integration.auth.0.clone();
    if integration.calling_enabled != Some(true) {
        return none("callingNotEnabled");
    }
    let graph = GraphAuth {
        base_url: c.graph_url,
        auth: &auth,
    };
    // ponytail: the 5-minute calling-settings cache is not kept; Meta is read
    // on every mode check.
    let Ok(settings) = voip::get_calling_settings(&graph).await else {
        return none("tokenInvalid");
    };
    if settings["status"] != "ENABLED" {
        return none("callingNotEnabled");
    }
    if calls_subscribed(c.db, c.graph_url, encryption, workspace_id, &auth).await == Some(false) {
        return none("webhookNotSubscribed");
    }
    if voip::is_blocked_business_calling_country(&integration.display_phone_number) {
        return none("ineligibleNumber");
    }
    let mut status = voip::resolve_permission_status(c.db, contact_inbox.id, now_ms()).await?;
    if status.is_none() {
        let (_, value, field) = voip::dial_identity(&contact_inbox);
        match voip::get_call_permissions(&graph, field, &value).await {
            Ok(response) => {
                status = voip::meta_permission_status(&response);
                if let Some(found) = status
                    && let Err(error) = voip::mirror_provider_grant(
                        c.db,
                        workspace_id,
                        contact_inbox.id,
                        found,
                        voip::meta_permission_expiration(&response),
                    )
                    .await
                {
                    tracing::warn!(err = %error.message(), contact_inbox_id = contact_inbox.id, "Whatsapp calling: could not mirror the call permission Meta reported");
                }
            }
            Err(error) => {
                tracing::warn!(
                    status = error.status,
                    integration_id = integration.id,
                    "Whatsapp calling: could not read call permissions from Meta"
                );
            }
        }
    }
    let manual = auth.pointer("/metadata/isManual") == Some(&Value::Bool(true));
    let has_secret = auth
        .get("clientSecret")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    Ok(json!({
        "mode": "voip",
        "permissionStatus": status,
        "unsignedWebhookWarning": manual && !has_secret,
        "manualCallsSubscriptionUnverified": manual,
        "integrationId": integration.id.to_string(),
    }))
}

// ---- calling settings / call hours ------------------------------------------------------------

async fn workspace_integration(
    db: &Db,
    ws: i64,
    id: i64,
    messages: &CallingMessages,
) -> Result<IntegrationWhatsapp> {
    tx::read(db, move |c| {
        Ok(integration_whatsapp::find_by_id_for_workspace(c, id, ws)?)
    })
    .await?
    .ok_or_else(|| BusinessError::exception(messages.not_found, "notFound", 404))
}

/// `updateWhatsappCallingSettingsSchema` (every field optional).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallingSettingsInput {
    pub status: Option<String>,
    pub call_icon_visibility: Option<String>,
    pub callback_permission_status: Option<String>,
    pub recording_enabled: Option<bool>,
    pub call_recording_retention_days: Option<i64>,
    pub call_transcription_enabled: Option<bool>,
    pub inbound_calls_enabled: Option<bool>,
}

/// `integrationWhatsappService.updateCallSettings`: recording off turns
/// transcription off; transcription alone needs recording on.
async fn persist_call_settings(
    db: &Db,
    ws: i64,
    id: i64,
    mut values: chatbotx_db::repo::patch::Patch,
    messages: &CallingMessages,
) -> Result<()> {
    use chatbotx_db::rusqlite::types::Value as Sql;
    if values.is_empty() {
        return Ok(());
    }
    let flag = |values: &chatbotx_db::repo::patch::Patch, key: &str| {
        values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v == &Sql::Integer(1))
    };
    let transcription_alone = flag(&values, "callTranscriptionEnabled") == Some(true)
        && flag(&values, "callRecordingEnabled") != Some(true);
    if flag(&values, "callRecordingEnabled") == Some(false) {
        values.retain(|(k, _)| *k != "callTranscriptionEnabled");
        values.push(("callTranscriptionEnabled", Sql::Integer(0)));
    }
    let row = tx::write(db, move |c| {
        Ok(integration_whatsapp::update_call_settings(
            c,
            id,
            ws,
            &values,
            transcription_alone,
        )?)
    })
    .await?;
    if transcription_alone && row.is_none() {
        return Err(system_error(messages.transcription_requires_recording));
    }
    Ok(())
}

/// `updateWhatsappCallingSettings`: Meta first, then the local mirror.
pub async fn update_calling_settings(
    db: &Db,
    graph_url: &str,
    workspace_id: i64,
    integration_id: i64,
    input: CallingSettingsInput,
    messages: &CallingMessages,
) -> Result<()> {
    use chatbotx_db::rusqlite::types::Value as Sql;
    let integration = workspace_integration(db, workspace_id, integration_id, messages).await?;
    let mut meta = serde_json::Map::new();
    if let Some(status) = &input.status {
        meta.insert("status".into(), json!(status));
    }
    if let Some(v) = &input.call_icon_visibility {
        meta.insert("call_icon_visibility".into(), json!(v));
    }
    if let Some(v) = &input.callback_permission_status {
        meta.insert("callback_permission_status".into(), json!(v));
    }
    let bool_sql = |b: bool| Sql::Integer(i64::from(b));
    let mut local: chatbotx_db::repo::patch::Patch = Vec::new();
    if let Some(v) = input.recording_enabled {
        local.push(("callRecordingEnabled", bool_sql(v)));
    }
    if let Some(v) = input.call_recording_retention_days {
        local.push(("callRecordingRetentionDays", Sql::Integer(v)));
    }
    if let Some(v) = input.call_transcription_enabled {
        local.push(("callTranscriptionEnabled", bool_sql(v)));
    }
    if let Some(v) = input.inbound_calls_enabled {
        local.push(("inboundCallsEnabled", bool_sql(v)));
    }
    if meta.is_empty() {
        return persist_call_settings(db, workspace_id, integration_id, local, messages).await;
    }
    let graph = GraphAuth {
        base_url: graph_url,
        auth: &integration.auth.0,
    };
    voip::update_calling_settings(&graph, Value::Object(meta))
        .await
        .map_err(|error| system_error(error.public_message(messages.update_failed)))?;
    if let Some(status) = &input.status {
        local.push(("callingEnabled", bool_sql(status == "ENABLED")));
    }
    match persist_call_settings(db, workspace_id, integration_id, local, messages).await {
        Ok(()) => Ok(()),
        Err(error @ BusinessError::Exception { .. }) => Err(error),
        Err(error) => {
            tracing::error!(err = %error.message(), workspace_id, integration_id, "Whatsapp calling: Meta accepted the settings but the local mirror write failed");
            Err(BusinessError::exception(
                messages.saved_on_meta_only,
                WHATSAPP_CALLING_UPSTREAM_CODE,
                502,
            ))
        }
    }
}

/// `CALL_HOURS_DAYS`.
pub const CALL_HOURS_DAYS: [&str; 7] = [
    "MONDAY",
    "TUESDAY",
    "WEDNESDAY",
    "THURSDAY",
    "FRIDAY",
    "SATURDAY",
    "SUNDAY",
];
const LAST_MINUTE_OF_DAY: i64 = 23 * 60 + 59;
const FALLBACK_TIMEZONE: &str = "Etc/UTC";

/// `CALL_HOURS_TIMEZONE_CODES` membership (a workspace timezone chrono-tz knows).
pub fn is_call_hours_timezone(timezone: &str) -> bool {
    crate::workspace::codes::TIMEZONE_CODES.contains(&timezone)
        && timezone.parse::<chrono_tz::Tz>().is_ok()
}

/// One range of a day (minutes since midnight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallHoursRange {
    pub open: i64,
    pub close: i64,
}

/// `CallHoursFormValues`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallHoursForm {
    pub enabled: bool,
    pub timezone_id: String,
    /// MONDAY..SUNDAY.
    pub days: Vec<Vec<CallHoursRange>>,
}

/// A schema issue: `(path, message)`.
pub type CallHoursIssue = (Vec<Value>, String);

/// `callHoursFormSchema.safeParse` (the first issue); messages as the public route maps them.
pub fn parse_call_hours(input: &Value) -> std::result::Result<CallHoursForm, CallHoursIssue> {
    let issue = |path: Vec<Value>, message: &str| Err((path, message.to_owned()));
    let Some(enabled) = input.get("enabled").and_then(Value::as_bool) else {
        return issue(vec![json!("enabled")], "Invalid input: expected boolean");
    };
    let timezone = input
        .get("timezoneId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_call_hours_timezone(timezone) {
        return issue(vec![json!("timezoneId")], "Invalid option");
    }
    let Some(days) = input.get("days").and_then(Value::as_array) else {
        return issue(vec![json!("days")], "Invalid input: expected array");
    };
    if days.len() != CALL_HOURS_DAYS.len() {
        return issue(
            vec![json!("days")],
            "Too small: expected array to have 7 items",
        );
    }
    let mut out = Vec::new();
    for (index, day) in days.iter().enumerate() {
        let name = day.get("dayOfWeek").and_then(Value::as_str);
        if !name.is_some_and(|n| CALL_HOURS_DAYS.contains(&n)) {
            return issue(
                vec![json!("days"), json!(index), json!("dayOfWeek")],
                "Invalid option",
            );
        }
        let Some(ranges) = day.get("ranges").and_then(Value::as_array) else {
            return issue(
                vec![json!("days"), json!(index), json!("ranges")],
                "Invalid input: expected array",
            );
        };
        if ranges.len() > 2 {
            return issue(
                vec![json!("days"), json!(index), json!("ranges")],
                "Too big: expected array to have <=2 items",
            );
        }
        let mut parsed = Vec::new();
        for (r, range) in ranges.iter().enumerate() {
            let minute = |key: &str| {
                range
                    .get(key)
                    .and_then(Value::as_i64)
                    .filter(|m| (0..=LAST_MINUTE_OF_DAY).contains(m))
            };
            let (Some(open), Some(close)) = (minute("openMinute"), minute("closeMinute")) else {
                return issue(
                    vec![json!("days"), json!(index), json!("ranges"), json!(r)],
                    "Invalid minute of day",
                );
            };
            parsed.push(CallHoursRange { open, close });
        }
        out.push((name.unwrap_or_default(), parsed));
    }
    for (index, (name, ranges)) in out.iter().enumerate() {
        if *name != CALL_HOURS_DAYS[index] {
            return issue(
                vec![json!("days"), json!(index), json!("dayOfWeek")],
                "Days must be listed in week order",
            );
        }
        for (r, range) in ranges.iter().enumerate() {
            if range.close <= range.open {
                return issue(
                    vec![json!("days"), json!(index), json!("ranges"), json!(r)],
                    "A range must close after it opens.",
                );
            }
        }
        if let [first, second] = ranges.as_slice()
            && first.open < second.close
            && second.open < first.close
        {
            return issue(
                vec![json!("days"), json!(index), json!("ranges")],
                "The two ranges of a day must not overlap.",
            );
        }
    }
    if out.iter().all(|(_, ranges)| ranges.is_empty()) {
        return issue(vec![json!("days")], "At least one day needs an open range.");
    }
    Ok(CallHoursForm {
        enabled,
        timezone_id: timezone.to_owned(),
        days: out.into_iter().map(|(_, r)| r).collect(),
    })
}

/// `callHoursIssueMessage`: `path.join(".")` + `: ` + message.
pub fn call_hours_issue_message(issue: &CallHoursIssue) -> String {
    let path: Vec<String> = issue
        .0
        .iter()
        .map(|p| p.as_str().map_or_else(|| p.to_string(), str::to_owned))
        .collect();
    if path.is_empty() {
        issue.1.clone()
    } else {
        format!("{}: {}", path.join("."), issue.1)
    }
}

/// `formatMetaCallTime`.
fn meta_time(minute: i64) -> String {
    let m = minute.clamp(0, LAST_MINUTE_OF_DAY);
    format!("{:02}{:02}", m / 60, m % 60)
}

/// `parseMetaCallTime`.
fn parse_meta_time(value: &str) -> Option<i64> {
    let digits: String = value.chars().filter(|c| *c != ':').collect();
    let valid = value.len() == 4 || (value.len() == 5 && value.as_bytes()[2] == b':');
    if !valid || digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = (
        digits[..2].parse::<i64>().ok()?,
        digits[2..].parse::<i64>().ok()?,
    );
    (h <= 23 && m < 60).then_some(h * 60 + m)
}

/// `toMetaCallHours(values, holidays)`.
pub fn to_meta_call_hours(form: &CallHoursForm, holidays: Option<Vec<Value>>) -> Value {
    let mut weekly = Vec::new();
    for (index, ranges) in form.days.iter().enumerate() {
        let mut sorted = ranges.clone();
        sorted.sort_by_key(|r| r.open);
        for range in sorted {
            weekly.push(json!({
                "day_of_week": CALL_HOURS_DAYS[index],
                "open_time": meta_time(range.open),
                "close_time": meta_time(range.close),
            }));
        }
    }
    let mut out = json!({
        "status": if form.enabled { "ENABLED" } else { "DISABLED" },
        "timezone_id": form.timezone_id,
        "weekly_operating_hours": weekly,
    });
    if let Some(holidays) = holidays {
        out["holiday_schedule"] = Value::Array(holidays);
    }
    out
}

/// `upcomingHolidays`: the holidays on or after today in the timezone.
pub fn upcoming_holidays(holidays: Option<&Value>, timezone: &str, now: i64) -> Option<Vec<Value>> {
    let holidays = holidays?.as_array()?;
    let tz = timezone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    let today = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now)
        .unwrap_or_default()
        .with_timezone(&tz)
        .format("%Y-%m-%d")
        .to_string();
    Some(
        holidays
            .iter()
            .filter(|h| h["date"].as_str().is_some_and(|d| d >= today.as_str()))
            .cloned()
            .collect(),
    )
}

/// `toCallHoursSnapshot`.
pub fn to_call_hours_snapshot(call_hours: &Value) -> Value {
    let weekly: Vec<Value> = call_hours["weekly_operating_hours"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| json!({ "dayOfWeek": w["day_of_week"], "openTime": w["open_time"], "closeTime": w["close_time"] }))
        .collect();
    let mut out = json!({
        "status": call_hours["status"],
        "timezoneId": call_hours["timezone_id"],
        "weeklyOperatingHours": weekly,
    });
    if let Some(holidays) = call_hours["holiday_schedule"].as_array() {
        out["holidaySchedule"] = Value::Array(
            holidays
                .iter()
                .map(|h| json!({ "date": h["date"], "startTime": h["start_time"], "endTime": h["end_time"] }))
                .collect(),
        );
    }
    out
}

/// `buildCallHoursFormValues(fromCallHoursSnapshot(snapshot), workspaceTimezone)`
/// as the public `callHoursInput`.
pub fn call_hours_form_values(snapshot: Option<&Value>, workspace_timezone: &str) -> Value {
    let known = |candidates: &[&str]| {
        candidates
            .iter()
            .copied()
            .find(|tz| is_call_hours_timezone(tz))
            .unwrap_or(FALLBACK_TIMEZONE)
            .to_owned()
    };
    let Some(snapshot) = snapshot.filter(|s| s.is_object()) else {
        let days: Vec<Value> = CALL_HOURS_DAYS
            .iter()
            .enumerate()
            .map(|(i, day)| {
                let ranges = if i < 5 {
                    json!([{ "openMinute": 540, "closeMinute": 1020 }])
                } else {
                    json!([])
                };
                json!({ "dayOfWeek": day, "ranges": ranges })
            })
            .collect();
        return json!({ "enabled": false, "timezoneId": known(&[workspace_timezone]), "days": days });
    };
    let timezone = snapshot["timezoneId"].as_str().unwrap_or_default();
    let weekly = snapshot["weeklyOperatingHours"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let days: Vec<Value> = CALL_HOURS_DAYS
        .iter()
        .map(|day| {
            let mut ranges: Vec<(i64, i64)> = weekly
                .iter()
                .filter(|w| w["dayOfWeek"] == *day)
                .filter_map(|w| {
                    Some((
                        parse_meta_time(w["openTime"].as_str()?)?,
                        parse_meta_time(w["closeTime"].as_str()?)?,
                    ))
                })
                .collect();
            ranges.sort_by_key(|r| r.0);
            let ranges: Vec<Value> = ranges
                .into_iter()
                .map(|(o, c)| json!({ "openMinute": o, "closeMinute": c }))
                .collect();
            json!({ "dayOfWeek": day, "ranges": ranges })
        })
        .collect();
    json!({
        "enabled": snapshot["status"] == "ENABLED",
        "timezoneId": known(&[timezone, workspace_timezone]),
        "days": days,
    })
}

/// `updateWhatsappCallHours`: Meta first (keeping its upcoming holidays),
/// then the local snapshot.
pub async fn update_call_hours(
    db: &Db,
    graph_url: &str,
    workspace_id: i64,
    integration_id: i64,
    form: &CallHoursForm,
    messages: &CallingMessages,
) -> Result<()> {
    let integration = workspace_integration(db, workspace_id, integration_id, messages).await?;
    let graph = GraphAuth {
        base_url: graph_url,
        auth: &integration.auth.0,
    };
    let current = voip::get_calling_settings(&graph).await.map_err(|_| {
        BusinessError::exception(messages.update_failed, WHATSAPP_CALLING_UPSTREAM_CODE, 502)
    })?;
    let holidays = upcoming_holidays(
        current.pointer("/call_hours/holiday_schedule"),
        &form.timezone_id,
        now_ms(),
    );
    let call_hours = to_meta_call_hours(form, holidays);
    voip::update_calling_settings(&graph, json!({ "call_hours": call_hours }))
        .await
        .map_err(|error| system_error(error.public_message(messages.update_failed)))?;
    let snapshot = to_call_hours_snapshot(&call_hours).to_string();
    let values: chatbotx_db::repo::patch::Patch = vec![(
        "callHours",
        chatbotx_db::rusqlite::types::Value::Text(snapshot),
    )];
    let written = tx::write(db, move |c| {
        Ok(integration_whatsapp::update_call_settings(
            c,
            integration_id,
            workspace_id,
            &values,
            false,
        )?)
    })
    .await;
    if let Err(error) = written {
        tracing::error!(err = %error.message(), workspace_id, integration_id, "Whatsapp calling: Meta accepted the call hours but the local mirror write failed");
        return Err(BusinessError::exception(
            messages.saved_on_meta_only,
            WHATSAPP_CALLING_UPSTREAM_CODE,
            502,
        ));
    }
    Ok(())
}

/// `whatsappChannels.getCallingSettings`: the stored settings + `callHoursInput`.
pub async fn calling_settings_resource(
    db: &Db,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<Value>> {
    let Some(integration) = tx::read(db, move |c| {
        Ok(integration_whatsapp::find_by_id_for_workspace(
            c,
            integration_id,
            workspace_id,
        )?)
    })
    .await?
    else {
        return Ok(None);
    };
    let workspace = crate::workspace::find_by_id(db, workspace_id).await?;
    let hours = integration.call_hours.as_ref().map(|h| h.0.clone());
    Ok(Some(json!({
        "callingEnabled": integration.calling_enabled,
        "inboundCallsEnabled": integration.inbound_calls_enabled,
        "callRecordingEnabled": integration.call_recording_enabled,
        "callRecordingRetentionDays": integration.call_recording_retention_days,
        "callTranscriptionEnabled": integration.call_transcription_enabled,
        "callHours": hours,
        "callHoursInput": call_hours_form_values(hours.as_ref(), &workspace.timezone),
    })))
}

/// `fixWhatsappCallsSubscriptionAction`: `ensureWhatsappAppWebhookFields`.
pub async fn fix_calls_subscription(
    db: &Db,
    graph_url: &str,
    encryption: Option<&Arc<chatbotx_core::encryption::Encryption>>,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Value> {
    let integration = tx::read(db, move |c| {
        Ok(integration_whatsapp::find_by_id_for_workspace(
            c,
            integration_id,
            workspace_id,
        )?)
    })
    .await?
    .ok_or_else(|| system_error(messages::NOT_FOUND))?;
    crate::workspace::find_by_id(db, workspace_id)
        .await
        .map_err(|_| system_error(messages::NOT_FOUND))?;
    if integration.auth.0.pointer("/metadata/isManual") == Some(&Value::Bool(true)) {
        return Err(system_error(messages::MANUAL_INTEGRATION_NOTICE));
    }
    let Some((app_id, secret, verify_token)) = app_credential(db, encryption, workspace_id).await?
    else {
        return Err(system_error(messages::APP_SETTINGS_NOT_FOUND));
    };
    let fail = |error: GraphError| system_error(error.public_message("Something went wrong"));
    let subscriptions = app_subscriptions(graph_url, &app_id, &secret)
        .await
        .map_err(fail)?;
    let Some(existing) = subscriptions.iter().find(|s| s["object"] == WABA_OBJECT) else {
        return Err(system_error(messages::APP_SUBSCRIPTION_MISSING));
    };
    let mut fields: Vec<String> = existing["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["name"].as_str().map(str::to_owned))
        .collect();
    let callback = existing["callback_url"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let post = |fields: Vec<String>| {
        let (app_id, secret, verify_token, callback) = (
            app_id.clone(),
            secret.clone(),
            verify_token.clone(),
            callback.clone(),
        );
        async move {
            voip::graph_send(
                voip::graph_client()
                    .post(format!("{graph_url}/{}/{app_id}/subscriptions", crate::whatsapp::DEFAULT_API_VERSION))
                    .bearer_auth(format!("{app_id}|{secret}"))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(json!({ "object": WABA_OBJECT, "callback_url": callback, "fields": fields.join(","), "verify_token": verify_token }).to_string()),
            )
            .await
        }
    };
    let mut status = "already-subscribed";
    let missing: Vec<&str> = ["calls", "account_update"]
        .into_iter()
        .filter(|f| !fields.iter().any(|x| x == f))
        .collect();
    if !missing.is_empty() {
        fields.extend(missing.into_iter().map(str::to_owned));
        post(fields.clone()).await.map_err(fail)?;
        status = "subscribed";
    }
    if !fields.iter().any(|f| f == "account_settings_update") {
        let mut with_optional = fields.clone();
        with_optional.push("account_settings_update".into());
        if let Err(error) = post(with_optional).await {
            tracing::warn!(
                message = error.message,
                "Optional app webhook field subscription failed"
            );
        }
    }
    Ok(json!({ "status": status }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_call_times_round_trip() {
        assert_eq!(meta_time(540), "0900");
        assert_eq!(parse_meta_time("0900"), Some(540));
        assert_eq!(parse_meta_time("09:00"), Some(540));
        assert_eq!(parse_meta_time("2400"), None);
        assert_eq!(parse_meta_time("9:00"), None);
    }
}
