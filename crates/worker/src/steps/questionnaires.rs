//! `integration/handlers/questionnaires.ts` and
//! `questionnaires/services/engine.ts` (`runQuestionnaireEngine`): asks the
//! questionnaire's questions one by one; each answer re-enters the step
//! through its step challenge (`runChallenge`).

use chatbotx_business::conversation_challenge::update_challenge;
use chatbotx_business::leaderboard::scoring::Award;
use chatbotx_business::questionnaire::flow::{self as engine, AnswerInput, Answered, Started};
use chatbotx_core::flow_config::steps::button_limits::BUTTON_LABEL_MAX;
use chatbotx_core::flow_config::steps::send_quick_reply::MAX_QUICK_REPLIES;
use chatbotx_db::repo::message::{
    FindLastByConversationOptions, FindMessageByIdParams, SinceTimeInput, find_by_id,
    find_last_by_conversation, get_safe_since_time,
};
use chatbotx_db::repo::models::QuestionnaireQuestion;
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_jobs::JobError;
use chatbotx_jobs::queues::integration::flow_continuation_job_options;
use serde_json::{Map, Value, json};

use super::StepCtx;
use super::get_user_data::send_chat_message;
use crate::challenge::{STEP_CHALLENGE, pending_step_challenge};
use crate::flow::{SEND_FLOW, StepStatus};
use crate::{id_of, ms_of, str_of};

/// `QUICK_REPLY_CHANNELS`.
const QUICK_REPLY_CHANNELS: &[&str] = &["messenger", "instagram", "telegram", "webchat"];
const LOOKBACK_MS: i64 = 365 * 24 * 60 * 60 * 1000;

type Engine = Result<StepStatus, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn options(question: &QuestionnaireQuestion) -> Vec<Value> {
    question
        .config
        .as_ref()
        .and_then(|c| c.0.get("options"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `questionText`: a multiple-choice question lists its options.
fn question_text(question: &QuestionnaireQuestion) -> String {
    let options = options(question);
    if question.r#type != "multipleChoice" || options.is_empty() {
        return question.title.clone();
    }
    let lines: Vec<String> = options
        .iter()
        .enumerate()
        .map(|(i, o)| format!("{}. {}", i + 1, o["label"].as_str().unwrap_or("")))
        .collect();
    format!("{}\n{}", question.title, lines.join("\n"))
}

/// `questionQuickReplies`, else `nativeContactQuickReply`.
fn quick_replies(question: &QuestionnaireQuestion, channel: &str) -> Option<Vec<Value>> {
    let options = options(question);
    if question.r#type == "multipleChoice"
        && QUICK_REPLY_CHANNELS.contains(&channel)
        && !options.is_empty()
        && options.len() <= MAX_QUICK_REPLIES
        && options.iter().all(|o| {
            chatbotx_core::js::utf16_len(o["label"].as_str().unwrap_or("")) <= BUTTON_LABEL_MAX
        })
    {
        return Some(
            options
                .iter()
                .map(|o| json!({ "id": o["id"], "label": o["label"], "buttonType": "postback", "postback": o["id"] }))
                .collect(),
        );
    }
    if question.r#type == "multipleChoice" || channel != "messenger" {
        return None;
    }
    use crate::channels::meta::{NATIVE_USER_EMAIL, NATIVE_USER_PHONE_NUMBER};
    let payload = match question.r#type.as_str() {
        "email" => NATIVE_USER_EMAIL,
        "phone" => NATIVE_USER_PHONE_NUMBER,
        _ => return None,
    };
    Some(vec![
        json!({ "id": payload, "label": question.title, "buttonType": "postback", "postback": payload }),
    ])
}

