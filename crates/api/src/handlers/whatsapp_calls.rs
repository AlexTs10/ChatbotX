//! The read routes of `features/whatsapp-calls/api/public.ts` (scope
//! `integrations`): `whatsappCalls.list` (`whatsappCallHistoryService.list`
//! with `member: "workspace"`), `getRecording`, `getTranscript`, `getSummary`,
//! `listSummaryProviders`, `generateSummary` (`generateCallSummaryForCall`).
//!
//! ponytail: the generate locks are in-process (`LockMap`, one API process),
//! like `email_marketing::run_exclusive`; use the KV lock when the API scales out.
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::whatsapp_call::{
    self as calls, ListForWorkspaceInput, WhatsappCallHistoryScope, WhatsappCallListCursor,
    WhatsappCallListFilters, WhatsappCallListRow,
};
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::contact_inboxes::lenient_base64;
use super::input::{Input, Str, key};
use super::tags::digits_at;
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, internal_error, json,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappCalls.list",
    "whatsappCalls.getRecording",
    "whatsappCalls.getTranscript",
    "whatsappCalls.getSummary",
    "whatsappCalls.listSummaryProviders",
    "whatsappCalls.generateSummary",
];

/// `aiProviders` with `PROVIDER_LABELS` and the integration table
/// (`listConnectedCallSummaryProviders`; `openaiCompatible` is left out).
pub(crate) const SUMMARY_PROVIDERS: [(&str, &str, &str); 6] = [
    ("openai", "OpenAI", "IntegrationOpenai"),
    ("gemini", "Gemini", "IntegrationGemini"),
    ("claude", "Claude", "IntegrationClaude"),
    ("deepseek", "DeepSeek", "IntegrationDeepseek"),
    ("openrouter", "OpenRouter", "IntegrationOpenrouter"),
    ("muse", "Muse", "IntegrationMuse"),
];

/// `DEFAULT_SUMMARY_MODELS`.
fn summary_model(provider: &str) -> &'static str {
    match provider {
        "gemini" => "gemini-3.5-flash",
        "claude" => "claude-sonnet-4-6",
        "deepseek" => "deepseek-flash",
        "openrouter" => "openai/gpt-5.4-mini",
        "muse" => "muse-spark-1.3",
        _ => "gpt-5.4-mini",
    }
}

/// `possibleErrorsOnGeneratingCallSummary`.
const GENERATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("callTranscriptEmpty", 422),
    ("callSummaryProviderNotConnected", 422),
    ("callSummaryProviderRequired", 422),
    ("summaryAlreadyGenerating", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
const PROVIDER_NOT_CONNECTED: &str = "callSummaryProviderNotConnected";

/// `possibleErrorsOnListingWithCursor`.
const LISTING_WITH_CURSOR: ErrorMap = &[("businessError", 400), ("invalidCursor", 400)];
const PAGE_SIZE: i64 = 25;
/// `RECORDING_SIGNED_URL_TTL_SECONDS`.
const RECORDING_URL_TTL_SECONDS: u64 = 15 * 60;

/// `resolveDisplayCallOutcome`.
fn display_outcome<'a>(status: &'a str, outcome: Option<&'a str>) -> Option<&'a str> {
    outcome.or(matches!(status, "completed" | "rejected" | "failed").then_some(status))
}

/// `resolveCallKind` (`CALL_KIND_RULES`, first match).
pub fn call_kind(status: &str, outcome: Option<&str>, direction: &str) -> Option<&'static str> {
    if matches!(status, "ringing" | "accepted") {
        return Some("ongoing");
    }
    let inbound = direction == "userInitiated";
    Some(match (display_outcome(status, outcome)?, inbound) {
        ("canceled", _) => "canceled",
        ("rejected", _) => "declined",
        ("failed", true) => "missed",
        ("failed", false) if direction == "businessInitiated" => "unanswered",
        ("completed", true) => "answeredInbound",
        ("completed", false) if direction == "businessInitiated" => "answeredOutbound",
        _ => return None,
    })
}

