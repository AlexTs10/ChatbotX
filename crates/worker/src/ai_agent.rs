//! The `ai-agent` queue (`apps/worker/src/ai-agent/worker.ts`): the story
//! automations, the comment AI jobs (`low::comment_ai`), the embeddings and
//! `processAutomatedResponse`, of which only the keyword step is ported
//! (`automatedResponseService.process`: `process-messages.ts` +
//! `dispatch-reply.ts`).

use chatbotx_business::keyword_match::{
    KeywordRule, MatchInput, is_keyword_media_file_type, rule_matches_message,
};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::contact_activity::{
    self as activity, ContactActivityRecord, ContactKeywordMessageRecord,
};
use chatbotx_db::repo::message::{self as repo_message, MessageTextWithFileTypes, SinceTimeInput};
use chatbotx_db::repo::models::{AutomatedResponse, ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{JobCtx, JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::inbound::{PROCESS_AUTOMATED_RESPONSE, automated_response_key, kv, workspace_is_active};
use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

/// `KEYWORD_MESSAGE_MATCH_MAX_CHARS`.
const KEYWORD_MESSAGE_MATCH_MAX_CHARS: usize = 512;

/// The ai-agent worker processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    match kind.as_str() {
        PROCESS_AUTOMATED_RESPONSE => process_automated_response(&deps, &job.data["data"]).await?,
        "processPendingEmbedding" => return crate::ai_file::handle_ai_agent(deps, job).await,
        crate::story_automation::PROCESS_STORY_REPLY => {
            let data = &job.data["data"];
            crate::story_automation::process(&deps, data, crate::story_automation::STORY_REPLY)
                .await?
        }
        crate::story_automation::PROCESS_STORY_MENTION => {
            let data = &job.data["data"];
            crate::story_automation::process(&deps, data, crate::story_automation::STORY_MENTION)
                .await?
        }
        crate::low::COMMENT_MODERATION => {
            crate::low::comment_moderation(&deps, &job.data["data"]).await?
        }
        crate::low::COMMENT_AI_REPLY => crate::low::comment_ai_reply(&deps, &job).await?,
        crate::ai::context::SUMMARIZE_CONVERSATION => {
            crate::ai::context::handle_summarize(&deps, &job.data["data"]).await?
        }
        crate::ai::conversation_source::PROCESS_CONVERSATION_SOURCE => {
            crate::ai::conversation_source::process_source(&deps, &job.data["data"]).await?
        }
        crate::ai::conversation_source::PROCESS_CONVERSATION_SOURCE_EMBEDDING => {
            crate::ai::conversation_source::process_embedding(&deps, &job.data["data"]).await?
        }
        crate::handoff_reentry::PROCESS_HANDOFF_REENTRY => {
            crate::handoff_reentry::process(&deps, &job.data["data"]).await?
        }
        other => {
            return Err(JobError::unrecoverable(format!(
                "AIJobAction action is not defined: {other}"
            )));
        }
    }
    Ok(Value::Null)
}

/// `status = true` rules, `createdAt` ascending (`automatedResponseService.getAll`),
/// inbound only, with their match shape.
pub(crate) async fn inbound_rules(
    deps: &Deps,
    workspace_id: i64,
) -> Result<Vec<(AutomatedResponse, KeywordRule)>, JobError> {
    rules_of_type(deps, workspace_id, "inbound").await
}

/// The active rules of one `type` (`inbound` / `outbound`), oldest first.
async fn rules_of_type(
    deps: &Deps,
    workspace_id: i64,
    rule_type: &'static str,
) -> Result<Vec<(AutomatedResponse, KeywordRule)>, JobError> {
    // ponytail: no 24 h rule cache; one indexed read per message.
    let rows = deps
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(&format!(
                r#"{} WHERE "workspaceId" = ?1 AND "status" = 1 AND "type" = ?2
                   ORDER BY "createdAt" ASC, "id" ASC"#,
                AutomatedResponse::select_sql()
            ))?;
            let rows = stmt
                .query_map(
                    params![workspace_id, rule_type],
                    AutomatedResponse::from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let rule = KeywordRule::of(&row);
            (row, rule)
        })
        .collect())
}

