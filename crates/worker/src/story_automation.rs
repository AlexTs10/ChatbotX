//! Instagram story reply / mention automations (`story-automation-dispatch.ts`
//! and `story-reply-automation/index.ts`): an inbound story reply or mention
//! queues `processStoryReplyAutomation` / `processStoryMentionAutomation` on
//! the AI queue, which answers through the chat / integration queues.
//!
//! An `AIAgent` reply runs `generateAIReplyText` (`low::comment_ai`).

use chatbotx_core::utils::normalize_for_match::normalize_for_match as norm;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::inbound::{Stored, add_job};
use crate::{Deps, db_fail, id_of, str_of};

pub const PROCESS_STORY_REPLY: &str = "processStoryReplyAutomation";
pub const PROCESS_STORY_MENTION: &str = "processStoryMentionAutomation";

/// `getStoryReply`.
fn story_reply(attributes: &Value) -> Option<&Value> {
    let story = if attributes["type"] == "story_reply" {
        &attributes["story"]
    } else {
        &attributes["storyReply"]
    };
    story.is_object().then_some(story)
}

/// `dispatchStoryAutomation`: `true` = a story automation was queued (the
/// message takes no other automation).
pub(crate) async fn dispatch(
    deps: &Deps,
    stored: &Stored,
    message: &Value,
    message_id: i64,
) -> Result<bool, JobError> {
    if stored.inbox.channel != "instagram" {
        return Ok(false);
    }
    let attributes = &message["contentAttributes"];
    let reply = story_reply(attributes);
    let mention = reply.is_none() && attributes["type"] == "story_mention";
    if reply.is_none() && !mention {
        return Ok(false);
    }
    let conversation = &stored.conversation;
    if mention {
        let additional = conversation
            .additional_attributes
            .as_ref()
            .map(|j| j.0.clone())
            .unwrap_or(Value::Null);
        let run =
            crate::integration::should_run_story_mention_automation(&additional, async || {
                crate::inbound::ensure_active(deps, conversation).await
            })
            .await;
        if !run {
            return Ok(false);
        }
    }
    let channel_type = instagram_channel_type(deps, stored.inbox.inbox_id).await?;
    let mut data = json!({
        "workspaceId": conversation.workspace_id.to_string(),
        "conversationId": conversation.id.to_string(),
        "contactInboxId": stored.contact_inbox.id.to_string(),
        "messageId": message_id.to_string(),
        "channelType": channel_type,
    });
    let (name, job_id) = match reply {
        Some(story) => {
            data["storyId"] = story["id"].clone();
            data["storyUrl"] = story["url"].clone();
            data["message"] = message["text"].clone();
            (
                PROCESS_STORY_REPLY,
                format!("story-reply-auto-{message_id}"),
            )
        }
        None => (
            PROCESS_STORY_MENTION,
            format!("story-mention-auto-{message_id}"),
        ),
    };
    add_job(
        deps,
        QueueName::AiAgent,
        name,
        json!({ "type": name, "data": data }),
        JobOptions {
            job_id: Some(job_id),
            ..JobOptions::default()
        },
    )
    .await?;
    Ok(true)
}

/// `instagram` or `instagramFacebook` (the integration row's login type).
async fn instagram_channel_type(deps: &Deps, inbox_id: i64) -> Result<&'static str, JobError> {
    let kind: Option<String> = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "type" FROM "IntegrationInstagram" WHERE "inboxId" = ?1 LIMIT 1"#,
                    [inbox_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })
        .await
        .map_err(db_fail)?;
    Ok(if kind.as_deref() == Some("facebook") {
        "instagramFacebook"
    } else {
        "instagram"
    })
}

/// `matchStory`.
fn match_story(story: &Value, story_id: &str) -> bool {
    story["type"] != "storyIds"
        || story["value"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|v| v.as_str() == Some(story_id))
}

