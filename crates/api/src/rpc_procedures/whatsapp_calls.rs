//! WhatsApp calling (VoIP) actions (`features/integration-whatsapp/calling/
//! actions`), the inbox call-artifact actions (`features/messages/actions/
//! *call*`) and the Calls page (`app/space/[workspaceId]/calls`,
//! `features/whatsapp-calls`).
//!
//! Gates as the original clients: `callingActionClient` (contacts access),
//! `callingAdminActionClient` (workspace super admin), `callHistoryActionClient`
//! (contacts access or analytics), and the plain workspace client for hangup,
//! heartbeat and the call artifacts (`assertCanReadCallArtifactOrThrow`).
//! ponytail: support sessions are not ported, so `rejectSupportSession` has nothing to reject.

use chatbotx_business::whatsapp_voip::{self as voip, ReadScope, VoipStore};
use chatbotx_business::whatsapp_voip_actions::{
    self as calling, BUILDER_CALLING_MESSAGES, Calling, CallingSettingsInput, messages,
};
use chatbotx_business::{BusinessError, tx, whatsapp};
use chatbotx_db::repo::whatsapp_call::WhatsappCallHistoryScope;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, invalid};
use crate::handlers::whatsapp_calls as history;
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "answerWhatsappVoipCallAction",
    "initiateOutboundVoipCallAction",
    "hangupWhatsappVoipCallAction",
    "heartbeatActiveVoipCallAction",
    "getOutboundVoipAnswerAction",
    "getPendingIncomingVoipCallAction",
    "getWhatsappVoipTurnCredentialsAction",
    "outboundVoipTurnCredentialsAction",
    "resolveOutboundCallModeAction",
    "requestCallPermissionAction",
    "updateWhatsappCallingSettingsAction",
    "updateWhatsappCallHoursAction",
    "fixWhatsappCallsSubscriptionAction",
    "getCallSummaryAction",
    "generateCallAiSummaryAction",
    "getCallTranscriptAction",
    "listCallSummaryProvidersAction",
    "getCallRecordingUrlAction",
    "listWhatsappCallsAction",
    "callsPage",
    "whatsappCallsPage",
];

const MAX_SDP_CHARS: usize = 100_000;
const MAX_ATTEMPT_ID_CHARS: usize = 200;
const TURN_MINT_WINDOW_SECONDS: u64 = 60;
const TURN_MINT_LIMIT_PER_WINDOW: i64 = 10;

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

fn fail(error: BusinessError) -> OrpcError {
    action_fail(error)
}

/// `requireContactsAccess` of `callingActionClient`.
fn require_calling(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.has("contacts") || ctx.has("onlyAssignedContacts") {
        return Ok(());
    }
    Err(fail(BusinessError::exception(
        messages::CALLING_ACCESS_DENIED,
        "callingAccessDenied",
        403,
    )))
}

/// `assertWorkspaceSuperAdmin` of `callingAdminActionClient`.
fn require_super_admin(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.permissions.get("superAdmin") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    Err(fail(calling::system_error(messages::SUPER_ADMIN_REQUIRED)))
}

/// `requireCallHistoryAccess` of `callHistoryActionClient`.
fn require_call_history(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.has("contacts") || ctx.has("onlyAssignedContacts") || ctx.has("analytics") {
        return Ok(());
    }
    Err(fail(BusinessError::exception(
        messages::CALL_HISTORY_ACCESS_DENIED,
        "callHistoryAccessDenied",
        403,
    )))
}

/// `assertCanReadCallArtifactOrThrow`.
async fn require_call_artifact(ctx: &Ctx<'_>, call_id: i64) -> Result<(), OrpcError> {
    let allowed = voip::can_read_call(
        &ctx.api.db,
        ctx.workspace_id,
        call_id,
        ctx.user_id,
        ctx.permissions.clone(),
        ReadScope::Artifact,
    )
    .await
    .map_err(fail)?;
    if allowed {
        return Ok(());
    }
    Err(fail(BusinessError::exception(
        messages::CALL_ARTIFACT_ACCESS_DENIED,
        "callArtifactAccessDenied",
        403,
    )))
}

