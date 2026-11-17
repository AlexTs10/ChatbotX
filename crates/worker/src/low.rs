//! The `low` queue (`low/worker.ts`): `replayMissedComment` (a comment the
//! missed-comment scan queued, `receiveComment` with `replay` +
//! `processCommentAutomation` with `onlyAutomationId`), counted down on its
//! run whatever happens.
//!
//! ponytail: Messenger only, and only the text replies. The Messenger
//! channel (Q.I1b) is not in Rust, so the replies go straight to Graph from
//! here instead of the chat queue; AI moderation, hide / like, image and
//! mention filters, flow / AI replies, the private-reply blocked-reason
//! events and the leaderboard award wait for Q.I1b. The other low actions
//! (`contactMarkAsRead`, `messageStatus`, echoes, avatars, coexist
//! downloads) fail unrecoverable until their channels exist.

use chatbotx_business::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use chatbotx_db::repo::message::{self as repo_message, CreateMessageInput};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingContact};
use crate::{Deps, db_fail, id_of, str_of};

const GRAPH: &str = "https://graph.facebook.com";
/// `MAX_ERROR_DETAIL_LENGTH`.
const MAX_ERROR_DETAIL: usize = 1000;

/// The low worker processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    let data = &job.data["data"];
    if kind != "replayMissedComment" {
        return Err(JobError::unrecoverable(format!(
            "Unhandled low job type: {kind}"
        )));
    }
    let result = match crate::is_blocked_workspace(
        &deps.db,
        str_of(data, "workspaceId").unwrap_or(""),
    )
    .await
    {
        Ok(true) => Ok(()),
        Ok(false) => replay_missed_comment(&deps, &HttpUpstream, data).await,
        Err(err) => Err(err),
    };
    // `settleMissedCommentReplay`: never fails the job.
    if let Some(automation_id) = id_of(&data["replay"]["automationId"]) {
        finish_missed_comment_replay(&deps, automation_id).await;
    }
    result.map(|()| Value::Null)
}

/// `finishMissedCommentReplay`.
async fn finish_missed_comment_replay(deps: &Deps, automation_id: i64) {
    let kv = crate::inbound::kv(deps);
    let key = format!("comment-automation:missed-comments:remaining:{automation_id}");
    match kv.increment_counter(&key, -1, None).await {
        Ok(Some(remaining)) if remaining <= 0 => {
            if let Err(err) = kv.delete(&[&key]).await {
                tracing::error!(%err, automation_id, "Failed to count down a missed-comment replay");
            }
        }
        Ok(_) => {}
        Err(err) => {
            tracing::error!(%err, automation_id, "Failed to count down a missed-comment replay")
        }
    }
}

/// The comment of a `replayMissedComment` job.
struct Comment {
    id: String,
    post_id: String,
    parent_id: Option<String>,
    from_id: String,
    from_name: Option<String>,
    message: Option<String>,
    created_ms: i64,
}

/// The Messenger page of the comment.
struct Page {
    inbox: InboundInbox,
    token: String,
    version: String,
}

async fn load_page(deps: &Deps, page_id: &str) -> Result<Option<Page>, JobError> {
    let page_id = page_id.to_owned();
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT m."inboxId", m."workspaceId", m."auth", i."channel"
                       FROM "IntegrationMessenger" m JOIN "Inbox" i ON i."id" = m."inboxId"
                       WHERE m."pageId" = ?1 ORDER BY m."id" LIMIT 1"#,
                    [&page_id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?
                .map(|(inbox_id, workspace_id, auth, channel)| {
                    let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
                    Page {
                        inbox: InboundInbox {
                            inbox_id,
                            workspace_id,
                            channel,
                        },
                        token: auth["tokens"]["accessToken"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        version: auth["metadata"]["version"]
                            .as_str()
                            .unwrap_or(crate::channels::whatsapp::DEFAULT_API_VERSION)
                            .to_owned(),
                    }
                }))
        })
        .await
        .map_err(db_fail)
}

/// `receiveComment` for a replay (Messenger).
pub(crate) async fn replay_missed_comment(
    deps: &Deps,
    upstream: &impl Upstream,
    data: &Value,
) -> Result<(), JobError> {
    let Some(automation_id) = id_of(&data["replay"]["automationId"]) else {
        return Err(JobError::unrecoverable("Invalid replayMissedComment data"));
    };
    receive_comment(deps, upstream, data, Some(automation_id)).await
}