/// `sendQuestion`: the question, then the submission and challenge marks.
async fn send_question(
    ctx: &StepCtx<'_, '_>,
    submission_id: i64,
    question: &QuestionnaireQuestion,
    text: Option<&str>,
    sent_at: Option<i64>,
    attempts: i64,
) -> Result<(), String> {
    let run = ctx.run;
    let sent_at = sent_at.unwrap_or_else(now_ms);
    let replies = quick_replies(question, &run.contact_inbox.channel);
    let text = match text.map(chatbotx_core::js::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_owned(),
        None if replies.is_some() => question.title.clone(),
        None => question_text(question),
    };
    let url = question
        .image
        .as_ref()
        .and_then(|i| i.0.get("url"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    send_chat_message(ctx, &text, url.as_deref(), replies, true)
        .await
        .map_err(|e| e.message)?;
    engine::mark_question_sent(
        &run.deps.db,
        ctx.workspace_id(),
        submission_id,
        question.id,
        sent_at,
    )
    .await
    .map_err(err)?;
    let mut data = Map::new();
    data.insert("flowId".into(), json!(run.flow_version.flow_id.to_string()));
    if let Some(version) = run.version_id() {
        data.insert("flowVersionId".into(), json!(version));
    }
    let node = if run.target_id.is_empty() {
        &run.target_node_id
    } else {
        &run.target_id
    };
    data.insert("nodeId".into(), json!(node));
    data.insert("stepId".into(), ctx.step["id"].clone());
    data.insert("attempts".into(), json!(attempts));
    data.insert("lastAttemptAt".into(), json!(iso(sent_at)));
    update_challenge(
        &run.deps.db,
        ctx.workspace_id(),
        run.conversation.id,
        Some(json!({ "type": "step", "data": data })),
    )
    .await
    .map_err(err)
}

/// `getLastIncomingText`: the trigger message when it is this contact's
/// incoming message, else the latest incoming one.
async fn last_incoming_text(ctx: &StepCtx<'_, '_>) -> Result<String, String> {
    let conversation = &ctx.run.conversation;
    let (id, ws) = (conversation.id, conversation.workspace_id);
    let trigger = ctx.run.data.get("triggerMessageId").and_then(id_of);
    let trigger_at = ctx.run.data.get("triggerMessageCreatedAt").and_then(ms_of);
    if let (Some(message_id), Some(at)) = (trigger, trigger_at) {
        let found = ctx
            .run
            .deps
            .db
            .read(move |conn| {
                find_by_id(
                    conn,
                    &FindMessageByIdParams {
                        conversation_id: None,
                        created_at: at,
                        id: message_id,
                        workspace_id: ws,
                    },
                )
                .map_err(crate::repo_err)
            })
            .await
            .map_err(err)?;
        if let Some(found) = found.map(|f| f.message)
            && found.conversation_id == id
            && found.sender_type == "contact"
            && found.message_type == "incoming"
        {
            return Ok(found.text.unwrap_or_default());
        }
    }
    let anchor = conversation
        .last_activity_at
        .unwrap_or(conversation.created_at);
    let since = get_safe_since_time(Some(SinceTimeInput::Date(anchor)), Some(LOOKBACK_MS));
    let rows = ctx
        .run
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
                    workspace_id: Some(ws),
                    ..FindLastByConversationOptions::default()
                },
            )
            .map_err(crate::repo_err)
        })
        .await
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|m| m.message.text)
        .unwrap_or_default())
}

async fn clear(ctx: &StepCtx<'_, '_>) -> Result<(), String> {
    update_challenge(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        ctx.run.conversation.id,
        None,
    )
    .await
    .map_err(err)
}

/// `startTriggerFlow`: the questionnaire's follow-up flow.
async fn start_trigger_flow(ctx: &StepCtx<'_, '_>, flow_id: Option<i64>) -> Result<(), String> {
    let Some(flow_id) = flow_id else {
        return Ok(());
    };
    let run = ctx.run;
    let mut data = Map::new();
    data.insert(
        "conversationId".into(),
        json!(run.conversation.id.to_string()),
    );
    data.insert(
        "contactInboxId".into(),
        json!(run.contact_inbox.id.to_string()),
    );
    data.insert("flowId".into(), json!(flow_id.to_string()));
    for key in ["metadata", "trackingContext", "sendFrom"] {
        if let Some(v) = run.data.get(key).filter(|v| !v.is_null()) {
            data.insert(key.into(), v.clone());
        }
    }
    data.insert("origin".into(), json!("webhook"));
    let opts = flow_continuation_job_options(run.metadata());
    run.deps
        .jobs
        .add(
            chatbotx_jobs::QueueName::Integration,
            SEND_FLOW,
            json!({ "type": SEND_FLOW, "data": Value::Object(data) }),
            opts,
        )
        .await
        .map(|_| ())
        .map_err(err)
}

/// `startOrResumeQuestionnaire`.
async fn start_or_resume(ctx: &StepCtx<'_, '_>, questionnaire_id: i64) -> Engine {
    let conversation = &ctx.run.conversation;
    let started = engine::start_or_resume(
        &ctx.run.deps.db,
        conversation.workspace_id,
        questionnaire_id,
        conversation.contact_id,
        Some(conversation.id),
    )
    .await
    .map_err(err)?;
    match started {
        Started::Skip(reason) => {
            tracing::info!(
                conversation_id = conversation.id,
                questionnaire_id,
                reason,
                "Questionnaire start skipped"
            );
            Ok(if reason == "questionnaire_already_submitted" {
                StepStatus::Wait
            } else {
                StepStatus::Success
            })
        }
        Started::Wait {
            submission_id,
            question,
        } => {
            send_question(ctx, submission_id, &question, None, None, 0).await?;
            Ok(StepStatus::Wait)
        }
    }
}