/// `matchIncludeKeywords`.
fn match_include_keywords(include: &Value, message: Option<&str>) -> bool {
    let kind = include["type"].as_str().unwrap_or("all");
    if kind == "all" {
        return true;
    }
    let text = norm(message.unwrap_or(""));
    let keywords: Vec<String> = include["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(norm)
        .collect();
    if kind == "equal" {
        return keywords.contains(&text);
    }
    keywords.iter().any(|k| text.contains(k.as_str()))
}

/// One `IgStoryAutomation` row.
struct StoryAutomation {
    id: i64,
    start_time: Option<String>,
    end_time: Option<String>,
    story: Value,
    reply: Value,
    include_keywords: Value,
}

/// `processStoryReplyAutomation` / `processStoryMentionAutomation`
/// (`runStoryAutomations`).
pub async fn process(deps: &Deps, data: &Value, trigger: &'static str) -> Result<(), JobError> {
    let (Some(ws), Some(conversation_id), Some(contact_inbox_id)) = (
        id_of(&data["workspaceId"]),
        id_of(&data["conversationId"]),
        id_of(&data["contactInboxId"]),
    ) else {
        return Err(JobError::unrecoverable("Invalid story automation data"));
    };
    let channel_type = match str_of(data, "channelType") {
        Some("instagramFacebook") => "instagramFacebook",
        _ => "instagram",
    };
    let loaded = deps
        .db
        .read(move |conn| {
            // The contact inbox and the conversation must both be the workspace's.
            let inbox_id: Option<i64> = conn
                .query_row(
                    r#"SELECT ci."inboxId" FROM "ContactInbox" ci
                       JOIN "Conversation" c ON c."id" = ?2 AND c."workspaceId" = ?3
                       JOIN "Inbox" i ON i."id" = ci."inboxId" AND i."workspaceId" = ?3
                       WHERE ci."id" = ?1"#,
                    params![contact_inbox_id, conversation_id, ws],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(inbox_id) = inbox_id else {
                return Ok(None);
            };
            let tz: Option<String> = conn
                .query_row(
                    r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            let mut stmt = conn.prepare(
                r#"SELECT "id", "startTime", "endTime", "story", "reply", "includeKeywords"
                   FROM "IgStoryAutomation"
                   WHERE "workspaceId" = ?1 AND "isActive" = 1 AND "type" = ?2 AND "trigger" = ?3
                     AND ("inboxId" IS NULL OR "inboxId" = ?4)
                   ORDER BY "id""#,
            )?;
            let rows = stmt
                .query_map(params![ws, channel_type, trigger, inbox_id], |r| {
                    let json = |i: usize| -> chatbotx_db::rusqlite::Result<Value> {
                        Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null))
                    };
                    Ok(StoryAutomation {
                        id: r.get(0)?,
                        start_time: r.get(1)?,
                        end_time: r.get(2)?,
                        story: json(3)?,
                        reply: json(4)?,
                        include_keywords: json(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some((tz.unwrap_or_else(|| "UTC".to_owned()), rows)))
        })
        .await
        .map_err(db_fail)?;
    let Some((tz, automations)) = loaded else {
        tracing::warn!(
            contact_inbox_id,
            workspace_id = ws,
            "{trigger} automation skipped: contactInbox not found"
        );
        return Ok(());
    };
    let story_id = str_of(data, "storyId").unwrap_or("");
    let message = str_of(data, "message");
    for automation in automations {
        let skipped = if !crate::low::within_schedule(
            automation.start_time.as_deref(),
            automation.end_time.as_deref(),
            &tz,
        ) {
            Some("outside schedule")
        } else if trigger == STORY_REPLY && !match_story(&automation.story, story_id) {
            Some("story does not match")
        } else if trigger == STORY_REPLY
            && !match_include_keywords(&automation.include_keywords, message)
        {
            Some("keywords do not match")
        } else {
            None
        };
        if let Some(reason) = skipped {
            tracing::info!(
                automation_id = automation.id,
                reason,
                "{trigger} automation skipped"
            );
            continue;
        }
        let value = automation.reply["value"].as_str().filter(|v| !v.is_empty());
        let dispatched = match (automation.reply["type"].as_str(), value) {
            (Some("text"), Some(raw)) => {
                let text = render(deps, conversation_id, contact_inbox_id, raw).await;
                add_job(
                    deps,
                    QueueName::Chat,
                    crate::chat::SEND_CHAT_MESSAGE,
                    json!({ "type": crate::chat::SEND_CHAT_MESSAGE, "data": {
                        "conversation": { "id": conversation_id.to_string(), "workspaceId": ws.to_string() },
                        "contactInbox": { "id": contact_inbox_id.to_string() },
                        "text": text,
                    }}),
                    JobOptions::default(),
                )
                .await?;
                true
            }
            (Some("flow"), Some(flow_id)) => {
                add_job(
                    deps,
                    QueueName::Integration,
                    crate::flow::SEND_FLOW,
                    json!({ "type": crate::flow::SEND_FLOW, "data": {
                        "conversationId": conversation_id.to_string(),
                        "contactInboxId": contact_inbox_id.to_string(),
                        "flowId": flow_id,
                        "origin": "channel",
                    }}),
                    JobOptions::default(),
                )
                .await?;
                true
            }
            (Some("AIAgent"), Some(agent_id)) => {
                // A mention usually has no text: the agent is told what happened.
                let ai_message = if trigger == STORY_MENTION {
                    STORY_MENTION_AI_MESSAGE
                } else {
                    message.unwrap_or("")
                };
                match crate::low::story_reply_text(
                    deps,
                    ws,
                    conversation_id,
                    contact_inbox_id,
                    agent_id,
                    ai_message,
                )
                .await
                {
                    Ok(Ok(text)) => {
                        add_job(
                            deps,
                            QueueName::Chat,
                            crate::chat::SEND_CHAT_MESSAGE,
                            json!({ "type": crate::chat::SEND_CHAT_MESSAGE, "data": {
                                "conversation": { "id": conversation_id.to_string(), "workspaceId": ws.to_string() },
                                "contactInbox": { "id": contact_inbox_id.to_string() },
                                "text": text,
                            }}),
                            JobOptions::default(),
                        )
                        .await?;
                        true
                    }
                    Ok(Err(reason)) => {
                        tracing::info!(
                            automation_id = automation.id,
                            reason,
                            "{trigger} automation skipped"
                        );
                        false
                    }
                    Err(err) => {
                        tracing::error!(
                            ?err,
                            automation_id = automation.id,
                            workspace_id = ws,
                            "Failed to process {trigger} automation"
                        );
                        false
                    }
                }
            }
            _ => false,
        };
        if dispatched {
            let id = automation.id;
            deps.db
                .write(move |tx| {
                    Ok(tx.execute(
                        r#"UPDATE "IgStoryAutomation" SET "repliesCount" = "repliesCount" + 1 WHERE "id" = ?1"#,
                        [id],
                    )?)
                })
                .await
                .map_err(db_fail)?;
        }
    }
    Ok(())
}

/// `STORY_MENTION_AI_MESSAGE`.
const STORY_MENTION_AI_MESSAGE: &str =
    "The contact mentioned your account in their Instagram story.";

/// The `trigger` column values.
pub const STORY_REPLY: &str = "storyReply";
pub const STORY_MENTION: &str = "storyMention";

/// Spintax, then the contact variables (raw text when they fail).
async fn render(deps: &Deps, conversation_id: i64, contact_inbox_id: i64, raw: &str) -> String {
    let spun = chatbotx_core::utils::spintax::apply_spintax(raw);
    let Some(vars) = &deps.variables else {
        return spun;
    };
    let Ok(Some((conversation, contact_inbox))) = crate::chat::load_pair(
        deps,
        &json!({ "conversationId": conversation_id, "contactInboxId": contact_inbox_id }),
    )
    .await
    else {
        return spun;
    };
    match vars
        .resolve(
            &conversation,
            &contact_inbox,
            &Value::String(spun.clone()),
            None,
            None,
            false,
        )
        .await
    {
        Ok(Value::String(text)) => text,
        _ => spun,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_matching_follows_the_original() {
        assert!(story_reply(&json!({"type": "story_reply", "story": {"id": "s1"}})).is_some());
        assert!(story_reply(&json!({"storyReply": {"id": "s1"}})).is_some());
        assert!(story_reply(&json!({"type": "story_mention"})).is_none());
        assert!(match_story(&json!({"type": "all"}), "s9"));
        assert!(match_story(
            &json!({"type": "storyIds", "value": ["s1"]}),
            "s1"
        ));
        assert!(!match_story(
            &json!({"type": "storyIds", "value": ["s1"]}),
            "s2"
        ));
        assert!(match_include_keywords(&json!({"type": "all"}), None));
        assert!(match_include_keywords(
            &json!({"type": "contain", "value": ["Price"]}),
            Some("what PRICE")
        ));
        assert!(!match_include_keywords(
            &json!({"type": "equal", "value": ["price"]}),
            Some("what price")
        ));
    }
}
