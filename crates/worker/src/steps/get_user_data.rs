//! `integration/handlers/get-user-data/{handler,challenge-cycle,prompt}.ts`:
//! the Get User Data step. The first run asks the question and writes the
//! step challenge; the reply (`runChallenge`), the Skip tap / timer
//! (`resolveStepInput`) or a webview submit re-enters it. Every outcome
//! claims the challenge first, so only one of them ever completes the step.

use chatbotx_business::contact_custom_field::{self, SetValueByKeyInput};
use chatbotx_business::conversation_challenge::{self as challenges, AdvanceAttempt};
use chatbotx_business::get_user_data::{
    ReplyInputAttachment, ReplyInputKind, ReplyInputMessage, ReplyValidation, validate_reply_input,
};
use chatbotx_core::flow_config::step_input_resolution::{
    STEP_INPUT_RESOLUTION_PAYLOAD_TYPE, encode_step_input_skip_postback,
};
use chatbotx_core::flow_config::steps::get_user_data::encode_contact_hint_quick_reply_payload;
use chatbotx_core::utils::id::create_id;
use chatbotx_db::repo::message::{
    FindLastByConversationOptions, FindTriggerMessageOptions, MessageWithAttachments,
    SinceTimeInput, find_last_by_conversation, find_trigger_message, get_safe_since_time,
};
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_jobs::chat_wait::{
    ChatWaitContext, SEND_CHAT_MESSAGE_ACTION, wait_for_chat_job_completion,
};
use chatbotx_jobs::{JobError, JobOptions, QueueName, WaitTarget};
use serde_json::{Map, Value, json};

use super::{StepCtx, step_error};
use crate::challenge::{STEP_CHALLENGE, id_text, pending_step_challenge};
use crate::flow::StepStatus;
use crate::{db_fail, id_of, ms_of, str_of};

/// `TRIGGER_MESSAGE_LOOKBACK_MS`.
const TRIGGER_MESSAGE_LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;
/// `GET_USER_DATA_WEBVIEW_SELECTION_PAYLOAD_TYPE`.
const WEBVIEW_SELECTION: &str = "getUserDataWebviewSelection";
/// `URL_QUICK_REPLY_CAPABLE_CHANNELS`.
const URL_QUICK_REPLY_CHANNELS: &[&str] = &["messenger", "telegram"];
/// `NATIVE_LOCATION_REQUEST_CHANNELS`.
const NATIVE_LOCATION_REQUEST_CHANNELS: &[&str] = &["whatsapp"];
/// `WHATSAPP_NATIVE_LOCATION_REQUEST`.
const WHATSAPP_NATIVE_LOCATION_REQUEST: &str = "whatsapp:native:location_request";
/// `CONTACT_HINT_QUICK_REPLY_CHANNELS`.
const CONTACT_HINT_CHANNELS: &[&str] = &["messenger", "instagram"];
/// Meta caps a quick reply title at 20 characters.
const QUICK_REPLY_TITLE_MAX_LENGTH: usize = 20;

/// Why the step failed: a transient error retries the job with the
/// challenge intact (`isMessageStorageError`); any other fails the step.
enum Fail {
    Job(JobError),
    Step(String),
}

impl From<JobError> for Fail {
    fn from(e: JobError) -> Fail {
        Fail::Step(e.message)
    }
}

fn step_fail(e: impl std::fmt::Display) -> Fail {
    Fail::Step(e.to_string())
}

type Outcome = Result<StepStatus, Fail>;

// ---- the run's view of the step --------------------------------------------

fn text_of(step: &Value, key: &str, default: &str) -> String {
    str_of(step, key).unwrap_or(default).to_owned()
}

fn number_of(step: &Value, key: &str, default: i64) -> i64 {
    match &step[key] {
        Value::Number(n) => n.as_f64().map_or(default, |n| n as i64),
        Value::String(s) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

fn reply_format(ctx: &StepCtx<'_, '_>) -> String {
    text_of(ctx.step, "replyFormat", "RF02")
}

fn step_id(ctx: &StepCtx<'_, '_>) -> String {
    id_text(&ctx.step["id"]).unwrap_or_default()
}

fn auto_skip(ctx: &StepCtx<'_, '_>) -> bool {
    ctx.step["autoSkip"] == json!(true)
}

/// `hasSkipButton`.
pub fn has_skip_button(step: &Value) -> bool {
    !skip_label(step).is_empty()
}

fn skip_label(step: &Value) -> String {
    chatbotx_core::js::trim(str_of(step, "skipButtonLabel").unwrap_or("Skip")).to_owned()
}

/// `add(origin, { [autoSkipTimeUnit]: autoSkipTimeValue })` in ms.
fn auto_skip_window_ms(step: &Value) -> i64 {
    let unit = match str_of(step, "autoSkipTimeUnit").unwrap_or("minutes") {
        "seconds" => 1_000,
        "hours" => 3_600_000,
        "days" => 86_400_000,
        _ => 60_000,
    };
    unit * number_of(step, "autoSkipTimeValue", 3)
}

/// The challenge variables of the run (`ctx.variables.conversation`).
fn variables<'v>(ctx: &'v StepCtx<'_, '_>) -> Option<&'v Value> {
    ctx.run.data.get(STEP_CHALLENGE).filter(|v| v.is_object())
}