fn services<'a>(ctx: &'a Ctx<'a>) -> Calling<'a> {
    Calling {
        db: &ctx.api.db,
        graph_url: &ctx.api.graph_url,
        jobs: ctx.api.jobs.as_ref(),
        realtime: ctx.api.realtime.as_ref(),
    }
}

fn bound_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

/// `z.string().min(1).max(max)` (not trimmed).
fn text(input: &In<'_>, key: &str, max: usize) -> Result<String, OrpcError> {
    input.str(key, false, 1, max)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id;
    let user = ctx.user_id;
    let input = In(arg(args, 1));
    match name {
        "answerWhatsappVoipCallAction" => {
            require_calling(ctx)?;
            let call_id = input.id("whatsappCallId")?;
            let sdp = text(&input, "sdpAnswer", MAX_SDP_CHARS)?;
            calling::answer_call(services(ctx), ws, user, call_id, &sdp)
                .await
                .map_err(fail)
        }
        "initiateOutboundVoipCallAction" => {
            require_calling(ctx)?;
            let conversation = input.id("conversationId")?;
            let contact_inbox = input.opt_id("contactInboxId")?;
            let sdp = text(&input, "sdpOffer", MAX_SDP_CHARS)?;
            calling::initiate_outbound_call(
                services(ctx),
                ws,
                user,
                conversation,
                contact_inbox,
                &sdp,
            )
            .await
            .map_err(fail)
        }
        "hangupWhatsappVoipCallAction" => {
            let call_id = input.id("whatsappCallId")?;
            calling::end_call_as_agent(
                services(ctx),
                ws,
                user,
                call_id,
                "WhatsApp VoIP call hangup: Graph action failed (call still finalized locally)",
            )
            .await
            .map_err(fail)?;
            Ok(json!({ "hungUp": true }))
        }
        "heartbeatActiveVoipCallAction" => {
            let wacid = text(&input, "wacid", usize::MAX)?;
            let ok = VoipStore::new(ctx.api.db.clone())
                .heartbeat_active_call(&ctx.api.db, &wacid, ws, user)
                .await
                .map_err(fail)?;
            Ok(json!({ "ok": ok }))
        }
        "getOutboundVoipAnswerAction" => {
            require_calling(ctx)?;
            let call_id = input.id("whatsappCallId")?;
            let store = VoipStore::new(ctx.api.db.clone());
            let sdp =
                voip::read_outbound_answer_for_initiator(&ctx.api.db, &store, ws, call_id, user)
                    .await
                    .map_err(fail)?;
            Ok(json!({ "sdp": sdp }))
        }
        "getPendingIncomingVoipCallAction" => {
            require_calling(ctx)?;
            let store = VoipStore::new(ctx.api.db.clone());
            voip::list_resumable_incoming(&ctx.api.db, &store, ws, user)
                .await
                .map_err(fail)
        }
        "getWhatsappVoipTurnCredentialsAction" => {
            require_calling(ctx)?;
            let call_id = input.id("whatsappCallId")?;
            calling::turn_credentials_for_call(services(ctx), ws, user, call_id)
                .await
                .map_err(fail)
        }
        "outboundVoipTurnCredentialsAction" => {
            require_calling(ctx)?;
            let attempt_id = text(&input, "attemptId", MAX_ATTEMPT_ID_CHARS)?;
            // `assertTurnMintNotRateLimited`: 10 mints a minute per user; no store = allow.
            if let Some(kv) = &ctx.api.kv {
                let window = ctx.now_ms / (TURN_MINT_WINDOW_SECONDS as i64 * 1000);
                let key = format!("outbound-voip-turn-mint-rate-limit:{user}:{window}");
                if kv.mem.incr_with_window(&key, TURN_MINT_WINDOW_SECONDS)
                    > TURN_MINT_LIMIT_PER_WINDOW
                {
                    return Err(fail(BusinessError::exception(
                        messages::VOIP_TURN_RATE_LIMITED,
                        "tooManyRequests",
                        429,
                    )));
                }
            }
            let (url, secret) = voip::turn_env();
            Ok(voip::issue_turn_credentials(
                user,
                &attempt_id,
                url.as_deref(),
                secret.as_deref(),
            ))
        }
        "resolveOutboundCallModeAction" => {
            require_calling(ctx)?;
            let conversation = input.id("conversationId")?;
            let contact_inbox = input.opt_id("contactInboxId")?;
            calling::resolve_outbound_call_mode(
                services(ctx),
                ctx.api.encryption.as_ref(),
                ws,
                user,
                conversation,
                contact_inbox,
            )
            .await
            .map_err(fail)
        }
        "requestCallPermissionAction" => request_call_permission(ctx, args).await,
        "updateWhatsappCallingSettingsAction" => {
            require_super_admin(ctx)?;
            let integration = bound_id(args, 1)?;
            let settings = calling_settings_input(&In(arg(args, 2)))?;
            calling::update_calling_settings(
                &ctx.api.db,
                &ctx.api.graph_url,
                ws,
                integration,
                settings,
                &BUILDER_CALLING_MESSAGES,
            )
            .await
            .map_err(fail)?;
            Ok(Value::Null)
        }
        "updateWhatsappCallHoursAction" => {
            require_super_admin(ctx)?;
            let integration = bound_id(args, 1)?;
            let form = calling::parse_call_hours(arg(args, 2)).map_err(|_| invalid())?;
            calling::update_call_hours(
                &ctx.api.db,
                &ctx.api.graph_url,
                ws,
                integration,
                &form,
                &BUILDER_CALLING_MESSAGES,
            )
            .await
            .map_err(fail)?;
            Ok(Value::Null)
        }
        "fixWhatsappCallsSubscriptionAction" => {
            require_super_admin(ctx)?;
            let integration = bound_id(args, 1)?;
            calling::fix_calls_subscription(
                &ctx.api.db,
                &ctx.api.graph_url,
                ctx.api.encryption.as_ref(),
                ws,
                integration,
            )
            .await
            .map_err(fail)
        }
        "getCallSummaryAction" => {
            let call_id = input.id("whatsappCallId")?;
            require_call_artifact(ctx, call_id).await?;
            let result = history::summary_for_call(ctx.api, ws, call_id)
                .await
                .map_err(fail)?;
            Ok(json!({ "result": result }))
        }
        "generateCallAiSummaryAction" => {
            let call_id = input.id("whatsappCallId")?;
            let provider = input.opt_str("provider")?.ok_or_else(invalid)?;
            if !history::SUMMARY_PROVIDERS.iter().any(|p| p.0 == provider) {
                return Err(invalid());
            }
            require_call_artifact(ctx, call_id).await?;
            let summary = history::generate_call_summary(ctx.api, ws, call_id, Some(provider))
                .await
                .map_err(fail)?;
            Ok(json!({ "aiSummary": summary }))
        }
        "getCallTranscriptAction" => {
            let call_id = input.id("whatsappCallId")?;
            require_call_artifact(ctx, call_id).await?;
            history::transcript(ctx.api, ws, call_id)
                .await
                .map_err(fail)
        }
        "listCallSummaryProvidersAction" => {
            let providers = history::providers_json(ctx.api, ws).await.map_err(fail)?;
            Ok(json!({ "providers": providers }))
        }
        "getCallRecordingUrlAction" => {
            let call_id = input.id("whatsappCallId")?;
            require_call_artifact(ctx, call_id).await?;
            let url = history::recording_url(ctx.api, ws, call_id, &[]).await?;
            Ok(json!({ "url": url }))
        }
        "listWhatsappCallsAction" => {
            require_call_history(ctx)?;
            list_calls(ctx, &input).await
        }
        "whatsappCallsPage" => {
            // The `whatsapps/[id]` layout guard (`superAdmin`, else 404), then
            // `findIntegrationWhatsapp` (404).
            if ctx.permissions.get("superAdmin") != Some(&Value::Bool(true)) {
                return Err(super::fail(BusinessError::not_found("Page not found")));
            }
            let id = input.id("id")?;
            let page = calling::calls_settings_page(
                &ctx.api.db,
                &ctx.api.graph_url,
                ctx.api.encryption.as_ref(),
                ws,
                id,
            )
            .await
            .map_err(fail)?
            .ok_or_else(|| super::fail(BusinessError::not_found("Integration not found")))?;
            let mut page = page;
            page["isSuperAdmin"] = json!(true);
            Ok(page)
        }
        _ => calls_page(ctx, &input).await,
    }
}

