//! The `low` queue (`low/worker.ts`) `replayMissedComment`, and the comment
//! automation runner it shares with the `incomingComment` /
//! `processCommentAutomation` integration jobs (`receiveComment`,
//! `processCommentAutomation`): Messenger, Instagram (both logins), Threads
//! and TikTok, through the channel trait and the chat / integration queues
//! as the original.
//!
//! AI moderation and AI-agent replies run on the AI queue (`comment_ai`).
//!
//! ponytail: not ported (specs/Q.D10.md "Left"): the deferred TikTok private
//! reply, live-comment pacing, the leaderboard award, post
//! tracking, commenter avatars, comment attachments and user-tag tracking.
//! The other low actions (`contactMarkAsRead`, `messageStatus`, echoes,
//! avatars, coexist downloads) fail unrecoverable until their channels exist.

use std::sync::{Arc, LazyLock};

use chatbotx_business::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use chatbotx_core::utils::normalize_for_match::normalize_for_match as norm;
use chatbotx_db::repo::message::{self as repo_message, CreateMessageInput};
use chatbotx_db::repo::models::{ContactInbox, Conversation, Message};
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobCtx, JobError, JobOptions, QueueName};
use regex::Regex;
use serde_json::{Value, json};

use crate::channel::{
    Channel, ChannelCtx, InboundInbox, IncomingContact, OutgoingContact, SendFailure, SendMessage,
};
use crate::{Deps, db_fail, id_of, str_of};

/// `MAX_ERROR_DETAIL_LENGTH`.
const MAX_ERROR_DETAIL: usize = 1000;
/// `IntegrationJobAction.processCommentAutomation`.
pub const PROCESS_COMMENT_AUTOMATION: &str = "processCommentAutomation";
/// `SINGLE_ATTEMPT_COMMENT_AUTOMATION_CHANNELS` / `SINGLE_ATTEMPT_REPLY_CHANNELS`.
const SINGLE_ATTEMPT: &[&str] = &["threads", "tiktok"];
/// `PUBLIC_REPLY_SPACING_MS`.
const PUBLIC_REPLY_SPACING_MS: u64 = 3000;
const DAY_MS: i64 = 86_400_000;
/// `TIKTOK_HIGH_INTENT_ATTRIBUTE` (on the comment's `contentAttributes`).
const TIKTOK_HIGH_INTENT: &str = "tiktokHighIntent";

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

/// `receiveComment` for a replay.
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

/// The `incomingComment` integration job.
pub async fn incoming_comment(deps: &Deps, data: &Value) -> Result<(), JobError> {
    receive_comment(deps, &HttpUpstream, data, None).await
}

/// The `processCommentAutomation` integration job.
pub async fn process_comment_automation_job(deps: &Deps, data: &Value) -> Result<(), JobError> {
    process_comment_automation(deps, &HttpUpstream, data).await
}