/// The `incomingComment` integration job (`receiveComment` +
/// `processCommentAutomation` over every active automation of the account).
///
/// ponytail: Messenger only (the automation runner posts to Graph); the
/// Instagram / Threads / TikTok comment runners wait for their channel
/// reply paths (specs/Q.D10.md "Left").
pub async fn incoming_comment(deps: &Deps, data: &Value) -> Result<(), JobError> {
    receive_comment(deps, &HttpUpstream, data, None).await
}

/// `receiveComment`; `only` = the replayed automation (`onlyAutomationId`).
async fn receive_comment(
    deps: &Deps,
    upstream: &impl Upstream,
    data: &Value,
    only: Option<i64>,
) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("");
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
    let c = &data["commentData"];
    let comment = Comment {
        id: str_of(c, "commentId").unwrap_or("").to_owned(),
        post_id: str_of(c, "postId").unwrap_or("").to_owned(),
        parent_id: str_of(c, "parentId").map(str::to_owned),
        from_id: str_of(c, "fromId").unwrap_or("").to_owned(),
        from_name: str_of(c, "fromName").map(str::to_owned),
        message: str_of(c, "message").map(str::to_owned),
        created_ms: c["createdTime"].as_i64().unwrap_or(0) * 1000,
    };
    if comment.from_id == identifier {
        tracing::info!(
            comment_id = comment.id,
            "receiveComment: skipping self-authored comment"
        );
        return Ok(());
    }
    if integration_type != "messenger" {
        if only.is_none() {
            return Err(JobError::unrecoverable(format!(
                "incomingComment: {integration_type} comment automations are not ported"
            )));
        }
        tracing::warn!(integration_type, "replayMissedComment: channel not ported");
        return Ok(());
    }
    let Some(page) = load_page(deps, identifier).await? else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
    };
    let contact = IncomingContact {
        source_id: comment.from_id.clone(),
        source_conversation_id: Some(comment.post_id.clone()),
        first_name: comment.from_name.clone(),
        ..IncomingContact::default()
    };
    let (contact_inbox, conversation, _) =
        crate::inbound::detect_contact_and_conversation(deps, &page.inbox, &contact, "comments")
            .await?;

    // The comment's own message row, dated at the comment.
    let parent_since = comment.created_ms;
    let (comment_message_id, is_new) = store_message(
        deps,
        &contact_inbox,
        &conversation,
        CreateMessageInput {
            id: Some(new_id()),
            contact_inbox_id: contact_inbox.id,
            conversation_id: conversation.id,
            workspace_id: page.inbox.workspace_id,
            content_attributes: Some(json!({ "postId": comment.post_id })),
            content_type: "text".into(),
            message_type: "incoming".into(),
            sender_type: "contact".into(),
            sender_id: Some(contact_inbox.contact_id),
            source_id: Some(comment.id.clone()),
            parent_id: None,
            text: comment.message.clone(),
            r#type: Some("comment".into()),
            created_at: Some(parent_since),
            updated_at: None,
        },
    )
    .await?;

    if !crate::inbound::workspace_is_active(deps, page.inbox.workspace_id).await? {
        return Ok(());
    }
    let automations = match only {
        Some(id) => vec![id],
        // ponytail: a redelivered comment (row already stored) is not run
        // again; the original re-enqueues under the deduplicating job id.
        None if !is_new => return Ok(()),
        None => active_automations(deps, &page.inbox).await?,
    };
    for automation_id in automations {
        process_comment_automation(
            deps,
            upstream,
            &page,
            &contact_inbox,
            &conversation,
            &comment,
            comment_message_id,
            automation_id,
        )
        .await?;
    }
    Ok(())
}

/// `findActiveAutomations`: the account's own plus the unscoped ones.
async fn active_automations(deps: &Deps, inbox: &InboundInbox) -> Result<Vec<i64>, JobError> {
    let (ws, inbox_id) = (inbox.workspace_id, inbox.inbox_id);
    deps.db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "id" FROM "CommentAutomation"
                   WHERE "workspaceId" = ?1 AND "isActive" = 1 AND "type" = 'messenger'
                     AND ("inboxId" IS NULL OR "inboxId" = ?2) ORDER BY "id""#,
            )?;
            let ids = stmt
                .query_map(params![ws, inbox_id], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
        .await
        .map_err(db_fail)
}