/// `updateWhatsappCallingSettingsSchema`.
fn calling_settings_input(input: &In<'_>) -> Result<CallingSettingsInput, OrpcError> {
    let one_of = |key: &str, options: &[&str]| -> Result<Option<String>, OrpcError> {
        match input.opt_str(key)? {
            Some(value) if !options.contains(&value.as_str()) => Err(invalid()),
            value => Ok(value),
        }
    };
    let retention = match input.get("callRecordingRetentionDays") {
        None => None,
        Some(value) => match value.as_i64() {
            Some(days) if (1..=3650).contains(&days) => Some(days),
            _ => return Err(invalid()),
        },
    };
    Ok(CallingSettingsInput {
        status: one_of("status", &["ENABLED", "DISABLED"])?,
        call_icon_visibility: one_of("callIconVisibility", &["DEFAULT", "DISABLE_ALL"])?,
        callback_permission_status: one_of("callbackPermissionStatus", &["ENABLED", "DISABLED"])?,
        recording_enabled: input.opt_bool("recordingEnabled")?,
        call_recording_retention_days: retention,
        call_transcription_enabled: input.opt_bool("callTranscriptionEnabled")?,
        inbound_calls_enabled: input.opt_bool("inboundCallsEnabled")?,
    })
}

/// `requestCallPermissionAction`: `[workspaceId, conversationId]` + `{ text, inboxId? }`.
async fn request_call_permission(ctx: &Ctx<'_>, args: &[Value]) -> Result<Value, OrpcError> {
    require_calling(ctx)?;
    let conversation_id = bound_id(args, 1)?;
    let input = In(arg(args, 2));
    let text = input.str("text", true, 1, 1024)?;
    let inbox_id = input.opt_id("inboxId")?;
    let ws = ctx.workspace_id;
    let conversation =
        chatbotx_business::conversation_list::find_or_fail(&ctx.api.db, ws, conversation_id)
            .await
            .map_err(fail)?;
    let allowed = voip::can_call_conversation(&ctx.api.db, ws, conversation_id, ctx.user_id)
        .await
        .map_err(fail)?;
    if !allowed {
        return Err(fail(BusinessError::exception(
            messages::VOIP_CALL_ACCESS_DENIED,
            calling::CALL_ACCESS_DENIED_CODE,
            403,
        )));
    }
    const BUILDER_MESSAGES: whatsapp::CallPermissionMessages = whatsapp::CallPermissionMessages {
        not_whatsapp_conversation: messages::NOT_WHATSAPP_CONVERSATION,
        not_found: messages::NOT_FOUND,
        inbox_not_found: messages::NOT_FOUND,
        permission_check_failed: messages::PERMISSION_CHECK_FAILED,
        permission_request_limit_reached: messages::PERMISSION_REQUEST_LIMIT_REACHED,
    };
    let sent = whatsapp::request_call_permission(
        &ctx.api.db,
        &ctx.api.graph_url,
        &conversation,
        text,
        inbox_id,
        &BUILDER_MESSAGES,
        Some(ctx.user_id),
    )
    .await
    .map_err(fail)?;
    crate::handlers::contact_messages::deliver(ctx.api, &conversation, sent).await;
    Ok(Value::Null)
}