/// `readChallengeAttempts` (1 when absent).
fn challenge_attempts(ctx: &StepCtx<'_, '_>) -> i64 {
    variables(ctx)
        .and_then(|v| v["attempts"].as_i64())
        .unwrap_or(1)
}

/// `readRunChallengeId`.
fn run_challenge_id(ctx: &StepCtx<'_, '_>) -> Option<String> {
    variables(ctx).and_then(|v| v["challengeId"].as_str().map(str::to_owned))
}

/// `readPendingStepChallenge`: the step challenge on the conversation this
/// run loaded.
fn pending(ctx: &StepCtx<'_, '_>) -> Option<Value> {
    pending_step_challenge(&ctx.run.conversation)
}

fn pending_is_this_step(ctx: &StepCtx<'_, '_>, pending: &Value) -> bool {
    id_text(&pending["data"]["stepId"]).as_deref() == Some(step_id(ctx).as_str())
}

/// `isRunForPendingCycle`.
fn is_run_for_pending_cycle(ctx: &StepCtx<'_, '_>, pending: &Value) -> bool {
    match run_challenge_id(ctx) {
        None => true,
        Some(id) => id_text(&pending["data"]["challengeId"]).as_deref() == Some(id.as_str()),
    }
}

/// `resolveChallengeNodeId`.
fn node_id(ctx: &StepCtx<'_, '_>) -> Result<String, Fail> {
    let node = &ctx.run.target_id;
    if node.is_empty() {
        return Err(Fail::Step(format!(
            "getUserData: missing target node for conversation {}",
            ctx.run.conversation.id
        )));
    }
    Ok(node.clone())
}

fn appointment_id(ctx: &StepCtx<'_, '_>) -> Option<Value> {
    ctx.run
        .data
        .get("appointmentId")
        .filter(|v| !v.is_null())
        .cloned()
}

/// The `ConversationStepChallenge` this step writes.
fn challenge_value(
    ctx: &StepCtx<'_, '_>,
    attempts: i64,
    challenge_id: Option<&str>,
    started_at: Option<i64>,
) -> Result<Value, Fail> {
    let mut data = Map::new();
    data.insert(
        "flowId".into(),
        json!(ctx.run.flow_version.flow_id.to_string()),
    );
    if let Some(version) = ctx.run.version_id() {
        data.insert("flowVersionId".into(), json!(version));
    }
    data.insert("nodeId".into(), json!(node_id(ctx)?));
    data.insert("stepId".into(), json!(step_id(ctx)));
    data.insert("attempts".into(), json!(attempts));
    data.insert("lastAttemptAt".into(), json!(iso(now_ms())));
    if let Some(at) = started_at {
        data.insert("startedAt".into(), json!(iso(at)));
    }
    if let Some(appointment) = appointment_id(ctx) {
        data.insert("appointmentId".into(), appointment);
    }
    if let Some(id) = challenge_id {
        data.insert("challengeId".into(), json!(id));
    }
    Ok(json!({ "type": "step", "data": data }))
}

// ---- the challenge cycle (`challenge-cycle.ts`) -----------------------------

/// `writeChallenge`.
async fn write_challenge(
    ctx: &StepCtx<'_, '_>,
    attempts: i64,
    challenge_id: &str,
    started_at: Option<i64>,
) -> Result<(), Fail> {
    let challenge = challenge_value(ctx, attempts, Some(challenge_id), started_at)?;
    challenges::update_challenge(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        ctx.run.conversation.id,
        Some(challenge),
    )
    .await
    .map_err(step_fail)
}

/// `armInputTimeout`: one pending timer per conversation, from the first
/// prompt.
async fn arm_input_timeout(
    ctx: &StepCtx<'_, '_>,
    challenge_id: &str,
    started_at: i64,
) -> Result<(), Fail> {
    let run = ctx.run;
    let row = chatbotx_db::repo::models::ContactOnSmartDelay {
        id: chatbotx_db::repo::new_id(),
        workspace_id: ctx.workspace_id(),
        flow_id: run.flow_version.flow_id,
        flow_version_id: run.version_id().and_then(|v| v.parse().ok()),
        contact_inbox_id: run.contact_inbox.id,
        conversation_id: run.conversation.id,
        node_id: Some(node_id(ctx)?),
        step_id: Some(step_id(ctx)),
        metadata: Some(chatbotx_db::repo::Json(json!({
            "type": STEP_INPUT_RESOLUTION_PAYLOAD_TYPE,
            "stepId": step_id(ctx),
            "challengeId": challenge_id,
            "reason": "timeout",
        }))),
        r#type: "stepInputTimeout".into(),
        created_at: now_ms(),
        trigger_at: started_at + auto_skip_window_ms(ctx.step),
        status: "pending".into(),
        appointment_id: appointment_id(ctx).as_ref().and_then(id_of),
    };
    let extras = crate::smart_delay::ResumeExtras {
        appointment_id: row.appointment_id,
        ..Default::default()
    };
    crate::smart_delay::schedule_smart_delay_resume(run.deps, row, extras)
        .await
        .map_err(|e| Fail::Step(e.message))
}