/// The `tiktokHighIntentComment` integration job
/// (`receiveTiktokHighIntentComment`): flags the stored comment; a comment
/// not stored yet retries (`TiktokHighIntentCommentPendingError`).
pub async fn tiktok_high_intent_comment(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("tiktok");
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
    let (_, _, inbox) = resolve(deps, integration_type, identifier).await?;
    let comment_id = str_of(data, "commentId").unwrap_or("").to_owned();
    let mut overlay = serde_json::Map::new();
    overlay.insert(
        TIKTOK_HIGH_INTENT.to_owned(),
        json!({
            "at": chatbotx_db::repo::iso(now_ms()),
            "isFollower": data["isFollower"],
            "commentedAt": data["commentedAt"],
        }),
    );
    let (ws, cid) = (inbox.workspace_id, comment_id.clone());
    let merged = deps
        .db
        .write(move |tx| {
            repo_message::merge_content_attributes_by_source_id(tx, &cid, ws, &overlay)
                .map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    match merged {
        Some(message) => {
            tracing::info!(
                comment_id,
                workspace_id = ws,
                message_id = message.id,
                "Flagged a TikTok comment as high intent"
            );
            Ok(())
        }
        None => Err(JobError::retryable(format!(
            "TikTok high-intent comment {comment_id} is not stored yet"
        ))),
    }
}

/// `identifyInboxAndIntegrationAuthFromIdentifier`.
async fn resolve(
    deps: &Deps,
    integration_type: &str,
    identifier: &str,
) -> Result<(Arc<dyn Channel>, ChannelCtx, InboundInbox), JobError> {
    deps.channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            integration_type,
            identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
        .ok_or_else(|| {
            JobError::unrecoverable(format!(
                "Integration not found: {integration_type} {identifier}"
            ))
        })
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
    let comment_id = str_of(c, "commentId").unwrap_or("");
    let post_id = str_of(c, "postId").unwrap_or("");
    let from_id = str_of(c, "fromId").unwrap_or("");
    if from_id == identifier {
        tracing::info!(comment_id, "receiveComment: skipping self-authored comment");
        return Ok(());
    }
    let (_, ctx, inbox) = resolve(deps, integration_type, identifier).await?;
    // TikTok: the webhook has no commenter identity and no self-authorship
    // signal; an unknown author is stored but never answered.
    let identity = if integration_type == "tiktok" {
        crate::channels::tiktok::commenter_identity(
            crate::channels::tiktok::BUSINESS_API_URL,
            &ctx.auth,
            comment_id,
            post_id,
        )
        .await
    } else {
        None
    };
    if identity.as_ref().is_some_and(|i| i["isOwner"] == true) {
        tracing::info!(comment_id, "receiveComment: skipping self-authored comment");
        return Ok(());
    }
    let authorship_unknown = integration_type == "tiktok" && identity.is_none();
    let identity = identity.unwrap_or(Value::Null);
    // ponytail: a Messenger live comment is not detected (attachment lookup).
    let is_live = c["isLive"] == true;
    let contact = IncomingContact {
        source_id: from_id.to_owned(),
        source_conversation_id: Some(post_id.to_owned()),
        first_name: str_of(&identity, "displayName")
            .or(str_of(c, "fromName"))
            .map(str::to_owned),
        source_username: str_of(&identity, "username")
            .map(str::to_lowercase)
            .or_else(|| str_of(c, "fromUsername").map(str::to_owned)),
        ..IncomingContact::default()
    };
    let source = if is_live { "liveComments" } else { "comments" };
    let (contact_inbox, conversation, _) =
        crate::inbound::detect_contact_and_conversation(deps, &inbox, &contact, source).await?;

    let created_time = c["createdTime"].as_i64().unwrap_or(0);
    let parent_id = match str_of(c, "parentId") {
        Some(parent) => find_by_source(deps, parent, &conversation, now_ms() - 90 * DAY_MS)
            .await?
            .map(|m| m.id.to_string()),
        None => None,
    };
    let mut attributes = json!({ "postId": post_id });
    if is_live {
        attributes["isLiveComment"] = json!(true);
    }
    let (_, _, is_new) = store_message(
        deps,
        &contact_inbox,
        &conversation,
        CreateMessageInput {
            id: Some(new_id()),
            contact_inbox_id: contact_inbox.id,
            conversation_id: conversation.id,
            workspace_id: inbox.workspace_id,
            content_attributes: Some(attributes),
            content_type: "text".into(),
            message_type: "incoming".into(),
            sender_type: "contact".into(),
            sender_id: Some(contact_inbox.contact_id),
            source_id: Some(comment_id.to_owned()),
            parent_id,
            text: str_of(c, "message").map(str::to_owned),
            r#type: Some("comment".into()),
            // A replay is dated at the comment (it may be days old); else now.
            created_at: Some(only.map_or_else(now_ms, |_| created_time * 1000)),
            updated_at: None,
        },
    )
    .await?;
    if !is_new {
        tracing::info!(
            comment_id,
            integration_type,
            "receiveComment: comment already stored, re-checking automation enqueue"
        );
    }
    if authorship_unknown {
        tracing::warn!(
            comment_id,
            "receiveComment: TikTok commenter identity unresolved, withholding automation"
        );
        return Ok(());
    }
    if !crate::inbound::workspace_is_active(deps, inbox.workspace_id).await? {
        return Ok(());
    }
    let mut automation = json!({
        "integrationType": integration_type,
        "integrationIdentifier": identifier,
        "workspaceId": inbox.workspace_id.to_string(),
        "conversationId": conversation.id.to_string(),
        "contactInboxId": contact_inbox.id.to_string(),
        "commentId": comment_id,
        "postId": post_id,
        "parentId": c["parentId"],
        "fromId": from_id,
        "message": c["message"],
        "tags": c["tags"],
        "createdTime": created_time,
    });
    if is_live {
        automation["isLive"] = json!(true);
    }
    if let Some(id) = only {
        automation["onlyAutomationId"] = json!(id.to_string());
        // `isMissedCommentReplay`: carried into the AI moderation job.
        automation["replay"] = data["replay"].clone();
        return process_comment_automation(deps, upstream, &automation).await;
    }
    // ponytail: no live-comment pacing delay and no completed-job retention
    // age; the job id alone rejects a redelivered webhook.
    deps.jobs
        .add(
            QueueName::Integration,
            PROCESS_COMMENT_AUTOMATION,
            json!({ "type": PROCESS_COMMENT_AUTOMATION, "data": automation }),
            JobOptions {
                job_id: Some(format!("comment-auto-{comment_id}")),
                attempts: SINGLE_ATTEMPT.contains(&integration_type).then_some(1),
                ..JobOptions::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// `messageRepo.findBySourceId` in the comment's conversation.
async fn find_by_source(
    deps: &Deps,
    source_id: &str,
    conversation: &Conversation,
    since: i64,
) -> Result<Option<Message>, JobError> {
    let (source_id, conversation_id, ws) = (
        source_id.to_owned(),
        conversation.id,
        conversation.workspace_id,
    );
    deps.db
        .read(move |conn| {
            repo_message::find_by_source_id(conn, &source_id, conversation_id, ws, Some(since))
                .map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)
}

/// `messageService.create` for a comment row; `(row json, id, is_new)`.
async fn store_message(
    deps: &Deps,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    input: CreateMessageInput,
) -> Result<(Value, i64, bool), JobError> {
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
            row.clone(),
        )
        .await;
    }
    Ok((row, id, is_new))
}

/// The comment of a `processCommentAutomation` job.
struct Comment {
    id: String,
    post_id: String,
    parent_id: Option<String>,
    message: Option<String>,
    /// `createdTime` (unix seconds).
    created_time: i64,
    is_live: bool,
    tags: Value,
}

impl Comment {
    /// The comment fields of a `processCommentAutomation` payload.
    fn of(data: &Value) -> Comment {
        Comment {
            id: str_of(data, "commentId").unwrap_or("").to_owned(),
            post_id: str_of(data, "postId").unwrap_or("").to_owned(),
            parent_id: str_of(data, "parentId").map(str::to_owned),
            message: str_of(data, "message").map(str::to_owned),
            created_time: data["createdTime"].as_i64().unwrap_or(0),
            is_live: data["isLive"] == true,
            tags: data["tags"].clone(),
        }
    }

    fn created_ms(&self) -> i64 {
        self.created_time * 1000
    }
}

/// One `CommentAutomation` row.
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
    hide_comments: Value,
    reply_after: Value,
}

impl Automation {
    fn option(&self, name: &str) -> Option<bool> {
        self.options[name].as_bool()
    }

    /// `withLiveCapabilityLimits`: an Instagram live comment takes no public
    /// reply, like, hide, reply delay or reply filter.
    fn limit_live(&mut self, channel_type: &str) {
        if self.post["type"] != "live" || !is_instagram(channel_type) {
            return;
        }
        self.public_reply = json!({ "type": "none", "value": null });
        self.options["likeUserComment"] = json!(false);
        self.options["ignoreCommentReplies"] = json!(false);
        for rule in HIDE_FLAGS {
            self.hide_comments[rule] = json!(false);
        }
        self.reply_after = json!({ "type": "immediately", "value": 0 });
    }
}

const HIDE_FLAGS: [&str; 8] = [
    "all",
    "hasPhoneNumber",
    "hasImage",
    "hasVideo",
    "hasLink",
    "hasKeywords",
    "hasGif",
    "hasEmoji",
];

fn is_instagram(channel_type: &str) -> bool {
    matches!(channel_type, "instagram" | "instagramFacebook")
}

/// `PRIVATE_REPLY_TEXT_SENDERS[channel] !== null`.
fn supports_private_reply(channel_type: &str) -> bool {
    channel_type != "threads"
}

/// `privateReplyWindowMs` and its label.
fn private_reply_window(channel_type: &str, is_live: bool) -> (i64, &'static str) {
    if is_live && is_instagram(channel_type) {
        return (
            4 * 3_600_000,
            "the Instagram Live broadcast (private replies are accepted only while it runs)",
        );
    }
    match channel_type {
        "tiktok" => (2 * DAY_MS, "TikTok's 48-hour Comment-to-Message window"),
        "threads" => (0, "the private reply window"),
        _ => (7 * DAY_MS, "Meta's 7-day private reply window"),
    }
}

/// `normalizePostId`.
fn normalize_post_id(id: &str) -> &str {
    id.split_once('_').map_or(id, |(_, rest)| rest)
}

/// `matchPost`.
fn match_post(post: &Value, post_id: &str, is_live: bool) -> bool {
    match post["type"].as_str() {
        Some("live") => is_live,
        Some("postIds") => post["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|v| v == post_id || normalize_post_id(v) == normalize_post_id(post_id)),
        _ => !is_live,
    }
}

/// `matchKeywords`.
fn match_keywords(
    include: &Value,
    exclude: &[String],
    message: Option<&str>,
    exclude_type: &str,
) -> bool {
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

static MENTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^\w@.])@([a-z0-9_](?:[a-z0-9_.]{0,28}[a-z0-9_])?)").expect("regex")
});

/// `countMentions(resolveMentions())`: Messenger's tagged ids, else the
/// `@handles` of the text (`extractInstagramMentions`).
///
/// ponytail: no Graph fallback for a Messenger comment without `tags`.
fn mention_count(channel_type: &str, message: Option<&str>, tags: &Value) -> usize {
    let mut seen = std::collections::HashSet::new();
    if channel_type == "messenger" {
        for tag in tags.as_array().into_iter().flatten() {
            if let Some(id) = tag["id"].as_str().filter(|id| !id.is_empty()) {
                seen.insert(id.to_owned());
            }
        }
    } else {
        for capture in MENTION_RE.captures_iter(message.unwrap_or("")) {
            seen.insert(capture[1].to_lowercase());
        }
    }
    seen.len()
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

/// `willSendReply`.
fn will_send(reply: &Value) -> bool {
    match reply["type"].as_str() {
        None | Some("none") => false,
        Some("text") => !reply_texts(reply).is_empty(),
        Some(_) => reply["value"].as_str().is_some_and(|v| !v.is_empty()),
    }
}

/// `computeDelayMs`.
fn compute_delay_ms(reply_after: &Value) -> u64 {
    let value = reply_after["value"].as_f64().unwrap_or(0.0).max(0.0);
    let unit = match reply_after["type"].as_str() {
        None | Some("immediately") => return 0,
        Some("seconds") => 1000.0,
        Some("minutes") => 60_000.0,
        Some("hours") => 3_600_000.0,
        Some(random) => {
            let minutes: u64 = match random {
                "randomWithin5Minutes" => 5,
                "randomWithin10Minutes" => 10,
                "randomWithin20Minutes" => 20,
                "randomWithin30Minutes" => 30,
                "randomWithin60Minutes" => 60,
                _ => 3,
            };
            return random_below(minutes * 60_000);
        }
    };
    (value * unit) as u64
}

/// A random number in `0..n` (`Math.random()`).
fn random_below(n: u64) -> u64 {
    use std::hash::BuildHasher as _;
    let x = std::collections::hash_map::RandomState::new().hash_one(now_ms());
    if n == 0 { 0 } else { x % n }
}

/// `isWithinSchedule` (`HH:mm` in the workspace time zone).
pub(crate) fn within_schedule(start: Option<&str>, end: Option<&str>, tz: &str) -> bool {
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

/// What the comment carries (`resolveAttachmentInfo`).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Attachments {
    image: bool,
    video: bool,
    gif: bool,
}

/// `commentAutomationChannelSupportsImageFilter`.
fn supports_attachment_lookup(channel_type: &str) -> bool {
    matches!(channel_type, "messenger" | "threads")
}

/// `createAttachmentInfoResolver`: one Graph read of the comment (fixed
/// hosts; a comment id that is not a plain id is never put in a URL).
async fn attachment_info(
    upstream: &impl Upstream,
    channel_type: &str,
    auth: &Value,
    comment_id: &str,
) -> Attachments {
    let plain = !comment_id.is_empty()
        && comment_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if !plain || !supports_attachment_lookup(channel_type) {
        return Attachments::default();
    }
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let (url, headers) = if channel_type == "messenger" {
        let version = auth["metadata"]["version"]
            .as_str()
            .filter(|v| {
                v.starts_with('v') && v[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
            })
            .unwrap_or(crate::channels::whatsapp::DEFAULT_API_VERSION);
        (
            format!("https://graph.facebook.com/{version}/{comment_id}?fields=attachment"),
            vec![("Authorization".to_owned(), format!("Bearer {token}"))],
        )
    } else {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("fields", "gif_url,media_type")
            .append_pair("access_token", token)
            .finish();
        (
            format!("https://graph.threads.com/v1.0/{comment_id}?{query}"),
            Vec::new(),
        )
    };
    let reply = upstream
        .send(UpstreamRequest {
            method: "GET",
            url,
            headers,
            body: None,
        })
        .await;
    let body: Value = match reply {
        Ok((status, text)) if (200..300).contains(&status) => {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        }
        other => {
            tracing::warn!(comment_id, reply = ?other.map(|(s, _)| s), "Comment attachment lookup failed");
            return Attachments::default();
        }
    };
    if channel_type == "messenger" {
        let kind = body["attachment"]["type"].as_str().unwrap_or("");
        Attachments {
            image: kind == "photo",
            video: kind == "video_inline",
            gif: kind.starts_with("animated_image"),
        }
    } else {
        let kind = body["media_type"].as_str().unwrap_or("");
        Attachments {
            image: matches!(kind, "IMAGE" | "CAROUSEL_ALBUM"),
            video: kind == "VIDEO",
            gif: body["gif_url"].as_str().is_some_and(|g| !g.is_empty()),
        }
    }
}

static PHONE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+?[0-9][0-9\s\-().]{7,}").expect("regex"));
static EMOJI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\p{Emoji_Presentation}|\p{Extended_Pictographic}\x{FE0F}").expect("regex")
});

/// `hasHideRule`.
fn has_hide_rule(hide: &Value) -> bool {
    HIDE_FLAGS.iter().any(|flag| hide[*flag] == true)
        || hide["showCommentsAfter"].as_str().unwrap_or("none") != "none"
}

/// `matchHideRule`: `(rule, matched keyword)`.
fn match_hide_rule(
    hide: &Value,
    message: Option<&str>,
    attachments: Attachments,
) -> Option<(&'static str, Option<String>)> {
    let text = message.unwrap_or("");
    let on = |flag: &str| hide[flag] == true;
    if on("all") {
        return Some(("all", None));
    }
    if on("hasPhoneNumber") && PHONE_RE.is_match(text) {
        return Some(("phoneNumber", None));
    }
    if on("hasLink") && chatbotx_core::utils::has_link::has_link(text) {
        return Some(("link", None));
    }
    if on("hasKeywords") {
        let normalized = norm(text);
        if let Some(keyword) = hide["keywords"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|k| normalized.contains(norm(k).as_str()))
        {
            return Some(("keyword", Some(keyword.to_owned())));
        }
    }
    if on("hasImage") && attachments.image {
        return Some(("image", None));
    }
    if on("hasVideo") && attachments.video {
        return Some(("video", None));
    }
    if on("hasGif") && attachments.gif {
        return Some(("gif", None));
    }
    if on("hasEmoji") && EMOJI_RE.is_match(text) {
        return Some(("emoji", None));
    }
    None
}

/// `UNHIDE_DELAY_MS`.
fn unhide_delay_ms(after: &str) -> u64 {
    const HOUR: u64 = 3_600_000;
    match after {
        "6h" => 6 * HOUR,
        "12h" => 12 * HOUR,
        days => days
            .strip_suffix('d')
            .and_then(|d| d.parse::<u64>().ok())
            .filter(|d| (1..=10).contains(d))
            .map_or(0, |d| d * 24 * HOUR),
    }
}

/// The automations of the account (`findActiveAutomations`, scoped by the
/// inbox the comment arrived on), plus the workspace time zone.
async fn load_automations(
    deps: &Deps,
    ws: i64,
    inbox_id: i64,
    channel_type: &str,
    only: Option<i64>,
) -> Result<(String, Vec<Automation>), JobError> {
    let channel_type = channel_type.to_owned();
    deps.db
        .read(move |conn| {
            let tz: Option<String> = conn
                .query_row(
                    r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            let mut stmt = conn.prepare(
                r#"SELECT "id", "startTime", "endTime", "post", "privateReply", "publicReply",
                          "includeKeywords", "excludeKeywords", "excludeKeywordsType", "options",
                          "hideComments", "replyAfter"
                   FROM "CommentAutomation"
                   WHERE "workspaceId" = ?1 AND "isActive" = 1 AND "type" = ?2
                     AND ("inboxId" IS NULL OR "inboxId" = ?3) AND (?4 IS NULL OR "id" = ?4)
                   ORDER BY "id""#,
            )?;
            let rows = stmt
                .query_map(params![ws, channel_type, inbox_id, only], |r| {
                    let json = |i: usize| -> chatbotx_db::rusqlite::Result<Value> {
                        Ok(r.get::<_, Option<String>>(i)?
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(Value::Null))
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
                        hide_comments: json(10)?,
                        reply_after: json(11)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((tz.unwrap_or_else(|| "UTC".to_owned()), rows))
        })
        .await
        .map_err(db_fail)
}

/// `processCommentAutomation`.
async fn process_comment_automation(
    deps: &Deps,
    upstream: &impl Upstream,
    data: &Value,
) -> Result<(), JobError> {
    let channel_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
    let (channel, ctx, inbox) = resolve(deps, &channel_type, identifier).await?;
    let ws = inbox.workspace_id;
    let pair = match id_of(&data["workspaceId"]) {
        Some(id) if id == ws => crate::chat::load_pair(deps, data).await?,
        _ => None,
    };
    let Some((conversation, contact_inbox)) = pair.filter(|(conversation, contact_inbox)| {
        conversation.workspace_id == ws && contact_inbox.inbox_id == inbox.inbox_id
    }) else {
        tracing::warn!(
            workspace_id = ws,
            comment_id = str_of(data, "commentId"),
            "Comment automation skipped: contactInbox not found"
        );
        return Ok(());
    };
    let comment = Comment::of(data);
    let only = id_of(&data["onlyAutomationId"]);
    let (tz, automations) = load_automations(deps, ws, inbox.inbox_id, &channel_type, only).await?;
    if only.is_some() && automations.is_empty() {
        tracing::info!(
            automation_id = only,
            comment_id = comment.id,
            "Missed comment replay skipped: automation no longer active"
        );
        return Ok(());
    }
    let comment_message = find_by_source(
        deps,
        &comment.id,
        &conversation,
        comment.created_ms() - DAY_MS,
    )
    .await?;
    let mut private_claimed = match only {
        Some(_) => has_sent_private_reply(deps, ws, &comment.id).await?,
        None => false,
    };
    let mut run = Run {
        deps,
        upstream,
        channel,
        ctx,
        channel_type,
        ws,
        conversation,
        contact_inbox,
        comment,
        comment_message,
        attachments: None,
        data: data.clone(),
    };
    for mut automation in automations {
        automation.limit_live(&run.channel_type);
        if let Some(reason) = run
            .automation(&automation, &tz, &mut private_claimed)
            .await?
        {
            tracing::info!(
                automation_id = automation.id,
                comment_id = run.comment.id,
                reason,
                "Comment automation skipped"
            );
            run.record_miss(automation.id, reason).await?;
        }
    }
    Ok(())
}

/// `hasSentPrivateReply`.
async fn has_sent_private_reply(deps: &Deps, ws: i64, comment_id: &str) -> Result<bool, JobError> {
    let cid = comment_id.to_owned();
    deps.db
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
        .map_err(db_fail)
}

/// A reply's result: `Ok(reply text)` (recorded `sent`) or
/// `Err((detail, http code))` (recorded `failed`).
type Outcome = Result<Option<String>, (String, Option<String>)>;

/// One comment's automation run.
struct Run<'a, U> {
    deps: &'a Deps,
    upstream: &'a U,
    channel: Arc<dyn Channel>,
    ctx: ChannelCtx,
    channel_type: String,
    ws: i64,
    conversation: Conversation,
    contact_inbox: ContactInbox,
    comment: Comment,
    comment_message: Option<Message>,
    attachments: Option<Attachments>,
    /// The `processCommentAutomation` payload.
    data: Value,
}

impl<U: Upstream> Run<'_, U> {
    async fn attachments(&mut self) -> Attachments {
        if let Some(found) = self.attachments {
            return found;
        }
        let found = attachment_info(
            self.upstream,
            &self.channel_type,
            &self.ctx.auth,
            &self.comment.id,
        )
        .await;
        self.attachments = Some(found);
        found
    }

    fn is_reply(&self) -> bool {
        is_comment_reply(
            self.comment.parent_id.as_deref(),
            &self.comment.post_id,
            &self.comment.id,
        )
    }

    /// One automation; `Some(reason)` = a miss.
    async fn automation(
        &mut self,
        a: &Automation,
        tz: &str,
        private_claimed: &mut bool,
    ) -> Result<Option<&'static str>, JobError> {
        let channel_type = self.channel_type.clone();
        let ct = channel_type.as_str();
        if !within_schedule(a.start_time.as_deref(), a.end_time.as_deref(), tz) {
            return Ok(Some("outsideSchedule"));
        }
        if !match_post(&a.post, &self.comment.post_id, self.comment.is_live) {
            return Ok(Some("postNotMatched"));
        }
        // Before the reply filters: a comment the AI moderates is never
        // answered; the AI job records the miss or re-runs this automation.
        if self.moderate_with_ai(a).await {
            tracing::info!(
                automation_id = a.id,
                comment_id = self.comment.id,
                reason = "waiting for AI moderation",
                "Comment automation skipped"
            );
            return Ok(None);
        }
        if a.option("ignoreCommentReplies").unwrap_or(true) && self.is_reply() {
            return Ok(Some("commentIsReply"));
        }
        if !match_keywords(
            &a.include_keywords,
            &a.exclude_keywords,
            self.comment.message.as_deref(),
            &a.exclude_keywords_type,
        ) {
            return Ok(Some("keywordsNotMatched"));
        }
        if a.include_keywords["type"] == "mentions" {
            let count = mention_count(ct, self.comment.message.as_deref(), &self.comment.tags);
            let wanted = a.include_keywords["mentionCount"].as_u64().unwrap_or(1);
            if (count as u64) < wanted {
                return Ok(Some("mentionCountNotMatched"));
            }
        }
        if a.option("onlyCommentsWithImage") == Some(true) {
            if !supports_attachment_lookup(ct) {
                tracing::info!(automation_id = a.id, "image filter unsupported");
            } else if !self.attachments().await.image {
                return Ok(Some("noImageAttachment"));
            }
        }
        let (miss, dedup_claimed) = self.dedup_checks(a).await?;
        if let Some(reason) = miss {
            return Ok(Some(reason));
        }
        let delay = compute_delay_ms(&a.reply_after);

        if let Some(message) = self.comment_message.clone() {
            if a.option("likeUserComment") == Some(true) {
                if ct == "threads" {
                    tracing::info!(automation_id = a.id, "like comment unsupported");
                } else {
                    let data = self.state_job(&message, json!({ "liked": true }));
                    if let Err(err) = self.chat_job(data, JobOptions::default()).await {
                        tracing::error!(?err, automation_id = a.id, "Failed to like comment");
                    }
                }
            }
            self.apply_hide_rules(a, &message).await;
        } else {
            tracing::warn!(
                automation_id = a.id,
                comment_id = self.comment.id,
                "Comment automation: incoming comment message row not found, skipping like/hide and parent threading"
            );
        }

        let public = self.public_reply(a, delay).await?;
        if let Some((reply_type, outcome)) = public.clone() {
            self.record_event(a.id, "public", reply_type, outcome)
                .await?;
            if reply_type == "AIAgent" {
                self.dispatch_ai_reply(a, "public", delay).await;
            }
        }

        let mut private = None;
        let mut deferred = false;
        let private_type = a.private_reply["type"].as_str().unwrap_or("none");
        if will_send(&a.private_reply) {
            let (window, label) = private_reply_window(ct, self.comment.is_live);
            let high_intent = self
                .comment_message
                .as_ref()
                .and_then(|m| m.content_attributes.as_ref())
                .and_then(|attributes| attributes.0.get(TIKTOK_HIGH_INTENT))
                .is_some_and(|flag| !flag.is_null() && flag != &json!(false));
            if !supports_private_reply(ct) || (private_type == "flow" && ct == "tiktok") {
                tracing::info!(automation_id = a.id, "private reply unsupported");
            } else if *private_claimed {
                self.blocked_private(
                    a.id,
                    private_type,
                    "private reply already claimed for this comment",
                    "Private reply not sent: another automation already used this comment's single private reply".to_owned(),
                )
                .await?;
            } else if now_ms() + delay as i64 - self.comment.created_ms() > window {
                self.blocked_private(
                    a.id,
                    private_type,
                    &format!("comment older than {label}"),
                    format!("Private reply not sent: the comment is outside {label}"),
                )
                .await?;
            } else if ct == "tiktok" && !high_intent {
                // ponytail: `enqueueDeferredPrivateReply` (wait for the
                // high-intent webhook, then send) is not ported.
                *private_claimed = true;
                deferred = true;
                tracing::warn!(
                    automation_id = a.id,
                    "TikTok private reply waits for high intent: deferral not ported, not sent"
                );
            } else {
                let sent = self.private_reply(a, delay).await?;
                *private_claimed |= sent.1.is_ok();
                private = Some(sent);
            }
        }
        if let Some((reply_type, outcome)) = private.clone() {
            self.record_event(a.id, "private", reply_type, outcome)
                .await?;
            if reply_type == "AIAgent" {
                self.dispatch_ai_reply(a, "private", delay).await;
            }
        }

        let public_ok = public.as_ref().is_some_and(|(_, o)| o.is_ok());
        let private_ok = private.as_ref().is_some_and(|(_, o)| o.is_ok());
        let dispatched = public_ok || private_ok || deferred;
        let configured = will_send(&a.public_reply) || will_send(&a.private_reply);
        let (automation_id, contact_id, post_id, ws) = (
            a.id,
            self.contact_inbox.contact_id,
            self.comment.post_id.clone(),
            self.ws,
        );
        let replies = if supports_private_reply(ct) {
            private_ok
        } else {
            public_ok
        };
        self.deps
            .db
            .write(move |tx| {
                if (dispatched || !configured) && !dedup_claimed {
                    tx.execute(
                        r#"INSERT INTO "CommentAutomationReply" ("id", "automationId", "contactId", "postId", "workspaceId")
                           VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
                        params![new_id(), automation_id, contact_id, post_id, ws],
                    )?;
                } else if !(dispatched || !configured) && dedup_claimed {
                    tx.execute(
                        r#"DELETE FROM "CommentAutomationReply" WHERE "automationId" = ?1 AND "contactId" = ?2 AND "postId" = ?3"#,
                        params![automation_id, contact_id, post_id],
                    )?;
                }
                if replies {
                    tx.execute(
                        r#"UPDATE "CommentAutomation" SET "repliesCount" = "repliesCount" + 1 WHERE "id" = ?1"#,
                        [automation_id],
                    )?;
                }
                Ok(())
            })
            .await
            .map_err(db_fail)?;
        Ok(None)
    }

    /// `recordBlockedPrivateReply`.
    async fn blocked_private(
        &self,
        automation_id: i64,
        reply_type: &str,
        log: &str,
        detail: String,
    ) -> Result<(), JobError> {
        tracing::info!(automation_id, reason = log, "Comment automation skipped");
        self.record_event(automation_id, "private", reply_type, Err((detail, None)))
            .await
    }

    /// `replyToNewContactsOnly`, `findDedup`, `hasRepliedOnOtherPost` and
    /// `claimDedup`, in the original order: `(miss, dedup claimed)`.
    async fn dedup_checks(&self, a: &Automation) -> Result<(Option<&'static str>, bool), JobError> {
        let new_only = a.option("replyToNewContactsOnly").unwrap_or(false);
        let once = a.option("replyOncePerUserPerPost").unwrap_or(false);
        let other_posts = a
            .option("replyToUsersWhoCommentedOnOtherPosts")
            .unwrap_or(true);
        let (automation_id, contact_id, pid, ws) = (
            a.id,
            self.contact_inbox.contact_id,
            self.comment.post_id.clone(),
            self.ws,
        );
        self.deps
            .db
            .write(move |tx| {
                if new_only {
                    let count: i64 = tx.query_row(
                        r#"SELECT COUNT(*) FROM "ContactInbox" WHERE "contactId" = ?1"#,
                        [contact_id],
                        |r| r.get(0),
                    )?;
                    if count > 1 {
                        return Ok((Some("contactNotNew"), false));
                    }
                }
                let replied = |same_post: bool| {
                    tx.query_row(
                        &format!(
                            r#"SELECT 1 FROM "CommentAutomationReply"
                               WHERE "automationId" = ?1 AND "contactId" = ?2 AND "postId" {} ?3 LIMIT 1"#,
                            if same_post { "=" } else { "!=" }
                        ),
                        params![automation_id, contact_id, pid],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .map(|found| found.is_some())
                };
                if once && replied(true)? {
                    return Ok((Some("alreadyRepliedOnPost"), false));
                }
                if !other_posts && replied(false)? {
                    return Ok((Some("engagedOnOtherPost"), false));
                }
                if once {
                    let claimed = tx.execute(
                        r#"INSERT INTO "CommentAutomationReply" ("id", "automationId", "contactId", "postId", "workspaceId")
                           VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
                        params![new_id(), automation_id, contact_id, pid, ws],
                    )?;
                    if claimed == 0 {
                        return Ok((Some("alreadyRepliedOnPost"), false));
                    }
                    return Ok((None, true));
                }
                Ok((None, false))
            })
            .await
            .map_err(db_fail)
    }

    /// The `changeChannelMessageState` data.
    fn state_job(&self, message: &Message, extra: Value) -> Value {
        state_job(
            crate::chat::CHANGE_CHANNEL_MESSAGE_STATE,
            self.ws,
            self.conversation.id,
            self.contact_inbox.id,
            (message.id, message.created_at),
            extra,
        )
    }

    async fn chat_job(&self, job: Value, options: JobOptions) -> Result<(), JobError> {
        let name = str_of(&job, "type").unwrap_or("").to_owned();
        self.deps
            .jobs
            .add(QueueName::Chat, &name, job, options)
            .await
            .map(|_| ())
            .map_err(db_fail)
    }

    /// `applyHideRules` (never fails the run).
    async fn apply_hide_rules(&mut self, a: &Automation, message: &Message) {
        let hide = &a.hide_comments;
        if !has_hide_rule(hide) {
            return;
        }
        // Threads hides top-level comments only.
        if self.channel_type == "threads" && self.is_reply() {
            tracing::info!(automation_id = a.id, "hide nested reply unsupported");
            return;
        }
        let needs = ["hasImage", "hasVideo", "hasGif"]
            .iter()
            .any(|flag| hide[*flag] == true);
        let attachments = if needs {
            self.attachments().await
        } else {
            Attachments::default()
        };
        let Some((rule, keyword)) =
            match_hide_rule(hide, self.comment.message.as_deref(), attachments)
        else {
            return;
        };
        let after = hide["showCommentsAfter"]
            .as_str()
            .unwrap_or("none")
            .to_owned();
        if let Err(err) = self.moderate(a.id, message, rule, keyword, &after).await {
            tracing::error!(?err, automation_id = a.id, "Failed to apply hide comments");
        }
    }

    /// `recordAndEnqueueModeration` + `enqueueModerationAction` (hide).
    async fn moderate(
        &self,
        automation_id: i64,
        message: &Message,
        rule: &'static str,
        keyword: Option<String>,
        show_after: &str,
    ) -> Result<(), JobError> {
        let entry = comment_ai::ModerationEntry {
            source: "rule",
            rule: Some(rule),
            keyword,
            action: "hide".into(),
            ..Default::default()
        };
        comment_ai::record_and_enqueue_moderation(
            self.deps,
            &self.target(message),
            automation_id,
            entry,
            show_after,
        )
        .await
        .map(|_| ())
    }

    fn target<'s>(&'s self, message: &Message) -> comment_ai::Target<'s> {
        comment_ai::Target {
            ws: self.ws,
            conversation_id: self.conversation.id,
            contact_inbox: &self.contact_inbox,
            comment: &self.comment,
            message_id: message.id,
            message_created_at: message.created_at,
        }
    }

    /// `executePublicReply`: `(reply type, outcome)`, `None` = nothing sent.
    async fn public_reply(
        &self,
        a: &Automation,
        delay: u64,
    ) -> Result<Option<(&'static str, Outcome)>, JobError> {
        let reply = &a.public_reply;
        let attempts = SINGLE_ATTEMPT
            .contains(&self.channel_type.as_str())
            .then_some(1);
        match reply["type"].as_str() {
            Some("text") => {
                let texts = reply_texts(reply);
                if texts.is_empty() {
                    return Ok(None);
                }
                let mut sent = Vec::new();
                for (index, raw) in texts.iter().enumerate() {
                    let text =
                        render(self.deps, &self.conversation, &self.contact_inbox, raw).await;
                    let delay = delay + index as u64 * PUBLIC_REPLY_SPACING_MS;
                    if let Err(err) = self
                        .reply_to()
                        .post_public(a.id, &text, delay, attempts)
                        .await?
                    {
                        return Ok(Some(("text", Err((err, None)))));
                    }
                    sent.push(text);
                }
                Ok(Some(("text", Ok(Some(sent.join("\n"))))))
            }
            Some("flow") => match reply["value"].as_str().filter(|v| !v.is_empty()) {
                Some(flow_id) => Ok(Some((
                    "flow",
                    self.send_flow(a.id, flow_id, "public", delay, attempts)
                        .await,
                ))),
                None => Ok(None),
            },
            // `processCommentAIReply` lands the text on the event row.
            Some("AIAgent") if will_send(reply) => Ok(Some(("AIAgent", Ok(None)))),
            _ => Ok(None),
        }
    }

    /// `executePrivateReply` (window and claim already checked).
    async fn private_reply(
        &self,
        a: &Automation,
        delay: u64,
    ) -> Result<(&'static str, Outcome), JobError> {
        let reply = &a.private_reply;
        match reply["type"].as_str() {
            Some("text") => {
                let raw = reply["value"].as_str().unwrap_or("");
                let text = render(self.deps, &self.conversation, &self.contact_inbox, raw).await;
                let send = private_text(
                    &self.contact_inbox,
                    &self.conversation,
                    &self.comment.id,
                    &self.comment.post_id,
                    &text,
                );
                match self.channel.send_private_reply(&self.ctx, send).await {
                    Ok(result) => {
                        // `recordInlinePrivateReply` (never fails the run).
                        if let Err(err) = self
                            .store_reply(
                                &text,
                                a.id,
                                "private",
                                result.message_ids.first().cloned(),
                            )
                            .await
                        {
                            tracing::warn!(
                                ?err,
                                automation_id = a.id,
                                "Failed to record the private reply in the inbox"
                            );
                        }
                        Ok(("text", Ok(Some(text))))
                    }
                    Err(failure) => Ok(("text", Err(failure_detail(&failure)))),
                }
            }
            Some("flow") => {
                // ponytail: the flow runs in the comment conversation, not a
                // resolved DM conversation (`resolveDirectMessageConversationId`).
                let flow_id = reply["value"].as_str().unwrap_or("");
                Ok((
                    "flow",
                    self.send_flow(a.id, flow_id, "private", delay, None).await,
                ))
            }
            _ => Ok(("AIAgent", Ok(None))),
        }
    }

    /// The `sendFlow` integration job of a flow reply.
    async fn send_flow(
        &self,
        automation_id: i64,
        flow_id: &str,
        reply_channel: &str,
        delay: u64,
        attempts: Option<u32>,
    ) -> Outcome {
        let flow_name = describe_flow(self.deps, self.ws, flow_id).await;
        let data = json!({
            "conversationId": self.conversation.id.to_string(),
            "contactInboxId": self.contact_inbox.id.to_string(),
            "flowId": flow_id,
            "origin": "channel",
            "metadata": {
                "type": chatbotx_core::flow_config::nodes::COMMENT_AUTOMATION_PAYLOAD_TYPE,
                "commentAutomationId": automation_id.to_string(),
                "commentId": self.comment.id,
                "replyChannel": reply_channel,
            },
            "commentAnchor": {
                "commentId": self.comment.id,
                "replyChannel": reply_channel,
                "automationId": automation_id.to_string(),
            },
        });
        self.deps
            .jobs
            .add(
                QueueName::Integration,
                crate::flow::SEND_FLOW,
                json!({ "type": crate::flow::SEND_FLOW, "data": data }),
                JobOptions {
                    delay_ms: Some(delay),
                    attempts,
                    ..JobOptions::default()
                },
            )
            .await
            .map(|_| Some(flow_name))
            .map_err(|err| (err.to_string(), None))
    }

    /// The outgoing reply's message row.
    async fn store_reply(
        &self,
        text: &str,
        automation_id: i64,
        channel: &str,
        source_id: Option<String>,
    ) -> Result<Value, JobError> {
        self.reply_to()
            .store_reply(text, automation_id, channel, source_id)
            .await
    }

    fn reply_to(&self) -> comment_ai::ReplyTo<'_> {
        comment_ai::ReplyTo {
            deps: self.deps,
            ws: self.ws,
            conversation: &self.conversation,
            contact_inbox: &self.contact_inbox,
            comment_id: &self.comment.id,
            parent: self.comment_message.as_ref().map(|m| (m.id, m.created_at)),
        }
    }

    /// `recordReplyEvent` / `recordReplyFailure` / `recordBlockedPrivateReply`
    /// (one row per automation, comment and reply channel).
    async fn record_event(
        &self,
        automation_id: i64,
        channel: &'static str,
        reply_type: &str,
        outcome: Outcome,
    ) -> Result<(), JobError> {
        let (ws, contact_id, ciid) = (
            self.ws,
            self.contact_inbox.contact_id,
            self.contact_inbox.id,
        );
        let (post_id, comment_id, text, occurred, reply_type) = (
            self.comment.post_id.clone(),
            self.comment.id.clone(),
            self.comment.message.clone(),
            self.comment.created_ms(),
            reply_type.to_owned(),
        );
        let (status, reply_text, detail, http) = match outcome {
            Ok(text) => ("sent", text, None, None),
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
        self.deps
            .db
            .write(move |tx| {
                let now = now_ms();
                let inserted = tx.execute(
                    r#"INSERT INTO "CommentAutomationEvent" ("id", "workspaceId", "automationId", "contactId",
                           "contactInboxId", "postId", "commentId", "commentText", "replyChannel", "replyType",
                           "replyText", "status", "errorDetail", "httpCode", "failedAt", "occurredAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                       ON CONFLICT DO NOTHING"#,
                    params![
                        new_id(), ws, automation_id, contact_id, ciid, post_id, comment_id, text, channel,
                        reply_type, reply_text, status, detail, http, (status == "failed").then_some(now), occurred
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

    /// `recordMisses` (one row).
    async fn record_miss(&self, automation_id: i64, reason: &'static str) -> Result<(), JobError> {
        comment_ai::record_miss(
            self.deps,
            self.ws,
            automation_id,
            &self.contact_inbox,
            &self.comment,
            reason,
        )
        .await
    }
}

/// A comment-anchored private text reply.
fn private_text(
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    comment_id: &str,
    post_id: &str,
    text: &str,
) -> SendMessage {
    SendMessage {
        contact: OutgoingContact::new(contact_inbox, conversation),
        message: json!({
            "type": "comment",
            "text": text,
            "contentAttributes": {
                "replyToCommentId": comment_id,
                "postId": post_id,
                "isPrivateReply": true,
            },
        }),
        quick_replies: None,
        metadata: None,
        send_from: None,
    }
}

/// The `changeChannelMessageState` / `deleteChannelMessage` job.
fn state_job(
    kind: &str,
    ws: i64,
    conversation_id: i64,
    contact_inbox_id: i64,
    (message_id, created_at): (i64, i64),
    extra: Value,
) -> Value {
    let mut data = json!({
        "conversation": { "id": conversation_id.to_string(), "workspaceId": ws.to_string() },
        "contactInbox": { "id": contact_inbox_id.to_string() },
        "message": { "id": message_id.to_string(), "createdAt": created_at },
    });
    if let (Some(data), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
        data.extend(extra.clone());
    }
    json!({ "type": kind, "data": data })
}

/// The failure detail and HTTP code of a channel send.
fn failure_detail(failure: &SendFailure) -> (String, Option<String>) {
    match failure {
        SendFailure::Channel(error) => (
            error.message.clone(),
            Some(error.http_status_code.to_string()),
        ),
        SendFailure::Other(message) => (message.clone(), None),
    }
}

/// `describeFlowReply`.
async fn describe_flow(deps: &Deps, ws: i64, flow_id: &str) -> String {
    let id = flow_id.parse::<i64>().ok();
    let name = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "name" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .ok()
        .flatten();
    format!("Flow: {}", name.as_deref().unwrap_or(flow_id))
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

#[path = "comment_ai.rs"]
mod comment_ai;
pub(crate) use comment_ai::story_reply_text;
pub use comment_ai::{COMMENT_AI_REPLY, COMMENT_MODERATION, comment_ai_reply, comment_moderation};

#[cfg(test)]
#[path = "low_tests.rs"]
mod tests;