/// `messageService.create` for a comment row; `(id, is_new)`.
async fn store_message(
    deps: &Deps,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    input: CreateMessageInput,
) -> Result<(i64, bool), JobError> {
    let (row, id, is_new) = deps
        .db
        .write(move |tx| {
            let r = repo_message::create_or_update(tx, &input).map_err(crate::repo_err)?;
            let stored = repo_message::MessageWithAttachments {
                message: r.message,
                attachment_count: None,
                attachments: Vec::new(),
            };
            let id = stored.message.id;
            Ok((
                chatbotx_business::message::message_json(&stored),
                id,
                r.is_new,
            ))
        })
        .await
        .map_err(db_fail)?;
    if is_new {
        crate::chat::queue_event(
            deps,
            conversation,
            contact_inbox.inbox_id,
            chatbotx_rt::EventType::MessageCreated,
            row,
        )
        .await;
    }
    Ok((id, is_new))
}

/// One automation row (`findActiveAutomations`, narrowed to the replay's).
struct Automation {
    id: i64,
    start_time: Option<String>,
    end_time: Option<String>,
    post: Value,
    private_reply: Value,
    public_reply: Value,
    include_keywords: Value,
    exclude_keywords: Vec<String>,
    exclude_keywords_type: String,
    options: Value,
}

/// `normalizePostId`.
fn normalize_post_id(id: &str) -> &str {
    id.split_once('_').map_or(id, |(_, rest)| rest)
}