/// `advanceChallengeAttempt`: the challenge id for the one winner of two
/// concurrent invalid replies; `None` for the loser.
async fn advance_challenge_attempt(
    ctx: &StepCtx<'_, '_>,
    from: i64,
) -> Result<Option<String>, Fail> {
    let Some(pending) =
        pending(ctx).filter(|p| pending_is_this_step(ctx, p) && is_run_for_pending_cycle(ctx, p))
    else {
        return Ok(None);
    };
    if let Some(challenge_id) = id_text(&pending["data"]["challengeId"]) {
        let advanced = challenges::advance_step_challenge_attempt(
            &ctx.run.deps.db,
            AdvanceAttempt {
                workspace_id: ctx.workspace_id(),
                conversation_id: ctx.run.conversation.id,
                step_id: step_id(ctx),
                challenge_id: challenge_id.clone(),
                from_attempts: from,
                to_attempts: from + 1,
                last_attempt_at: now_ms(),
            },
        )
        .await
        .map_err(step_fail)?;
        return Ok(advanced.then_some(challenge_id));
    }
    // A challenge written before ids existed is upgraded in place.
    let challenge_id = create_id();
    write_challenge(ctx, from + 1, &challenge_id, None).await?;
    Ok(Some(challenge_id))
}

async fn clear_challenge(ctx: &StepCtx<'_, '_>) -> Result<(), String> {
    challenges::update_challenge(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        ctx.run.conversation.id,
        None,
    )
    .await
    .map_err(|e| e.to_string())
}

async fn consume(ctx: &StepCtx<'_, '_>, challenge_id: &str) -> Result<bool, String> {
    challenges::consume_challenge(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        ctx.run.conversation.id,
        &step_id(ctx),
        challenge_id,
    )
    .await
    .map_err(|e| e.to_string())
}

/// `consumeCurrentChallenge`: the atomic claim of the pending challenge (a
/// legacy id-less one is cleared unconditionally).
async fn consume_current_challenge(ctx: &StepCtx<'_, '_>) -> Result<bool, String> {
    let Some(current) =
        pending(ctx).filter(|p| pending_is_this_step(ctx, p) && is_run_for_pending_cycle(ctx, p))
    else {
        return Ok(false);
    };
    match id_text(&current["data"]["challengeId"]) {
        None => clear_challenge(ctx).await.map(|()| true),
        Some(id) => consume(ctx, &id).await,
    }
}

/// `clearChallengeOwnedByRun`: `false` when the cycle was already settled or
/// replaced; a failing claim retries the job.
async fn clear_challenge_owned_by_run(
    ctx: &StepCtx<'_, '_>,
    owned: Option<&str>,
) -> Result<bool, JobError> {
    let Some(owned) = owned else {
        if let Err(err) = clear_challenge(ctx).await {
            tracing::warn!(err = %err, conversation_id = ctx.run.conversation.id, "getUserData: failed to clear challenge after terminal error");
        }
        return Ok(true);
    };
    match consume(ctx, owned).await {
        Ok(cleared) => {
            if !cleared {
                tracing::info!(
                    conversation_id = ctx.run.conversation.id,
                    "getUserData: the challenge was already settled or replaced, not reporting the failure"
                );
            }
            Ok(cleared)
        }
        Err(err) => {
            tracing::warn!(err = %err, conversation_id = ctx.run.conversation.id, "getUserData: failed to clear challenge after terminal error, retrying the job");
            Err(JobError::retryable(err))
        }
    }
}