pub(crate) fn row_json(row: &WhatsappCallListRow) -> Value {
    let call = &row.call;
    let user = |u: &Option<calls::CallUser>| {
        u.as_ref().map_or(
            Value::Null,
            |u| json!({ "id": u.id.to_string(), "name": u.name }),
        )
    };
    json!({
        "id": call.id.to_string(),
        "createdAt": chatbotx_db::repo::iso(call.created_at),
        "direction": call.direction,
        "status": call.status,
        "outcome": call.outcome,
        "kind": call_kind(&call.status, call.outcome.as_deref(), &call.direction),
        "durationSeconds": call.duration_seconds,
        "hasRecording": call.recording_path.as_deref().is_some_and(|p| !p.is_empty()),
        "conversationId": call.conversation_id.to_string(),
        "contact": {
            "id": row.contact.id.to_string(),
            "fullName": row.contact.full_name,
            "avatar": row.contact.avatar,
        },
        "inbox": { "id": row.inbox_id.to_string(), "name": row.inbox_name },
        "answeredByUser": user(&row.answered_by_user),
        "initiatedByUser": user(&row.initiated_by_user),
    })
}

/// `decodeCursor(cursor, whatsappCallListCursorSchema)`.
fn decode_cursor(text: &str) -> Option<WhatsappCallListCursor> {
    let value: Value = serde_json::from_slice(&lenient_base64(text)?).ok()?;
    let created_at = value.get("createdAt")?.as_str()?;
    chatbotx_db::repo::parse_iso(created_at)?;
    let id = value.get("id")?.as_str()?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(WhatsappCallListCursor {
        created_at: created_at.to_owned(),
        id: id.parse().ok()?,
    })
}

fn encode_cursor(cursor: &WhatsappCallListCursor) -> String {
    STANDARD
        .encode(json!({ "createdAt": cursor.created_at, "id": cursor.id.to_string() }).to_string())
}

fn call_not_found() -> BusinessError {
    BusinessError::not_found("Call not found")
}

/// `loadCallForWorkspace`.
async fn load_call(state: &ApiState, ws: i64, id: i64) -> Result<WhatsappCall, BusinessError> {
    tx::read(&state.db, move |c| {
        Ok(calls::find_by_id_for_workspace(c, id, ws)?)
    })
    .await?
    .filter(|call| call.workspace_id == ws)
    .ok_or_else(call_not_found)
}