/// `processAutomatedResponse`: the workspace / bot gates, then the keyword
/// rules over the pending messages.
pub(crate) async fn process_automated_response(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let conversation_id = data
        .get("conversationId")
        .and_then(id_of)
        .ok_or_else(|| JobError::unrecoverable("processAutomatedResponse: no conversationId"))?;
    let contact_inbox_id = data
        .get("contactInboxId")
        .and_then(id_of)
        .ok_or_else(|| JobError::unrecoverable("processAutomatedResponse: no contactInboxId"))?;
    let Some(conversation) = load_conversation(&deps.db, conversation_id).await? else {
        return Err(fail("Conversation not found"));
    };
    let Some(contact_inbox) =
        load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    else {
        return Err(fail("Contact inbox not found"));
    };
    if !workspace_is_active(deps, conversation.workspace_id).await? || !conversation.bot_enabled {
        return Ok(());
    }
    let handled = process_pending_messages(deps, &conversation, &contact_inbox).await?;
    // ponytail: product search, the AI agent reply, the Global Router and
    // the default reply flow after a keyword miss are Q.D6.
    let _ = handled;
    Ok(())
}

/// `checkOutboundAutomatedResponse` (`replyByOutboundAutomatedResponse`): an
/// agent's message text against the workspace's outbound (Page) rules.
pub(crate) async fn check_outbound_automated_response(
    deps: &Deps,
    data: &Value,
) -> Result<(), JobError> {
    let text = str_of(&data["message"], "text").unwrap_or("").to_owned();
    let message_id = data["message"].get("id").and_then(id_of);
    let (Some(message_id), false) = (message_id, text.is_empty()) else {
        return Ok(());
    };
    let conversation_id = data["conversation"].get("id").and_then(id_of);
    let contact_inbox_id = data["contactInbox"].get("id").and_then(id_of);
    let (Some(conversation_id), Some(contact_inbox_id)) = (conversation_id, contact_inbox_id)
    else {
        return Err(JobError::unrecoverable(
            "checkOutboundAutomatedResponse: no conversation or contact inbox",
        ));
    };
    let Some(conversation) = load_conversation(&deps.db, conversation_id).await? else {
        return Err(fail("Conversation not found"));
    };
    let Some(contact_inbox) =
        load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    else {
        return Err(fail("Contact inbox not found"));
    };
    let rules = rules_of_type(deps, conversation.workspace_id, "outbound").await?;
    let message = MessageTextWithFileTypes {
        id: message_id,
        text: Some(text),
        attachment_file_types: Vec::new(),
    };
    dispatch_reply(deps, &conversation, &contact_inbox, &message, &rules, false).await?;
    Ok(())
}

/// `processPendingMessages`: read the pending list, try the rules, clear it.
async fn process_pending_messages(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
) -> Result<bool, JobError> {
    let key = automated_response_key(conversation.id, contact_inbox.id);
    let store = kv(deps);
    let ids: Vec<String> = store.range(&key).await.map_err(|e| fail(e.to_string()))?;
    let ids: Vec<i64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
    let mut handled = false;
    if !ids.is_empty() {
        let since = repo_message::get_safe_since_time(
            contact_inbox.last_message_at.map(SinceTimeInput::Ms),
            Some(365 * 24 * 60 * 60 * 1000),
        );
        let ci_id = contact_inbox.id;
        let workspace_id = conversation.workspace_id;
        let messages = deps
            .db
            .read(move |conn| {
                repo_message::find_many_by_ids(conn, &ids, ci_id, since, Some(workspace_id))
                    .map_err(crate::repo_err)
            })
            .await
            .map_err(db_fail)?;
        let messages: Vec<MessageTextWithFileTypes> = messages
            .into_iter()
            .filter(|m| {
                m.text.as_deref().is_some_and(|t| !t.is_empty())
                    || m.attachment_file_types
                        .iter()
                        .any(|t| is_keyword_media_file_type(t))
            })
            .collect();
        if !messages.is_empty() {
            let rules = inbound_rules(deps, conversation.workspace_id).await?;
            for message in &messages {
                if dispatch_reply(deps, conversation, contact_inbox, message, &rules, true).await? {
                    handled = true;
                    break;
                }
            }
        }
    }
    store.clear(&key).await.map_err(|e| fail(e.to_string()))?;
    Ok(handled)
}