/// `restoreChallengeAfterFailedClaim`: puts the consumed challenge back
/// (only while none exists) and re-arms its timer; `true` when pending again.
async fn restore_challenge_after_failed_claim(
    ctx: &StepCtx<'_, '_>,
    challenge_id: Option<&str>,
) -> bool {
    let restored = async {
        let challenge = match pending(ctx).filter(|p| {
            pending_is_this_step(ctx, p)
                && id_text(&p["data"]["challengeId"]).as_deref() == challenge_id
        }) {
            Some(p) => p,
            None => challenge_value(ctx, 1, challenge_id, None).map_err(|f| match f {
                Fail::Step(m) => m,
                Fail::Job(e) => e.message,
            })?,
        };
        let restored = challenges::restore_challenge_if_absent(
            &ctx.run.deps.db,
            ctx.workspace_id(),
            ctx.run.conversation.id,
            challenge.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok::<_, String>((restored, challenge))
    }
    .await;
    match restored {
        Ok((restored, challenge)) => {
            tracing::info!(
                conversation_id = ctx.run.conversation.id,
                restored,
                "getUserData: challenge restore after a side-effect failure"
            );
            if restored {
                rearm_restored_timeout(ctx, &challenge).await;
            }
            restored
        }
        Err(err) => {
            tracing::warn!(err = %err, conversation_id = ctx.run.conversation.id, "getUserData: failed to restore challenge after a side-effect failure");
            false
        }
    }
}

/// `rearmRestoredTimeout` (best effort).
async fn rearm_restored_timeout(ctx: &StepCtx<'_, '_>, challenge: &Value) {
    let data = &challenge["data"];
    let (Some(id), Some(started_at)) = (id_text(&data["challengeId"]), ms_of(&data["startedAt"]))
    else {
        return;
    };
    if !auto_skip(ctx) {
        return;
    }
    if arm_input_timeout(ctx, &id, started_at).await.is_err() {
        tracing::warn!(
            conversation_id = ctx.run.conversation.id,
            "getUserData: could not re-arm the timeout of a restored challenge"
        );
    }
}

/// How an outcome claims its challenge.
enum Claim {
    /// The conversation's pending challenge.
    Pending,
    /// A webview token's own challenge.
    Token(String),
}

/// `settleChallenge`: claim first, then the side effects. `None` = won and
/// applied; else the status to report (`wait` lost / restored, `error`).
async fn settle(
    ctx: &StepCtx<'_, '_>,
    claim: Claim,
    side_effects: impl AsyncFnOnce() -> Result<(), String>,
) -> Result<Option<StepStatus>, Fail> {
    let (challenge_id, claimed) = match claim {
        Claim::Pending => (
            pending(ctx).and_then(|p| id_text(&p["data"]["challengeId"])),
            consume_current_challenge(ctx).await,
        ),
        Claim::Token(id) => {
            let claimed = consume(ctx, &id).await;
            (Some(id), claimed)
        }
    };
    if !claimed.map_err(Fail::Step)? {
        tracing::info!(
            conversation_id = ctx.run.conversation.id,
            "getUserData: challenge already settled or replaced, not completing"
        );
        return Ok(Some(StepStatus::Wait));
    }
    if let Err(err) = side_effects().await {
        tracing::error!(err = %err, conversation_id = ctx.run.conversation.id, "getUserData: side effect failed after claiming the challenge");
        return Ok(Some(
            if restore_challenge_after_failed_claim(ctx, challenge_id.as_deref()).await {
                StepStatus::Wait
            } else {
                step_error(ctx, &err)
            },
        ));
    }
    Ok(None)
}

// ---- the prompt (`prompt.ts`) -----------------------------------------------

/// `toQuickReplyTitle`: at most 20 code points (an ellipsis when cut).
pub fn to_quick_reply_title(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() > QUICK_REPLY_TITLE_MAX_LENGTH {
        let mut title: String = chars[..QUICK_REPLY_TITLE_MAX_LENGTH - 1].iter().collect();
        title.push('…');
        title
    } else {
        value.to_owned()
    }
}

fn postback(id: &str, label: &str, payload: &str) -> Value {
    json!({ "id": id, "label": label, "buttonType": "postback", "postback": payload })
}

/// `buildSkipQuickReplies`.
fn skip_quick_replies(step: &Value, challenge_id: &str) -> Vec<Value> {
    if !has_skip_button(step) {
        return Vec::new();
    }
    vec![postback(
        &create_id(),
        &to_quick_reply_title(&skip_label(step)),
        &encode_step_input_skip_postback(challenge_id),
    )]
}

async fn workspace_is_vi(ctx: &StepCtx<'_, '_>) -> bool {
    let ws = ctx.workspace_id();
    let language: Option<String> = ctx
        .run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| r.get(0),
                )
                .unwrap_or(None))
        })
        .await
        .unwrap_or(None);
    chatbotx_business::contact::update_fields::normalize_language(language.as_deref()).as_deref()
        == Some("vi")
}

/// A `sendChatMessage` chat job; `wait` holds the step until it is sent.
pub(crate) async fn send_chat_message(
    ctx: &StepCtx<'_, '_>,
    text: &str,
    url: Option<&str>,
    quick_replies: Option<Vec<Value>>,
    wait: bool,
) -> Result<(), JobError> {
    let run = ctx.run;
    let mut data = Map::new();
    data.insert(
        "conversationId".into(),
        json!(run.conversation.id.to_string()),
    );
    data.insert(
        "contactInbox".into(),
        json!(run.contact_inbox.id.to_string()),
    );
    data.insert("text".into(), json!(text));
    if let Some(url) = url {
        data.insert("url".into(), json!(url));
    }
    if let Some(replies) = quick_replies {
        data.insert("quickReplies".into(), Value::Array(replies));
    }
    for key in ["trackingContext", "metadata"] {
        if let Some(value) = run.data.get(key).filter(|v| !v.is_null()) {
            data.insert(key.into(), value.clone());
        }
    }
    if let Some(bulk) = run.is_bulk_broadcast {
        data.insert("isBulkBroadcast".into(), json!(bulk));
    }
    let added = run
        .deps
        .jobs
        .add(
            QueueName::Chat,
            SEND_CHAT_MESSAGE_ACTION,
            json!({ "type": SEND_CHAT_MESSAGE_ACTION, "data": Value::Object(data) }),
            JobOptions::default(),
        )
        .await
        .map_err(db_fail)?;
    if wait {
        let target = WaitTarget {
            queue: Some(QueueName::Chat),
            job_id: added.job_id,
            ..WaitTarget::default()
        };
        let context = ChatWaitContext {
            conversation_id: Some(run.conversation.id.to_string()),
            step_id: ctx.str("id").map(str::to_owned),
        };
        wait_for_chat_job_completion(run.deps.waiter.as_ref(), Some(&target), &context).await;
    }
    Ok(())
}