/// `resolveSpeakerNames`: the agent's name (else the workspace's) and the
/// customer's name (else phone).
async fn speaker_names(state: &ApiState, call: &WhatsappCall) -> Result<Value, BusinessError> {
    let agent = if call.direction == "businessInitiated" {
        call.initiated_by_user_id
    } else {
        call.answered_by_user_id
    };
    let (ws, contact_inbox) = (call.workspace_id, call.contact_inbox_id);
    tx::read(&state.db, move |c| {
        let agent_name: Option<String> = match agent {
            Some(id) => c
                .query_row(
                    r#"SELECT "name" FROM "User" WHERE "id" = ?1"#,
                    params![id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten(),
            None => None,
        };
        let business = match agent_name.filter(|n: &String| !n.is_empty()) {
            Some(name) => name,
            None => c
                .query_row(
                    r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
                    params![ws],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default(),
        };
        let customer: Option<(Option<String>, Option<String>)> = c
            .query_row(
                r#"SELECT ct."fullName", ct."phoneNumber" FROM "ContactInbox" ci
                   JOIN "Contact" ct ON ct."id" = ci."contactId" AND ct."workspaceId" = ?2
                   WHERE ci."id" = ?1"#,
                params![contact_inbox, ws],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let customer = customer
            .and_then(|(name, phone)| name.or(phone))
            .unwrap_or_default();
        Ok(json!({ "business": business, "customer": customer }))
    })
    .await
}

fn id_input(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, call.query);
    let value = input.get("id").cloned();
    let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
    input.finish()?;
    Ok(id)
}

pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "whatsappCalls.list" => list(state, call).await,
        "whatsappCalls.getRecording" => {
            let id = id_input(&call)?;
            recording_url(state, ws, id, FINDING)
                .await
                .map(|url| json(200, json!({ "url": url })))
        }
        "whatsappCalls.getTranscript" => {
            let id = id_input(&call)?;
            transcript(state, ws, id)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappCalls.listSummaryProviders" => providers_json(state, ws)
            .await
            .map(|providers| json(200, json!({ "providers": providers })))
            .map_err(|e| business_error(e, LISTING)),
        "whatsappCalls.generateSummary" => generate_summary(state, call).await,
        _ => {
            let id = id_input(&call)?;
            load_call(state, ws, id)
                .await
                .map(|c| {
                    let summary = c.ai_summary.map(|s| s.0).filter(|s| !s.is_null());
                    let provider = summary.as_ref().and(c.ai_summary_provider);
                    json(200, json!({ "summary": summary, "provider": provider }))
                })
                .map_err(|e| business_error(e, FINDING))
        }
    }
}

/// `callRecordingService.getRecordingUrlForCall`: a 15-minute signed URL.
pub(crate) async fn recording_url(
    state: &ApiState,
    ws: i64,
    id: i64,
    errors: ErrorMap,
) -> Result<String, crate::orpc::OrpcError> {
    let path = tx::read(&state.db, move |c| {
        Ok(calls::find_by_id_for_workspace(c, id, ws)?)
    })
    .await
    .map_err(|e| business_error(e, errors))?
    .and_then(|c| c.recording_path)
    .filter(|p| !p.is_empty())
    .ok_or_else(|| business_error(BusinessError::not_found("Call recording not found"), errors))?;
    let files = state
        .files
        .as_ref()
        .ok_or_else(|| internal_error("file store is off"))?;
    files
        .get_presigned_download(&path, Some(RECORDING_URL_TTL_SECONDS))
        .await
        .map_err(|e| internal_error(&e.to_string()))
}

/// `whatsappCallTranscriptService.getTranscriptForCall`.
pub(crate) async fn transcript(state: &ApiState, ws: i64, id: i64) -> Result<Value, BusinessError> {
    let found = load_call(state, ws, id).await?;
    let segments: Vec<Value> = found
        .transcript_segments
        .as_ref()
        .and_then(|s| s.0.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|s| json!({ "speaker": s["speaker"], "start": s["start"], "end": s["end"], "text": s["text"] }))
        .collect();
    let has_speakers = segments
        .iter()
        .any(|s| s["speaker"].as_str().is_some_and(|v| !v.is_empty()));
    let names = speaker_names(state, &found).await?;
    Ok(json!({ "segments": segments, "speakerNames": names, "hasSpeakers": has_speakers }))
}

/// `listConnectedCallSummaryProviders` as JSON options.
pub(crate) async fn providers_json(state: &ApiState, ws: i64) -> Result<Vec<Value>, BusinessError> {
    Ok(connected_providers(state, ws)
        .await?
        .iter()
        .map(|(id, provider, label)| json!({ "id": id.to_string(), "provider": provider, "label": label }))
        .collect())
}

/// `whatsappCallSummaryService.getSummaryForCall`: `None` = no summary yet.
pub(crate) async fn summary_for_call(
    state: &ApiState,
    ws: i64,
    id: i64,
) -> Result<Option<Value>, BusinessError> {
    let found = load_call(state, ws, id).await?;
    Ok(found.ai_summary.map(|s| s.0).filter(|s| !s.is_null()).map(
        |summary| json!({ "aiSummary": summary, "aiSummaryProvider": found.ai_summary_provider }),
    ))
}

/// `listConnectedCallSummaryProviders`: (integration id, provider, label).
async fn connected_providers(
    state: &ApiState,
    ws: i64,
) -> Result<Vec<(i64, &'static str, &'static str)>, BusinessError> {
    tx::read(&state.db, move |c| {
        let mut providers = Vec::new();
        for (provider, label, table) in SUMMARY_PROVIDERS {
            let id: Option<i64> = c
                .query_row(
                    &format!(
                        r#"SELECT "id" FROM "{table}" WHERE "workspaceId" = ?1 ORDER BY "id" LIMIT 1"#
                    ),
                    params![ws],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = id {
                providers.push((id, provider, label));
            }
        }
        Ok(providers)
    })
    .await
}

fn summary_locks() -> &'static chatbotx_kv::lock::LockMap {
    static LOCKS: std::sync::OnceLock<chatbotx_kv::lock::LockMap> = std::sync::OnceLock::new();
    LOCKS.get_or_init(chatbotx_kv::lock::LockMap::new)
}

/// A fail-fast lock (`retryTimeoutInSeconds: 0`); a held key is `summaryAlreadyGenerating`.
fn summary_lock(key: &str) -> Result<chatbotx_kv::lock::LockGuard, BusinessError> {
    summary_locks().try_acquire(key).map_err(|_| {
        BusinessError::conflict(
            "A summary is already being generated for this call.",
            "summaryAlreadyGenerating",
        )
    })
}

async fn generate_summary(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let value = input.get("id").cloned();
    let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
    let provider = input.get("provider").cloned();
    let names: Vec<&str> = SUMMARY_PROVIDERS.iter().map(|p| p.0).collect();
    let provider = input.enum_at(&key("provider"), provider.as_ref(), &names, false);
    input.finish()?;
    generate_call_summary(state, call.workspace_id, id, provider)
        .await
        .map(|summary| json(200, json!({ "summary": summary })))
        .map_err(|e| business_error(e, GENERATING))
}

/// `generateCallSummaryForCall`: transcript, provider, the 120 s generate
/// lock, `generateCallSummary`, `attachSummary`.
pub(crate) async fn generate_call_summary(
    state: &ApiState,
    ws: i64,
    id: i64,
    provider: Option<String>,
) -> Result<Value, BusinessError> {
    let found = load_call(state, ws, id).await?;
    let text = transcript_text(&found);
    if text.trim().is_empty() {
        return Err(BusinessError::exception(
            "This call has no transcript to summarize",
            "callTranscriptEmpty",
            422,
        ));
    }
    let provider = match provider {
        Some(provider) => provider,
        None => only_connected_provider(state, ws).await?,
    };
    let _generating = summary_lock(&format!("whatsapp-call-summary-generate:{id}"))?;
    let summary = summarize(state, ws, &provider, &text).await?;
    attach_summary(state, &found, summary.clone(), provider).await?;
    Ok(summary)
}

/// `getTranscriptTextForCall`: the flat transcript, else the joined segments.
fn transcript_text(call: &WhatsappCall) -> String {
    if let Some(text) = call.transcript.as_deref().filter(|t| !t.is_empty()) {
        return text.to_owned();
    }
    call.transcript_segments
        .as_ref()
        .and_then(|s| s.0.as_array())
        .map(|segments| {
            segments
                .iter()
                .map(|s| s["text"].as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// `resolveOnlyConnectedProvider`.
async fn only_connected_provider(state: &ApiState, ws: i64) -> Result<String, BusinessError> {
    let connected = connected_providers(state, ws).await?;
    match connected.as_slice() {
        [(_, only, _)] => Ok((*only).to_owned()),
        [] => Err(BusinessError::exception(
            "No AI integration is connected to this workspace",
            PROVIDER_NOT_CONNECTED,
            422,
        )),
        many => Err(BusinessError::exception(
            format!(
                "Several AI providers are connected ({}): pass `provider`",
                many.iter().map(|p| p.1).collect::<Vec<_>>().join(", ")
            ),
            "callSummaryProviderRequired",
            422,
        )),
    }
}

/// `generateCallSummary` (`call-summarizer.ts`): one structured-output call.
async fn summarize(
    state: &ApiState,
    ws: i64,
    provider: &str,
    transcript: &str,
) -> Result<Value, BusinessError> {
    use chatbotx_business::ai::{llm, model};
    let name = provider.to_owned();
    let auth = tx::read(&state.db, move |c| {
        Ok(model::native_auth(c, ws, &name, None)?)
    })
    .await?
    .ok_or_else(|| {
        BusinessError::exception(
            format!("The {provider} integration is not connected to this workspace"),
            PROVIDER_NOT_CONNECTED,
            422,
        )
    })?;
    let failed = |message: String| {
        tracing::error!(workspace_id = ws, provider, %message, "[call-summarizer] Failed to generate call summary");
        BusinessError::exception("Internal server error", "INTERNAL_SERVER_ERROR", 500)
    };
    let model = model::native_model(provider, &auth, summary_model(provider)).map_err(failed)?;
    let prompt = [
        "Summarize the following phone call transcript for a customer support agent.",
        "Provide a concise summary, a short list of key discussion points, and any concrete action items.",
        "If the transcript has no clear key points or action items, return null for that field rather than inventing one.",
        "",
        transcript,
    ]
    .join("\n");
    let request = llm::Request::new(None, vec![llm::Turn::user(prompt)]);
    let object = llm::generate_object(
        &llm::ReqwestHttp::default(),
        &model,
        &request,
        summary_schema(),
    )
    .await
    .map_err(|e| failed(e.message))?;
    parse_summary(&object)
        .ok_or_else(|| failed("No object generated: response did not match schema.".into()))
}

/// `callSummarySchema` (nullable lists, as OpenAI strict mode needs).
fn summary_schema() -> Value {
    let list = json!({ "type": ["array", "null"], "items": { "type": "string" } });
    json!({
        "type": "object",
        "properties": { "summary": { "type": "string" }, "keyPoints": list, "actionItems": list },
        "required": ["summary", "keyPoints", "actionItems"],
        "additionalProperties": false,
    })
}

/// The schema check; a `null` list is left out.
fn parse_summary(object: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut out = serde_json::Map::new();
    out.insert(
        "summary".into(),
        Value::String(object.get("summary")?.as_str()?.to_owned()),
    );
    for field in ["keyPoints", "actionItems"] {
        match object.get(field) {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
                out.insert(field.into(), Value::Array(items.clone()));
            }
            Some(_) => return None,
        }
    }
    Some(Value::Object(out))
}

/// `whatsappCallSummaryService.attachSummary`: first write or overwrite under
/// its own 60 s lock, then `hasSummary` on the call card + `messageContentUpdated`.
async fn attach_summary(
    state: &ApiState,
    call: &WhatsappCall,
    summary: Value,
    provider: String,
) -> Result<(), BusinessError> {
    let _writing = summary_lock(&format!("whatsapp-call-summary:{}", call.id))?;
    let input = calls::AiSummaryInput {
        id: call.id,
        ai_summary: summary,
        ai_summary_provider: provider,
        ai_summarized_at: None,
    };
    let overwrite = call.ai_summarized_at.is_some();
    let (ws, conversation_id, source) = (
        call.workspace_id,
        call.conversation_id,
        format!("wacall-{}", call.id),
    );
    let (merged, route) = tx::write(&state.db, move |c| {
        if overwrite {
            calls::overwrite_ai_summary(c, &input)?;
        } else {
            calls::attach_ai_summary(c, &input)?;
        }
        let mut overlay = serde_json::Map::new();
        overlay.insert("hasSummary".into(), Value::Bool(true));
        let merged =
            chatbotx_db::repo::message::merge_content_attributes_by_source_id(c, &source, ws, &overlay)?;
        let route: Option<(Option<i64>, Option<i64>)> = c
            .query_row(
                r#"SELECT "assignedInboxTeamId", "assignedUserId" FROM "Conversation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![conversation_id, ws],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok((merged, route))
    })
    .await?;
    let Some(merged) = merged else {
        tracing::info!(
            call_id = call.id,
            "Whatsapp call: no finalize message to enrich with AI summary yet"
        );
        return Ok(());
    };
    let Some(entity) = merged
        .content_attributes
        .filter(|e| e["type"] == "whatsapp_call")
    else {
        return Ok(());
    };
    let route = route.map(|(team, user)| chatbotx_rt::protocol::ConversationRoute {
        assigned_inbox_team_id: team.map(|i| i.to_string()),
        assigned_user_id: user.map(|i| i.to_string()),
        inbox_id: None,
    });
    let mut event = chatbotx_rt::Envelope::new(
        chatbotx_rt::EventType::MessageContentUpdated,
        json!({ "messageId": merged.id.to_string(), "contentAttributes": entity }),
    );
    event.route = Some(chatbotx_rt::protocol::route_for_conversation(
        route.as_ref(),
        None,
    ));
    state.queue_realtime(ws, event).await;
    Ok(())
}

async fn list(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let activity = input.get("activity").cloned();
    let activity = input.enum_at(
        &key("activity"),
        activity.as_ref(),
        &["missed", "noReply"],
        false,
    );
    let inbox = input.get("inboxId").cloned();
    let inbox_id = digits_at(&mut input, &key("inboxId"), inbox.as_ref(), false);
    let agent = input.get("agentUserId").cloned();
    let agent_user_id = digits_at(&mut input, &key("agentUserId"), agent.as_ref(), false);
    let cursor = input.opt_string("cursor", Str::default());
    input.finish()?;
    let filter = ListFilter {
        activity,
        inbox_id,
        agent_user_id,
        cursor,
    };
    let (rows, next) = list_calls(
        state,
        call.workspace_id,
        WhatsappCallHistoryScope::AllCalls,
        filter,
    )
    .await?;
    Ok(json(
        200,
        json!({ "data": rows.iter().map(row_json).collect::<Vec<_>>(), "nextCursor": next }),
    ))
}

/// The list inputs of `whatsappCallHistoryService.list`.
pub(crate) struct ListFilter {
    pub activity: Option<String>,
    pub inbox_id: Option<i64>,
    pub agent_user_id: Option<i64>,
    pub cursor: Option<String>,
}

/// `whatsappCallHistoryService.list`: one page and the next cursor.
pub(crate) async fn list_calls(
    state: &ApiState,
    workspace_id: i64,
    scope: WhatsappCallHistoryScope,
    filter: ListFilter,
) -> Result<(Vec<WhatsappCallListRow>, Option<String>), crate::orpc::OrpcError> {
    let cursor = match filter.cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(text) => Some(decode_cursor(text).ok_or_else(|| {
            business_error(
                BusinessError::exception(
                    "Whatsapp calls: cursor failed to decode",
                    "invalidCursor",
                    400,
                ),
                LISTING_WITH_CURSOR,
            )
        })?),
        None => None,
    };
    // `CALL_ACTIVITY_FILTERS`.
    let direction = match filter.activity.as_deref() {
        Some("missed") => Some("userInitiated".to_owned()),
        Some(_) => Some("businessInitiated".to_owned()),
        None => None,
    };
    let filters = WhatsappCallListFilters {
        outcome: filter.activity.as_ref().map(|_| "failed".to_owned()),
        direction,
        inbox_id: filter.inbox_id,
        agent_user_id: filter.agent_user_id,
        ongoing: false,
    };
    let query = ListForWorkspaceInput {
        workspace_id,
        scope,
        filters,
        cursor,
        limit: PAGE_SIZE + 1,
    };
    let mut rows = tx::read(&state.db, move |c| {
        Ok(calls::list_for_workspace(c, &query)?)
    })
    .await
    .map_err(|e| business_error(e, LISTING_WITH_CURSOR))?;
    let has_more = rows.len() as i64 > PAGE_SIZE;
    rows.truncate(PAGE_SIZE as usize);
    let next = rows.last().filter(|_| has_more).map(|row| {
        encode_cursor(&WhatsappCallListCursor {
            created_at: row.created_at_cursor.clone(),
            id: row.call.id,
        })
    });
    Ok((rows, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_kinds_follow_the_rule_order() {
        assert_eq!(call_kind("ringing", None, "userInitiated"), Some("ongoing"));
        assert_eq!(call_kind("failed", None, "userInitiated"), Some("missed"));
        assert_eq!(
            call_kind("failed", None, "businessInitiated"),
            Some("unanswered")
        );
        assert_eq!(
            call_kind("completed", Some("canceled"), "userInitiated"),
            Some("canceled")
        );
        assert_eq!(
            call_kind("completed", None, "businessInitiated"),
            Some("answeredOutbound")
        );
        assert_eq!(
            call_kind("rejected", None, "userInitiated"),
            Some("declined")
        );
    }

    #[test]
    fn a_summary_object_must_match_the_schema() {
        let object = |v: Value| v.as_object().cloned().unwrap();
        assert_eq!(
            parse_summary(&object(
                json!({ "summary": "s", "keyPoints": null, "actionItems": ["a"] })
            )),
            Some(json!({ "summary": "s", "actionItems": ["a"] }))
        );
        assert_eq!(parse_summary(&object(json!({ "keyPoints": [] }))), None);
        assert_eq!(
            parse_summary(&object(json!({ "summary": "s", "keyPoints": [1] }))),
            None
        );
    }

    #[test]
    fn the_cursor_round_trips_and_rejects_garbage() {
        let cursor = WhatsappCallListCursor {
            created_at: "2026-11-18T01:02:03.456Z".into(),
            id: 42,
        };
        assert_eq!(decode_cursor(&encode_cursor(&cursor)), Some(cursor));
        assert_eq!(decode_cursor("nope"), None);
        assert_eq!(
            decode_cursor(&STANDARD.encode(r#"{"createdAt":"x","id":"1"}"#)),
            None
        );
    }
}