/// `resolveHistoryScope`: `None` = an empty page.
fn history_scope(ctx: &Ctx<'_>) -> Option<WhatsappCallHistoryScope> {
    if voip::is_call_history_admin(ctx.permissions) {
        return Some(WhatsappCallHistoryScope::AllCalls);
    }
    if ctx.has("contacts") {
        return Some(WhatsappCallHistoryScope::Own {
            user_id: ctx.user_id,
            assigned_only: false,
        });
    }
    ctx.has("onlyAssignedContacts")
        .then_some(WhatsappCallHistoryScope::Own {
            user_id: ctx.user_id,
            assigned_only: true,
        })
}

/// `listWhatsappCalls`: one page as the Calls page rows (`recordingPath`).
async fn list_calls(ctx: &Ctx<'_>, input: &In<'_>) -> Result<Value, OrpcError> {
    let activity = match input.opt_str("activity")? {
        Some(chip) if chip != "missed" && chip != "noReply" => return Err(invalid()),
        chip => chip,
    };
    let filter = history::ListFilter {
        activity,
        inbox_id: input.opt_id("inboxId")?,
        agent_user_id: input.opt_id("agentUserId")?,
        cursor: input.opt_str("cursor")?,
    };
    let Some(scope) = history_scope(ctx) else {
        return Ok(json!({ "data": [], "nextCursor": null }));
    };
    let (rows, next) = history::list_calls(ctx.api, ctx.workspace_id, scope, filter).await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut out = history::row_json(row);
            if let Some(object) = out.as_object_mut() {
                object.remove("hasRecording");
                object.insert("recordingPath".into(), json!(row.call.recording_path));
            }
            out
        })
        .collect();
    Ok(json!({ "data": data, "nextCursor": next }))
}