/// `sendTextPrompt`: a `sendText` flow message with preset quick replies.
async fn send_text_prompt(
    ctx: &StepCtx<'_, '_>,
    text: &str,
    preset: Vec<Value>,
) -> Result<(), Fail> {
    let step = json!({
        "id": step_id(ctx),
        "nodeId": node_id(ctx)?,
        "stepType": "sendText",
        "text": text,
        "buttons": [],
    });
    let mut extras = Map::new();
    if !preset.is_empty() {
        extras.insert("presetQuickReplies".into(), Value::Array(preset));
    }
    crate::flow::send_flow_message_with(ctx.run, &step, ctx.anchor.clone(), extras).await?;
    Ok(())
}

/// `sendDateTimePrompt`: the signed date / datetime picker link.
async fn send_date_time_prompt(
    ctx: &StepCtx<'_, '_>,
    challenge_id: &str,
    mode: chatbotx_core::encryption::UserDataReplyFormat,
    skip: Vec<Value>,
) -> Result<(), Fail> {
    let run = ctx.run;
    let encryption = crate::channels::meta_webhook::encryption_from_env()
        .ok_or_else(|| Fail::Step("ENCRYPTION_KEY is not configured".into()))?;
    let claims = chatbotx_core::encryption::UserDataWebviewClaims {
        workspace_id: ctx.workspace_id().to_string(),
        conversation_id: run.conversation.id.to_string(),
        contact_inbox_id: run.contact_inbox.id.to_string(),
        contact_id: ctx.contact_id().to_string(),
        channel: run.contact_inbox.channel.clone(),
        flow_id: run.flow_version.flow_id.to_string(),
        flow_version_id: run.version_id(),
        step_id: step_id(ctx),
        node_id: node_id(ctx)?,
        challenge_id: challenge_id.to_owned(),
        output_field_id: text_of(ctx.step, "outputFieldId", ""),
        reply_format: mode,
    };
    let token = encryption
        .sign_user_data_webview_token(
            &claims,
            chatbotx_core::encryption::USER_DATA_WEBVIEW_TOKEN_TTL_MS,
        )
        .map_err(step_fail)?;
    let mut url = url::Url::parse(&run.deps.app_url)
        .and_then(|base| base.join("/extensions/datetime-picker"))
        .map_err(step_fail)?;
    url.query_pairs_mut().append_pair("token", &token);
    let label = if workspace_is_vi(ctx).await {
        "Chọn ngày"
    } else {
        "Select Date"
    };
    let mut replies = vec![json!({
        "id": create_id(),
        "label": label,
        "buttonType": "url",
        "url": url.to_string(),
        "messengerExtensions": true,
    })];
    replies.extend(skip);
    send_chat_message(
        ctx,
        &text_of(ctx.step, "message", ""),
        None,
        Some(replies),
        false,
    )
    .await?;
    Ok(())
}

/// `buildContactHintQuickReplies`: the stored phone / email (when it passes
/// the step's own validation), else Messenger's native quick reply.
async fn contact_hint_quick_replies(ctx: &StepCtx<'_, '_>) -> Result<Vec<Value>, Fail> {
    let format = reply_format(ctx);
    let (is_phone, is_email) = (format == "RF04", format == "RF03");
    let channel = ctx.run.contact_inbox.channel.as_str();
    if !(ctx.step["showHint"] == json!(true) && (is_phone || is_email))
        || ctx.anchor.is_some()
        || !CONTACT_HINT_CHANNELS.contains(&channel)
    {
        return Ok(Vec::new());
    }
    let (ws, contact) = (ctx.workspace_id(), ctx.contact_id());
    let column = if is_email { "email" } else { "phoneNumber" };
    let stored: Option<String> = ctx
        .run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        r#"SELECT "{column}" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#
                    ),
                    [contact, ws],
                    |r| r.get(0),
                )
                .unwrap_or(None))
        })
        .await
        .map_err(step_fail)?;
    let value = stored
        .as_deref()
        .map(chatbotx_core::js::trim)
        .filter(|v| !v.is_empty())
        .filter(|v| {
            let message = ReplyInputMessage {
                text: Some((*v).to_owned()),
                ..ReplyInputMessage::default()
            };
            matches!(
                validate_reply_input(&format, &message),
                ReplyValidation::Ok { .. }
            )
        });
    if let Some(value) = value {
        return Ok(vec![postback(
            &create_id(),
            &to_quick_reply_title(value),
            &encode_contact_hint_quick_reply_payload(value),
        )]);
    }
    if channel == "messenger" {
        use crate::channels::meta::{NATIVE_USER_EMAIL, NATIVE_USER_PHONE_NUMBER};
        let payload = if is_email {
            NATIVE_USER_EMAIL
        } else {
            NATIVE_USER_PHONE_NUMBER
        };
        return Ok(vec![postback(
            payload,
            if is_email { "Email" } else { "Phone" },
            payload,
        )]);
    }
    Ok(Vec::new())
}