/// `runQuestionnaireEngine`.
async fn run_engine(ctx: &StepCtx<'_, '_>) -> Engine {
    let conversation = &ctx.run.conversation;
    let (ws, contact) = (conversation.workspace_id, conversation.contact_id);
    let questionnaire_id = ctx
        .step
        .get("questionnaireId")
        .and_then(id_of)
        .unwrap_or_default();
    let db = &ctx.run.deps.db;
    match ctx.str("mode") {
        Some("end") => {
            engine::cancel(db, ws, questionnaire_id, contact)
                .await
                .map_err(err)?;
            clear(ctx).await?;
            return Ok(StepStatus::Success);
        }
        Some("deleteApplicant") => {
            engine::delete_applicant(db, ws, questionnaire_id, contact)
                .await
                .map_err(err)?;
            clear(ctx).await?;
            return Ok(StepStatus::Success);
        }
        _ => {}
    }
    if pending_step_challenge(conversation).is_none() {
        return start_or_resume(ctx, questionnaire_id).await;
    }
    let attempts = ctx
        .run
        .data
        .get(STEP_CHALLENGE)
        .and_then(|v| v["attempts"].as_i64())
        .unwrap_or(0);
    let raw_text = last_incoming_text(ctx).await?;
    let answered = engine::answer_current(
        db,
        AnswerInput {
            workspace_id: ws,
            contact_id: contact,
            conversation_id: conversation.id,
            questionnaire_id,
            raw_text,
            attempts,
            trigger_message_id: ctx.run.data.get("triggerMessageId").and_then(id_of),
        },
    )
    .await
    .map_err(err)?;
    match answered {
        Answered::Retry {
            submission_id,
            question,
            attempts,
            retry_message,
            reason,
        } => {
            tracing::info!(
                conversation_id = conversation.id,
                questionnaire_id,
                reason,
                "Questionnaire answer rejected, retrying"
            );
            send_question(
                ctx,
                submission_id,
                &question,
                retry_message.as_deref(),
                None,
                attempts,
            )
            .await?;
            Ok(StepStatus::Retry)
        }
        Answered::Completed {
            submission_id,
            trigger_flow_id,
            completed_at,
        } => {
            // WhatsApp leaderboards score a completed questionnaire ("survey").
            crate::leaderboard::enqueue(
                ctx.run.deps,
                false,
                Award {
                    workspace_id: ws,
                    contact_id: contact,
                    inbox_id: Some(ctx.run.contact_inbox.inbox_id),
                    minigame_id: None,
                    rule: "survey".into(),
                    source_ref: format!("questionnaire:{submission_id}"),
                    event_at: completed_at,
                    context: Value::Null,
                },
            )
            .await;
            clear(ctx).await?;
            start_trigger_flow(ctx, trigger_flow_id).await?;
            Ok(StepStatus::Success)
        }
        Answered::Wait {
            submission_id,
            question,
            sent_at,
        } => {
            send_question(ctx, submission_id, &question, None, Some(sent_at), 0).await?;
            Ok(StepStatus::Wait)
        }
        Answered::Skip("submission_not_found") => {
            tracing::info!(
                conversation_id = conversation.id,
                questionnaire_id,
                "Questionnaire challenge has no active submission, starting configured questionnaire"
            );
            start_or_resume(ctx, questionnaire_id).await
        }
        Answered::Skip(_) => {
            clear(ctx).await?;
            Ok(StepStatus::Success)
        }
    }
}

/// `questionnaires`: any failure clears the challenge and fails the step.
pub(crate) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    match run_engine(ctx).await {
        Ok(status) => Ok(status),
        Err(message) => {
            tracing::error!(
                err = %message,
                conversation_id = ctx.run.conversation.id,
                questionnaire_id = %ctx.step["questionnaireId"],
                mode = ctx.str("mode").unwrap_or(""),
                "Questionnaires step failed"
            );
            if let Err(clear_err) = clear(ctx).await {
                tracing::warn!(err = %clear_err, conversation_id = ctx.run.conversation.id, "Questionnaires step failed to clear challenge after error");
            }
            Ok(StepStatus::Error)
        }
    }
}