/// `dispatchAutomatedResponseReply`: the first matching rule with a
/// published flow or a text reply wins. `inbound` = a contact's message
/// (`contact_message_in`), else an agent's (`agent_message_out`).
async fn dispatch_reply(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    message: &MessageTextWithFileTypes,
    rules: &[(AutomatedResponse, KeywordRule)],
    inbound: bool,
) -> Result<bool, JobError> {
    let text = message.text.clone().unwrap_or_default();
    let input = MatchInput {
        text: Some(text.clone()),
        attachment_file_types: message.attachment_file_types.clone(),
    };
    for (row, rule) in rules {
        if !rule_matches_message(rule, &input) {
            continue;
        }
        // ponytail: the keyword leaderboard award is not ported. The CTWA ads
        // `keywordMatched` evaluation waits for its consumer
        // (`evaluateConversionTrigger` / `sendConversionEvent`, not ported):
        // a job with no handler fails unrecoverable (DECISIONS 2026-11-20).
        let tracking = json!({
            "aiProvider": "none",
            "messageId": message.id.to_string(),
            "conversationId": conversation.id.to_string(),
            "responseType": "automated_response",
            "startTime": now_ms(),
            "triggerType": if inbound { "contact_message_in" } else { "agent_message_out" },
            "workspaceId": conversation.workspace_id.to_string(),
        });
        if let Some(flow_id) = row.flow_id {
            if !flow_is_published(deps, flow_id).await? {
                continue;
            }
            if row.r#type == "inbound" {
                record_keyword_match(deps, conversation, contact_inbox, message.id, row.id, &text)
                    .await?;
            }
            let mut data = json!({
                "conversationId": conversation.id.to_string(),
                "contactInboxId": contact_inbox.id.to_string(),
                "flowId": flow_id.to_string(),
                "trackingContext": tracking,
                // `webhookChannelOrigin()`: the job runs inside the
                // inbound (webhook) context.
                "origin": "channel",
            });
            if inbound {
                data["startedByContactMessage"] = Value::Bool(true);
            }
            deps.jobs
                .add(
                    QueueName::Integration,
                    "sendFlow",
                    json!({ "type": "sendFlow", "data": data }),
                    JobOptions::default(),
                )
                .await
                .map_err(|e| fail(e.to_string()))?;
            return Ok(true);
        }
        let Some(reply) = row.text.as_deref().filter(|t| !t.is_empty()) else {
            continue;
        };
        if row.r#type == "inbound" {
            record_keyword_match(deps, conversation, contact_inbox, message.id, row.id, &text)
                .await?;
        }
        // Spun before the variable pass: the rule text is author copy.
        let spun = chatbotx_core::utils::spintax::apply_spintax(reply);
        // ponytail: `resolve_step` spins again after the substitution; contact
        // data holding `{a|b}` is spun too (the original ships it as-is).
        let text = match &deps.variables {
            Some(vars) => vars
                .resolve_step(
                    conversation,
                    contact_inbox,
                    &Value::String(spun.clone()),
                    None,
                    None,
                )
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or(spun),
            None => spun,
        };
        deps.jobs
            .add(
                QueueName::Chat,
                "sendChatMessage",
                json!({ "type": "sendChatMessage", "data": {
                    "conversation": chatbotx_business::message::conversation_json(conversation),
                    "text": text,
                    "trackingContext": tracking,
                }}),
                JobOptions::default(),
            )
            .await
            .map_err(|e| fail(e.to_string()))?;
        return Ok(true);
    }
    Ok(false)
}

async fn flow_is_published(deps: &Deps, flow_id: i64) -> Result<bool, JobError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT EXISTS (SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "currentVersionId" IS NOT NULL)"#,
                    params![flow_id],
                    |r| r.get::<_, bool>(0),
                )?)
        })
        .await
        .map_err(db_fail)
}

/// `contactActivityService.recordKeywordMatch`: the "keyword received"
/// activity and the matched text, idempotent by message id.
async fn record_keyword_match(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    message_id: i64,
    rule_id: i64,
    text: &str,
) -> Result<(), JobError> {
    if std::env::var("CONTACT_ACTIVITY_WRITES_ENABLED").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off" | "n" | "f"
        )
    }) {
        return Ok(());
    }
    let occurred_at = chatbotx_core::utils::id::created_at_from_id(&message_id.to_string())
        .unwrap_or_else(now_ms);
    let normalized: String = chatbotx_core::utils::normalize_for_match::normalize_for_match(text)
        .trim()
        .chars()
        .take(KEYWORD_MESSAGE_MATCH_MAX_CHARS)
        .collect();
    let workspace_id = conversation.workspace_id;
    let contact_id = contact_inbox.contact_id;
    let text = text.to_owned();
    deps.db
        .write(move |tx| {
            activity::upsert_activities(
                tx,
                workspace_id,
                &[ContactActivityRecord {
                    activity_type: "automatedResponseMatched".into(),
                    contact_id,
                    occurred_at,
                    parent_reference_id: 0,
                    reference_id: rule_id,
                    reference_type: "automatedResponse".into(),
                }],
            )
            .map_err(crate::repo_err)?;
            activity::insert_keyword_message(
                tx,
                workspace_id,
                &ContactKeywordMessageRecord {
                    automated_response_id: rule_id,
                    contact_id,
                    message_id,
                    normalized_text: normalized,
                    occurred_at,
                    text,
                },
            )
            .map_err(crate::repo_err)?;
            Ok(())
        })
        .await
        .map_err(db_fail)
}