/// `sendPrompt`: the question in the format its reply type and channel
/// call for.
async fn send_prompt(ctx: &StepCtx<'_, '_>, challenge_id: &str) -> Result<(), Fail> {
    use chatbotx_core::encryption::UserDataReplyFormat;
    let skip = skip_quick_replies(ctx.step, challenge_id);
    let channel = ctx.run.contact_inbox.channel.as_str();
    let format = reply_format(ctx);
    let message = text_of(ctx.step, "message", "");
    let mode = match format.as_str() {
        "RF09" => Some(UserDataReplyFormat::Date),
        "RF10" => Some(UserDataReplyFormat::Datetime),
        _ => None,
    };
    if let Some(mode) = mode.filter(|_| URL_QUICK_REPLY_CHANNELS.contains(&channel)) {
        return send_date_time_prompt(ctx, challenge_id, mode, skip).await;
    }
    if format == "RF08" && NATIVE_LOCATION_REQUEST_CHANNELS.contains(&channel) {
        let label = if workspace_is_vi(ctx).await {
            "Gửi vị trí"
        } else {
            "Send location"
        };
        let request = postback(
            WHATSAPP_NATIVE_LOCATION_REQUEST,
            label,
            WHATSAPP_NATIVE_LOCATION_REQUEST,
        );
        send_chat_message(ctx, &message, None, Some(vec![request]), true).await?;
        // The native request carries no other control: Skip follows alone.
        if !skip.is_empty() {
            send_text_prompt(ctx, &skip_label(ctx.step), skip).await?;
        }
        return Ok(());
    }
    let hints = contact_hint_quick_replies(ctx).await?;
    if !hints.is_empty() {
        let mut replies = hints;
        replies.extend(skip);
        send_chat_message(ctx, &message, None, Some(replies), true).await?;
        return Ok(());
    }
    send_text_prompt(ctx, &message, skip).await
}

// ---- the answer (`handler.ts`) ------------------------------------------------