/// `matchPost` (no live comments in a replay).
fn match_post(post: &Value, post_id: &str) -> bool {
    match post["type"].as_str() {
        Some("live") => false,
        Some("postIds") => post["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|v| v == post_id || normalize_post_id(v) == normalize_post_id(post_id)),
        _ => true,
    }
}

/// `matchKeywords`.
fn match_keywords(
    include: &Value,
    exclude: &[String],
    message: Option<&str>,
    exclude_type: &str,
) -> bool {
    use chatbotx_core::utils::normalize_for_match::normalize_for_match as norm;
    let text = norm(message.unwrap_or(""));
    let kws: Vec<String> = include["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(norm)
        .collect();
    match include["type"].as_str() {
        Some("equal") if !kws.is_empty() && !kws.contains(&text) => return false,
        Some("contain") if !kws.is_empty() && !kws.iter().any(|k| text.contains(k.as_str())) => {
            return false;
        }
        _ => {}
    }
    let excluded: Vec<String> = exclude
        .iter()
        .map(|k| norm(k.trim()))
        .filter(|k| !k.is_empty())
        .collect();
    if exclude_type == "equal" {
        return !excluded.contains(&text.trim().to_owned());
    }
    !excluded.iter().any(|k| text.contains(k.as_str()))
}

/// `isCommentReply`.
fn is_comment_reply(parent_id: Option<&str>, post_id: &str, comment_id: &str) -> bool {
    let Some(parent) = parent_id.filter(|p| !p.is_empty()) else {
        return false;
    };
    let parent = normalize_post_id(parent);
    if parent == normalize_post_id(post_id) {
        return false;
    }
    parent
        != comment_id
            .split_once('_')
            .map_or(comment_id, |(object, _)| object)
}

/// `resolveReplyTexts`.
fn reply_texts(reply: &Value) -> Vec<String> {
    let raw: Vec<String> = match reply["values"].as_array() {
        Some(values) => values
            .iter()
            .map(|v| v["value"].as_str().unwrap_or("").to_owned())
            .collect(),
        None => vec![reply["value"].as_str().unwrap_or("").to_owned()],
    };
    raw.into_iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

/// `isWithinSchedule` (`HH:mm` in the workspace time zone).
fn within_schedule(start: Option<&str>, end: Option<&str>, tz: &str) -> bool {
    let (Some(start), Some(end)) = (
        start.filter(|s| !s.is_empty()),
        end.filter(|s| !s.is_empty()),
    ) else {
        return true;
    };
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    let now = chrono::Utc::now()
        .with_timezone(&zone)
        .format("%H:%M")
        .to_string();
    let now = now.as_str();
    if start <= end {
        now >= start && now <= end
    } else {
        now >= start || now <= end
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_comment_automation(
    deps: &Deps,
    upstream: &impl Upstream,
    page: &Page,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    comment: &Comment,
    comment_message_id: i64,
    automation_id: i64,
) -> Result<(), JobError> {
    let ws = page.inbox.workspace_id;
    let inbox_id = page.inbox.inbox_id;
    let loaded = deps
        .db
        .read(move |conn| {
            let tz: Option<String> = conn
                .query_row(r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#, [ws], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten();
            let automation = conn
                .query_row(
                    r#"SELECT "id", "startTime", "endTime", "post", "privateReply", "publicReply",
                              "includeKeywords", "excludeKeywords", "excludeKeywordsType", "options"
                       FROM "CommentAutomation"
                       WHERE "id" = ?1 AND "workspaceId" = ?2 AND "isActive" = 1 AND "type" = 'messenger'
                         AND ("inboxId" IS NULL OR "inboxId" = ?3)"#,
                    params![automation_id, ws, inbox_id],
                    |r| {
                        let json = |i: usize| -> chatbotx_db::rusqlite::Result<Value> {
                            Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null))
                        };
                        Ok(Automation {
                            id: r.get(0)?,
                            start_time: r.get(1)?,
                            end_time: r.get(2)?,
                            post: json(3)?,
                            private_reply: json(4)?,
                            public_reply: json(5)?,
                            include_keywords: json(6)?,
                            exclude_keywords: serde_json::from_value(json(7)?).unwrap_or_default(),
                            exclude_keywords_type: r.get(8)?,
                            options: json(9)?,
                        })
                    },
                )
                .optional()?;
            Ok((tz.unwrap_or_else(|| "UTC".to_owned()), automation))
        })
        .await
        .map_err(db_fail)?;
    let (tz, Some(automation)) = loaded else {
        tracing::info!(
            automation_id,
            comment_id = comment.id,
            "Missed comment replay skipped: automation no longer active"
        );
        return Ok(());
    };
    let option = |name: &str| automation.options[name].as_bool();
    let contact_id = contact_inbox.contact_id;
    let post_id = comment.post_id.clone();

    let miss = if !within_schedule(
        automation.start_time.as_deref(),
        automation.end_time.as_deref(),
        &tz,
    ) {
        Some("outsideSchedule")
    } else if !match_post(&automation.post, &comment.post_id) {
        Some("postNotMatched")
    } else if option("ignoreCommentReplies").unwrap_or(true)
        && is_comment_reply(comment.parent_id.as_deref(), &comment.post_id, &comment.id)
    {
        Some("commentIsReply")
    } else if !match_keywords(
        &automation.include_keywords,
        &automation.exclude_keywords,
        comment.message.as_deref(),
        &automation.exclude_keywords_type,
    ) {
        Some("keywordsNotMatched")
    } else {
        let new_only = option("replyToNewContactsOnly").unwrap_or(false);
        let once = option("replyOncePerUserPerPost").unwrap_or(false);
        let other_posts = option("replyToUsersWhoCommentedOnOtherPosts").unwrap_or(true);
        let pid = post_id.clone();
        deps.db
            .write(move |tx| {
                if new_only {
                    let count: i64 = tx.query_row(
                        r#"SELECT COUNT(*) FROM "ContactInbox" WHERE "contactId" = ?1"#,
                        [contact_id],
                        |r| r.get(0),
                    )?;
                    if count > 1 {
                        return Ok(Some("contactNotNew"));
                    }
                }
                if !other_posts {
                    let elsewhere: Option<i64> = tx
                        .query_row(
                            r#"SELECT 1 FROM "CommentAutomationReply"
                               WHERE "automationId" = ?1 AND "contactId" = ?2 AND "postId" != ?3 LIMIT 1"#,
                            params![automation_id, contact_id, pid],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if elsewhere.is_some() {
                        return Ok(Some("engagedOnOtherPost"));
                    }
                }
                if once {
                    // `claimDedup`: the insert enforces once per user per post.
                    let claimed = tx.execute(
                        r#"INSERT INTO "CommentAutomationReply" ("id", "automationId", "contactId", "postId", "workspaceId")
                           VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
                        params![new_id(), automation_id, contact_id, pid, ws],
                    )?;
                    if claimed == 0 {
                        return Ok(Some("alreadyRepliedOnPost"));
                    }
                }
                Ok(None)
            })
            .await
            .map_err(db_fail)?
    };
    let occurred_at = comment.created_ms;
    if let Some(reason) = miss {
        tracing::info!(
            automation_id,
            comment_id = comment.id,
            reason,
            "Comment automation skipped"
        );
        let (cid, ciid, text) = (
            comment.id.clone(),
            contact_inbox.id,
            comment.message.clone(),
        );
        deps.db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO "CommentAutomationMiss" ("id", "workspaceId", "automationId", "contactId",
                           "contactInboxId", "postId", "commentId", "commentText", "reason", "occurredAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT DO NOTHING"#,
                    params![new_id(), ws, automation_id, contact_id, ciid, post_id, cid, text, reason, occurred_at],
                )?;
                tx.execute(
                    r#"UPDATE "CommentAutomation" SET "missedCount" = "missedCount" + 1 WHERE "id" = ?1"#,
                    [automation_id],
                )?;
                Ok(())
            })
            .await
            .map_err(db_fail)?;
        return Ok(());
    }

    let mut sent_any = false;
    // Public reply: each text as a comment under the customer's comment.
    if automation.public_reply["type"] == "text" {
        let texts = reply_texts(&automation.public_reply);
        if !texts.is_empty() {
            let mut sent = Vec::new();
            let mut failure = None;
            for raw in texts {
                let text = render(deps, conversation, contact_inbox, &raw).await;
                let url = format!("{GRAPH}/{}/{}/comments", page.version, comment.id);
                match graph_post(upstream, &page.token, url, json!({ "message": text })).await {
                    Ok(source_id) => {
                        store_reply(
                            deps,
                            contact_inbox,
                            conversation,
                            &text,
                            &comment.id,
                            automation.id,
                            "public",
                            source_id,
                            Some(comment_message_id),
                        )
                        .await?;
                        sent.push(text);
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            let outcome = match failure {
                None => Ok(sent.join("\n")),
                Some(error) if sent.is_empty() => Err(error),
                Some(_) => Ok(sent.join("\n")),
            };
            sent_any |= outcome.is_ok();
            record_event(
                deps,
                ws,
                &automation,
                contact_inbox,
                comment,
                "public",
                outcome,
            )
            .await?;
        }
    }

    // Private reply: one comment-anchored DM (text only, Meta allows one per
    // comment, within 7 days of it).
    if automation.private_reply["type"] == "text" {
        let texts = reply_texts(&automation.private_reply);
        let cid = comment.id.clone();
        let claimed: bool = deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "CommentAutomationEvent" WHERE "workspaceId" = ?1 AND "commentId" = ?2
                             AND "replyChannel" = 'private' AND "status" = 'sent' LIMIT 1"#,
                        params![ws, cid],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .map_err(db_fail)?;
        let expired = now_ms() - comment.created_ms > 7 * 86_400_000;
        if let Some(raw) = texts.first().filter(|_| !claimed && !expired) {
            let text = render(deps, conversation, contact_inbox, raw).await;
            let url = format!("{GRAPH}/{}/me/messages", page.version);
            let body =
                json!({ "recipient": { "comment_id": comment.id }, "message": { "text": text } });
            let outcome = match graph_post(upstream, &page.token, url, body).await {
                Ok(source_id) => {
                    store_reply(
                        deps,
                        contact_inbox,
                        conversation,
                        &text,
                        &comment.id,
                        automation.id,
                        "private",
                        source_id,
                        None,
                    )
                    .await?;
                    Ok(text)
                }
                Err(error) => Err(error),
            };
            sent_any |= outcome.is_ok();
            record_event(
                deps,
                ws,
                &automation,
                contact_inbox,
                comment,
                "private",
                outcome,
            )
            .await?;
        } else if !texts.is_empty() {
            tracing::info!(
                automation_id,
                comment_id = comment.id,
                claimed,
                expired,
                "Private reply not sent"
            );
        }
    }
    if sent_any {
        deps.db
            .write(move |tx| {
                Ok(tx.execute(
                    r#"UPDATE "CommentAutomation" SET "repliesCount" = "repliesCount" + 1 WHERE "id" = ?1"#,
                    [automation_id],
                )?)
            })
            .await
            .map_err(db_fail)?;
    }
    Ok(())
}

/// Spintax, then the contact variables (raw text when they fail).
async fn render(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    raw: &str,
) -> String {
    let spun = chatbotx_core::utils::spintax::apply_spintax(raw);
    let Some(vars) = &deps.variables else {
        return spun;
    };
    match vars
        .resolve(
            conversation,
            contact_inbox,
            &Value::String(spun.clone()),
            None,
            None,
            false,
        )
        .await
    {
        Ok(Value::String(text)) => text,
        Ok(_) => spun,
        Err(err) => {
            tracing::warn!(err = %err, "Failed to resolve variables in reply text, sending raw text");
            spun
        }
    }
}

/// A Graph POST; `Ok(id)` of the created object, `Err((detail, http code))`.
async fn graph_post(
    upstream: &impl Upstream,
    token: &str,
    url: String,
    body: Value,
) -> Result<Option<String>, (String, Option<String>)> {
    let reply = upstream
        .send(UpstreamRequest {
            method: "POST",
            url,
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some(body.to_string()),
        })
        .await;
    match reply {
        Ok((status, text)) if (200..300).contains(&status) => {
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            Ok(body["id"]
                .as_str()
                .or(body["message_id"].as_str())
                .map(str::to_owned))
        }
        Ok((status, text)) => {
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let detail = body["error"]["message"]
                .as_str()
                .map_or(text.clone(), str::to_owned);
            Err((detail, Some(status.to_string())))
        }
        Err(error) => Err((error, None)),
    }
}

/// The outgoing reply's message row (`postPublicCommentReply` / the DM).
#[allow(clippy::too_many_arguments)]
async fn store_reply(
    deps: &Deps,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    text: &str,
    comment_id: &str,
    automation_id: i64,
    channel: &str,
    source_id: Option<String>,
    parent: Option<i64>,
) -> Result<(), JobError> {
    let public = channel == "public";
    store_message(
        deps,
        contact_inbox,
        conversation,
        CreateMessageInput {
            id: Some(new_id()),
            contact_inbox_id: contact_inbox.id,
            conversation_id: conversation.id,
            workspace_id: conversation.workspace_id,
            content_attributes: Some(json!({
                "replyToCommentId": comment_id,
                "commentAutomation": { "automationId": automation_id.to_string(), "replyChannel": channel },
            })),
            content_type: "text".into(),
            message_type: "outgoing".into(),
            sender_type: "bot".into(),
            sender_id: None,
            source_id,
            parent_id: parent.filter(|_| public).map(|id| id.to_string()),
            text: Some(text.to_owned()),
            r#type: Some(if public { "comment" } else { "message" }.into()),
            created_at: Some(now_ms()),
            updated_at: None,
        },
    )
    .await
    .map(|_| ())
}

/// `recordReplyEvent` / `recordReplyFailure` (one row per automation,
/// comment and reply channel).
async fn record_event(
    deps: &Deps,
    ws: i64,
    automation: &Automation,
    contact_inbox: &ContactInbox,
    comment: &Comment,
    channel: &'static str,
    outcome: Result<String, (String, Option<String>)>,
) -> Result<(), JobError> {
    let (automation_id, contact_id, ciid) =
        (automation.id, contact_inbox.contact_id, contact_inbox.id);
    let (post_id, comment_id, text, occurred) = (
        comment.post_id.clone(),
        comment.id.clone(),
        comment.message.clone(),
        comment.created_ms,
    );
    let (status, reply_text, detail, http) = match outcome {
        Ok(text) => ("sent", Some(text), None, None),
        Err((detail, http)) => {
            tracing::error!(
                automation_id,
                comment_id,
                detail,
                "Failed to send {channel} reply"
            );
            (
                "failed",
                None,
                Some(detail.chars().take(MAX_ERROR_DETAIL).collect::<String>()),
                http,
            )
        }
    };
    deps.db
        .write(move |tx| {
            let now = now_ms();
            let inserted = tx.execute(
                r#"INSERT INTO "CommentAutomationEvent" ("id", "workspaceId", "automationId", "contactId",
                       "contactInboxId", "postId", "commentId", "commentText", "replyChannel", "replyType",
                       "replyText", "status", "errorDetail", "httpCode", "failedAt", "occurredAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'text', ?10, ?11, ?12, ?13, ?14, ?15)
                   ON CONFLICT DO NOTHING"#,
                params![
                    new_id(), ws, automation_id, contact_id, ciid, post_id, comment_id, text, channel,
                    reply_text, status, detail, http, (status == "failed").then_some(now), occurred
                ],
            )?;
            if inserted > 0 {
                let column = if status == "sent" { "sentCount" } else { "failedCount" };
                tx.execute(
                    &format!(r#"UPDATE "CommentAutomation" SET "{column}" = "{column}" + 1 WHERE "id" = ?1"#),
                    [automation_id],
                )?;
            }
            Ok(())
        })
        .await
        .map_err(db_fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_rules_follow_the_original() {
        assert!(match_post(
            &json!({"type": "postIds", "value": ["1_22"]}),
            "9_22"
        ));
        assert!(!match_post(
            &json!({"type": "postIds", "value": ["1_22"]}),
            "1_23"
        ));
        assert!(match_post(&json!({"type": "all", "value": []}), "1_23"));
        assert!(!match_post(&json!({"type": "live"}), "1_23"));
        let all = json!({"type": "all", "value": []});
        assert!(match_keywords(
            &json!({"type": "contain", "value": ["PRÍCE"]}),
            &[],
            Some("what price?"),
            "contain"
        ));
        assert!(!match_keywords(
            &json!({"type": "equal", "value": ["price"]}),
            &[],
            Some("what price?"),
            "contain"
        ));
        assert!(!match_keywords(
            &all,
            &["spam".into()],
            Some("buy SPAM"),
            "contain"
        ));
        assert!(match_keywords(
            &all,
            &["spam".into()],
            Some("buy spam"),
            "equal"
        ));
        assert!(!is_comment_reply(Some("1_22"), "1_22", "22_5"));
        assert!(!is_comment_reply(Some("22"), "1_22", "22_5"));
        assert!(is_comment_reply(Some("22_4"), "1_22", "22_5"));
        assert_eq!(
            reply_texts(&json!({"type": "text", "value": " hi "})),
            vec!["hi"]
        );
        assert_eq!(
            reply_texts(&json!({"type": "text", "values": [{"value": "a"}, {"value": " "}]})),
            vec!["a"]
        );
        assert!(within_schedule(None, Some("10:00"), "UTC"));
        assert!(within_schedule(
            Some("00:00"),
            Some("23:59"),
            "Europe/Berlin"
        ));
    }

    /// p1-05 `07`: the replay stores the comment, posts the public reply on
    /// Graph and records the event a second scan skips.
    #[tokio::test]
    async fn a_replay_posts_the_public_reply_and_records_the_event() {
        let (deps, _) = crate::contact_scan::tests::setup().await;
        deps.db
            .write(|tx| {
                Ok(tx.execute(
                    r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "post", "publicReply")
                       VALUES (7, 'A', 2, 'messenger', '{"type":"postIds","value":["p1_55"]}', '{"type":"text","value":"Thanks {a|a}"}')"#,
                    [],
                )?)
            })
            .await
            .unwrap();
        let graph =
            crate::contact_scan::tests::Graph(Default::default(), json!({ "id": "reply-1" }));
        let data = json!({
            "workspaceId": "2", "integrationType": "messenger", "integrationIdentifier": "p1",
            "commentData": { "commentId": "55_9", "postId": "p1_55", "parentId": "p1_55", "fromId": "u7",
                "fromName": "Ana Lima", "message": "How much?", "createdTime": now_ms() / 1000 - 60 },
            "replay": { "automationId": "7" },
        });
        replay_missed_comment(&deps, &graph, &data).await.unwrap();
        let calls = graph.0.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].starts_with("POST https://graph.facebook.com/v23.0/55_9/comments "),
            "{}",
            calls[0]
        );
        assert!(calls[0].contains(r#""message":"Thanks a""#));
        let (events, sent, messages): (i64, i64, i64) = deps
            .db
            .read(|c| {
                Ok(c.query_row(
                    r#"SELECT (SELECT COUNT(*) FROM "CommentAutomationEvent" WHERE "commentId" = '55_9' AND "status" = 'sent'),
                              (SELECT "sentCount" FROM "CommentAutomation" WHERE "id" = 7),
                              (SELECT COUNT(*) FROM "Message" WHERE "type" = 'comment')"#,
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!((events, sent, messages), (1, 1, 2));
    }
}