/// `callsPage` (`app/space/[workspaceId]/calls/page.tsx`): the first page,
/// the filter options and `showAgentFilter`. 404 without contacts access or analytics.
async fn calls_page(ctx: &Ctx<'_>, input: &In<'_>) -> Result<Value, OrpcError> {
    if !(ctx.has("contacts") || ctx.has("onlyAssignedContacts") || ctx.has("analytics")) {
        return Err(super::fail(BusinessError::not_found("Page not found")));
    }
    let show_agent_filter = voip::is_call_history_admin(ctx.permissions);
    // A non-admin's `agentUserId` is dropped (`scopedAgentUserId`).
    let mut query = input.0.clone();
    if !show_agent_filter && let Some(object) = query.as_object_mut() {
        object.remove("agentUserId");
    }
    let page = list_calls(ctx, &In(&query)).await?;
    let ws = ctx.workspace_id;
    let (inbox_options, agent_options) = tx::read(&ctx.api.db, move |c| {
        let mut stmt = c.prepare_cached(
            r#"SELECT "id", "name" FROM "Inbox" WHERE "workspaceId" = ?1 AND "channel" = 'whatsapp' ORDER BY "id""#,
        )?;
        let inboxes = stmt
            .query_map([ws], |r| {
                Ok(json!({ "id": r.get::<_, i64>(0)?.to_string(), "name": r.get::<_, Option<String>>(1)? }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut agents = Vec::new();
        if show_agent_filter {
            let mut stmt = c.prepare_cached(
                r#"SELECT u."id", COALESCE(u."name", u."email") FROM "WorkspaceMember" m
                   JOIN "User" u ON u."id" = m."userId" WHERE m."workspaceId" = ?1 ORDER BY m."id""#,
            )?;
            agents = stmt
                .query_map([ws], |r| {
                    Ok(json!({ "id": r.get::<_, i64>(0)?.to_string(), "name": r.get::<_, Option<String>>(1)? }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok((inboxes, agents))
    })
    .await
    .map_err(fail)?;
    Ok(json!({
        "data": page["data"],
        "nextCursor": page["nextCursor"],
        "inboxOptions": inbox_options,
        "agentOptions": agent_options,
        "showAgentFilter": show_agent_filter,
        "activity": query.get("activity"),
        "inboxId": query.get("inboxId"),
        "agentUserId": query.get("agentUserId"),
    }))
}