/// `recordLastInputFailure` (`contactInboxService.updateTracking`).
async fn record_last_input_failure(
    ctx: &StepCtx<'_, '_>,
    reason: Option<&'static str>,
) -> Result<(), String> {
    let (id, contact) = (ctx.run.contact_inbox.id, ctx.run.contact_inbox.contact_id);
    ctx.run
        .deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "lastInputFailure" = ?1, "updatedAt" = ?4
                   WHERE "id" = ?2 AND "contactId" = ?3"#,
                chatbotx_db::rusqlite::params![reason, id, contact, now_ms()],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

async fn set_output_field(ctx: &StepCtx<'_, '_>, value: String) -> Result<(), String> {
    let Some(field) = ctx.str("outputFieldId").filter(|f| !f.is_empty()) else {
        return Ok(());
    };
    contact_custom_field::set_value_by_key(
        &ctx.run.deps.db,
        SetValueByKeyInput {
            workspace_id: ctx.workspace_id(),
            contact_id: ctx.contact_id(),
            keyword: field.to_owned(),
            value,
            allow_bot_fields: true,
            // A re-shared identical location still ranks newest.
            touch_unchanged: reply_format(ctx) == "RF08",
            ..SetValueByKeyInput::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| e.message().to_string())
}

/// `findAnsweredMessage`: the trigger message when the job names it, else
/// the latest incoming one. A read failure retries the job.
async fn find_answered_message(
    ctx: &StepCtx<'_, '_>,
) -> Result<Option<MessageWithAttachments>, JobError> {
    let conversation = &ctx.run.conversation;
    let (id, ws) = (conversation.id, conversation.workspace_id);
    let trigger = ctx.run.data.get("triggerMessageId").and_then(id_of);
    let trigger_at = ctx.run.data.get("triggerMessageCreatedAt").and_then(ms_of);
    if let (Some(message_id), Some(at)) = (trigger, trigger_at) {
        let since = get_safe_since_time(
            Some(SinceTimeInput::Date(at)),
            Some(TRIGGER_MESSAGE_LOOKBACK_MS),
        )
        .unwrap_or(0);
        let found = ctx
            .run
            .deps
            .db
            .read(move |conn| {
                find_trigger_message(
                    conn,
                    &FindTriggerMessageOptions {
                        conversation_id: id,
                        id: message_id,
                        require_complete_results: true,
                        since_time: since,
                        workspace_id: ws,
                    },
                )
                .map_err(crate::repo_err)
            })
            .await
            .map_err(|e| JobError::retryable(e.to_string()))?;
        if found.is_some() {
            return Ok(found);
        }
    }
    let anchor = conversation
        .last_activity_at
        .unwrap_or(conversation.created_at);
    let since = get_safe_since_time(Some(SinceTimeInput::Date(anchor)), Some(YEAR_MS));
    ctx.run
        .deps
        .db
        .read(move |conn| {
            find_last_by_conversation(
                conn,
                id,
                &FindLastByConversationOptions {
                    limit: Some(1),
                    message_types: vec!["incoming".into()],
                    require_complete_results: true,
                    since_time: since,
                    with_attachments: true,
                    workspace_id: Some(ws),
                    ..FindLastByConversationOptions::default()
                },
            )
            .map_err(crate::repo_err)
        })
        .await
        .map(|rows| rows.into_iter().next())
        .map_err(|e| JobError::retryable(e.to_string()))
}

/// `validateUserData`.
async fn validate_user_data(ctx: &StepCtx<'_, '_>) -> Result<ReplyValidation, JobError> {
    let Some(found) = find_answered_message(ctx).await? else {
        return Ok(ReplyValidation::Rejected(format!(
            "getUserData: unable to find last message of conversation {}",
            ctx.run.conversation.id
        )));
    };
    let message = ReplyInputMessage {
        attachments: found
            .attachments
            .iter()
            .map(|a| ReplyInputAttachment {
                file_type: a.file_type.clone(),
                id: Some(a.id),
                message_created_at: Some(a.message_created_at),
                origin_path: a.origin_path.clone(),
            })
            .collect(),
        content_attributes: found
            .message
            .content_attributes
            .as_ref()
            .map(|j| j.0.clone()),
        content_type: found.message.content_type.clone(),
        text: found.message.text.clone(),
    };
    let validated = validate_reply_input(&reply_format(ctx), &message);
    // ponytail: no lazy-media store (`ensureAttachmentStored`) in Rust yet: a
    // failed ingest is "no longer available", a provider URL is kept as is.
    if let ReplyValidation::Ok {
        user_input,
        attachment: Some(_),
        ..
    } = &validated
        && chatbotx_business::media_url::is_failed_origin_path(user_input)
    {
        return Ok(ReplyValidation::Rejected(
            "getUserData: attachment is no longer available".into(),
        ));
    }
    Ok(validated)
}

/// `resolveOutputValue`: an attachment's storage key as a public URL.
fn output_value(ctx: &StepCtx<'_, '_>, user_input: &str, kind: ReplyInputKind) -> String {
    if kind != ReplyInputKind::Attachment {
        return user_input.to_owned();
    }
    let storage = std::env::var("NEXT_PUBLIC_STORAGE_URL").ok();
    let urls = chatbotx_business::contact::avatar::TenantUrls::from_app_url(
        &ctx.run.deps.app_url,
        storage.as_deref(),
    );
    chatbotx_core::utils::storage::get_public_file_url(user_input, &urls.storage_url)
        .unwrap_or_else(|_| user_input.to_owned())
}

/// `findAutoSkipReason`: the deadline counts from the first prompt.
fn auto_skip_reason(ctx: &StepCtx<'_, '_>) -> Option<&'static str> {
    let vars = variables(ctx);
    let origin = vars
        .and_then(|v| ms_of(&v["startedAt"]))
        .or_else(|| vars.and_then(|v| ms_of(&v["lastAttemptAt"])))
        .unwrap_or_else(now_ms);
    if origin + auto_skip_window_ms(ctx.step) < now_ms() {
        return Some("timeout");
    }
    (challenge_attempts(ctx) >= number_of(ctx.step, "autoSkipFailAttempts", 3))
        .then_some("invalid_input_attempts")
}

/// `sendRetryPrompt`: the retry message, then the question again.
async fn send_retry_prompt(ctx: &StepCtx<'_, '_>) -> Outcome {
    let Some(challenge_id) = advance_challenge_attempt(ctx, challenge_attempts(ctx)).await? else {
        tracing::info!(
            conversation_id = ctx.run.conversation.id,
            "getUserData: a concurrent reply already advanced the attempt, not retrying"
        );
        return Ok(StepStatus::Wait);
    };
    let retry = text_of(ctx.step, "retryMessage", "");
    let retry = chatbotx_core::js::trim(&retry);
    if !retry.is_empty() {
        send_text_prompt(ctx, retry, Vec::new()).await?;
    }
    send_prompt(ctx, &challenge_id).await?;
    Ok(StepStatus::Retry)
}

async fn handle_skip_or_error(ctx: &StepCtx<'_, '_>) -> Outcome {
    let validated = validate_user_data(ctx).await.map_err(Fail::Job)?;
    match validated {
        ReplyValidation::Ok {
            user_input, kind, ..
        } => {
            let value = output_value(ctx, &user_input, kind);
            let lost = settle(ctx, Claim::Pending, async || {
                record_last_input_failure(ctx, None).await?;
                set_output_field(ctx, value).await
            })
            .await?;
            Ok(lost.unwrap_or(StepStatus::Success))
        }
        ReplyValidation::Rejected(reason) => {
            if let Some(skip) = auto_skip(ctx).then(|| auto_skip_reason(ctx)).flatten() {
                let lost = settle(ctx, Claim::Pending, async || {
                    record_last_input_failure(ctx, Some(skip)).await
                })
                .await?;
                return Ok(lost.unwrap_or(StepStatus::Skip));
            }
            tracing::info!(conversation_id = ctx.run.conversation.id, reason = %reason, "getUserData: input rejected, retrying");
            send_retry_prompt(ctx).await
        }
    }
}

/// `handleInputResolution`: the timer ran out or the contact tapped Skip.
async fn handle_input_resolution(ctx: &StepCtx<'_, '_>, reason: &'static str) -> Outcome {
    let enabled = match reason {
        "timeout" => auto_skip(ctx),
        _ => has_skip_button(ctx.step),
    };
    if !enabled {
        tracing::info!(
            conversation_id = ctx.run.conversation.id,
            reason,
            "getUserData: input resolution is not enabled on the step, ignoring"
        );
        return Ok(StepStatus::Wait);
    }
    let lost = settle(ctx, Claim::Pending, async || {
        let recorded = record_last_input_failure(ctx, Some(reason)).await;
        // A timeout has no contact to retry: the Skip branch is taken anyway.
        match recorded {
            Err(err) if reason == "timeout" => {
                tracing::warn!(err = %err, conversation_id = ctx.run.conversation.id, "getUserData: could not record the input failure, taking the Skip branch anyway");
                Ok(())
            }
            other => other,
        }
    })
    .await?;
    Ok(lost.unwrap_or(StepStatus::Skip))
}

/// `handleWebviewSelection`: the claim names the token's own cycle.
async fn handle_webview_selection(ctx: &StepCtx<'_, '_>, metadata: &Value) -> Outcome {
    let selected = str_of(metadata, "selectedValue").unwrap_or("").to_owned();
    let challenge_id = id_text(&metadata["challengeId"]).unwrap_or_default();
    let lost = settle(ctx, Claim::Token(challenge_id), async || {
        record_last_input_failure(ctx, None).await?;
        set_output_field(ctx, selected).await
    })
    .await?;
    Ok(lost.unwrap_or(StepStatus::Success))
}

/// `failStep`: a run only clears the cycle it owns.
async fn fail_step(
    ctx: &StepCtx<'_, '_>,
    failure: Fail,
    owned: Option<&str>,
) -> Result<StepStatus, JobError> {
    let message = match failure {
        Fail::Job(err) => return Err(err),
        Fail::Step(message) => message,
    };
    tracing::error!(err = %message, "getUserData: error");
    if !clear_challenge_owned_by_run(ctx, owned).await? {
        return Ok(StepStatus::Wait);
    }
    Ok(step_error(ctx, &message))
}

/// `firstSendMessage`: writes the challenge (reusing this step's pending
/// cycle id), arms the timer, then asks.
async fn first_send_message(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let challenge_id = pending(ctx)
        .filter(|p| pending_is_this_step(ctx, p))
        .and_then(|p| id_text(&p["data"]["challengeId"]))
        .unwrap_or_else(create_id);
    let started_at = now_ms();
    if let Err(failure) = write_challenge(ctx, 1, &challenge_id, Some(started_at)).await {
        return fail_step(ctx, failure, None).await;
    }
    let asked = async {
        if auto_skip(ctx) {
            arm_input_timeout(ctx, &challenge_id, started_at).await?;
        }
        send_prompt(ctx, &challenge_id).await
    }
    .await;
    match asked {
        Ok(()) => Ok(StepStatus::Wait),
        Err(failure) => fail_step(ctx, failure, Some(&challenge_id)).await,
    }
}

/// `getUserData`.
pub(crate) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let metadata = ctx.run.metadata();
    let webview = metadata.filter(|m| {
        str_of(m, "type") == Some(WEBVIEW_SELECTION) && id_text(&m["stepId"]) == Some(step_id(ctx))
    });
    // The resolution names the pending challenge's step, so another step of
    // the run never consumes it.
    let resolution = variables(ctx)
        .and_then(|v| match v["resolution"].as_str() {
            Some("timeout") => Some("timeout"),
            Some("user_skipped") => Some("user_skipped"),
            _ => None,
        })
        .filter(|_| pending(ctx).is_some_and(|p| pending_is_this_step(ctx, &p)));
    let is_answer = variables(ctx).is_some();
    if webview.is_none() && resolution.is_none() && !is_answer {
        return first_send_message(ctx).await;
    }
    let outcome = if let Some(metadata) = webview {
        handle_webview_selection(ctx, metadata).await
    } else if let Some(reason) = resolution {
        handle_input_resolution(ctx, reason).await
    } else {
        handle_skip_or_error(ctx).await
    };
    match outcome {
        Ok(status) => Ok(status),
        Err(failure) => fail_step(ctx, failure, run_challenge_id(ctx).as_deref()).await,
    }
}
