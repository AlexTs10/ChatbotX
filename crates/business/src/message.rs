//! `message/list-for-conversation.ts` (`listForConversation`,
//! `findForContact`) and a reduced `message/create-outgoing.ts`
//! (`createOutgoing`: text, flow, products, device files, media library
//! files and image limits; no private replies or agent-reply metric).
//!
//! The service does not enqueue or publish: [`Outgoing`] carries the jobs
//! (`sendChannelMessage` on `chat`, `sendFlow` on `integration`) and the
//! realtime `messageCreated` payload for the caller (`ApiState`).
//! [`message_json`] leaves the attachment `url` / `fallbackUrl` `null`; the
//! read paths resolve them with `crate::media_url` (`withResolvedAttachmentUrls`)
//! and the message a send creates uses `getPublicFileUrl(originPath,
//! storageUrl)` ([`message_json_with_urls`]).

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::inbox_image_support::{
    InboxImageCheckResult, check_inbox_images, is_image_mime_type, resolve_inbox_reply_kind,
};
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::Db;
use chatbotx_db::repo::media_library_file;
use chatbotx_db::repo::message::{
    self as repo_message, CreateAttachmentInput, CreateMessageInput, FindTriggerMessageOptions,
    ListMessagesQuery, MessageWithAttachments, Pagination, PaginationCursor, SinceTimeInput,
    get_safe_since_time,
};
use chatbotx_db::repo::models::MediaLibraryFile;
use chatbotx_db::repo::models::{Attachment, ContactInbox, Conversation, Message};
use chatbotx_db::repo::{Model, flow, iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_files::Uploader;
use chatbotx_files::mime::guess_file_type_from_mime_type;
use chatbotx_jobs::QueueName;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

const HOUR_MS: i64 = 60 * 60 * 1000;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn sid(value: i64) -> Value {
    Value::String(value.to_string())
}

fn opt_sid(value: Option<i64>) -> Value {
    value.map_or(Value::Null, sid)
}

fn opt_iso(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |ms| Value::String(iso(ms)))
}

/// `conversationService.findBy({ id, workspaceId })`.
fn conversation_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Conversation>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            Conversation::select_sql()
        ),
        params![id, workspace_id],
        Conversation::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// The conversation fields the job payloads carry (reduced; the worker
/// port reads the rest by id).
pub fn conversation_json(conversation: &Conversation) -> Value {
    json!({
        "id": sid(conversation.id),
        "workspaceId": sid(conversation.workspace_id),
        "contactId": sid(conversation.contact_id),
        "sourceId": conversation.source_id,
        "assignedUserId": opt_sid(conversation.assigned_user_id),
        "assignedInboxTeamId": opt_sid(conversation.assigned_inbox_team_id),
        "botEnabled": conversation.bot_enabled,
        "createdAt": iso(conversation.created_at),
        "updatedAt": iso(conversation.updated_at),
    })
}

/// The contact inbox fields the job payloads carry (reduced).
pub fn contact_inbox_json(contact_inbox: &ContactInbox) -> Value {
    json!({
        "id": sid(contact_inbox.id),
        "contactId": sid(contact_inbox.contact_id),
        "inboxId": sid(contact_inbox.inbox_id),
        "channel": contact_inbox.channel,
        "source": contact_inbox.source,
        "sourceId": contact_inbox.source_id,
        "sourceUserId": contact_inbox.source_user_id,
        "personaId": contact_inbox.persona_id,
        "lastMessageAt": opt_iso(contact_inbox.last_message_at),
    })
}

fn attachment_json(attachment: &Attachment) -> Value {
    json!({
        "id": sid(attachment.id),
        "createdAt": iso(attachment.created_at),
        "updatedAt": iso(attachment.updated_at),
        "workspaceId": sid(attachment.workspace_id),
        "conversationId": sid(attachment.conversation_id),
        "messageId": sid(attachment.message_id),
        "messageCreatedAt": iso(attachment.message_created_at),
        "fileType": attachment.file_type,
        "sourceId": attachment.source_id,
        "mimeType": attachment.mime_type,
        "width": attachment.width,
        "height": attachment.height,
        "size": attachment.size,
        "thumbnailPath": attachment.thumbnail_path,
        "originPath": attachment.origin_path,
        "name": attachment.name,
        "keepOriginal": attachment.keep_original,
        // Resolved by `crate::media_url` on the read paths.
        "url": Value::Null,
        "fallbackUrl": Value::Null,
    })
}

fn message_row_json(message: &Message) -> serde_json::Map<String, Value> {
    let out = json!({
        "id": sid(message.id),
        "createdAt": iso(message.created_at),
        "updatedAt": iso(message.updated_at),
        "conversationId": sid(message.conversation_id),
        "contactInboxId": sid(message.contact_inbox_id),
        "workspaceId": sid(message.workspace_id),
        "text": message.text,
        "contentAttributes": message.content_attributes.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "messageType": message.message_type,
        "contentType": message.content_type,
        "senderType": message.sender_type,
        "senderId": opt_sid(message.sender_id),
        "sourceId": message.source_id,
        "deletedAt": opt_iso(message.deleted_at),
        "type": message.r#type,
        "parentId": message.parent_id,
        "attributes": message.attributes.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "sendError": message.send_error,
    });
    match out {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

/// `ListForConversationInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListForConversation {
    pub workspace_id: i64,
    pub conversation_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
    pub cursor: Option<PaginationCursor>,
    pub limit: i64,
}

/// `ListForConversationResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagePage {
    pub data: Vec<MessageWithAttachments>,
    pub next_cursor: Option<PaginationCursor>,
}

/// `listForConversation`: newest first; without a cursor the page starts
/// at the end of the hour of the contact inbox's `lastMessageAt` (or now).
pub async fn list_for_conversation(db: &Db, input: ListForConversation) -> Result<MessagePage> {
    tx::read(db, move |conn| {
        let conversation = match input.conversation_id {
            Some(id) => conversation_in(conn, input.workspace_id, id)?,
            None => None,
        };
        let mut last_message_at = None;
        if let Some(conversation) = &conversation {
            let contact_inbox = match input.contact_inbox_id {
                Some(id) => conn
                    .query_row(
                        r#"SELECT "lastMessageAt" FROM "ContactInbox" WHERE "contactId" = ?1 AND "id" = ?2"#,
                        params![conversation.contact_id, id],
                        |r| r.get::<_, Option<i64>>(0),
                    )
                    .optional()
                    .map_err(repo_err)?
                    .flatten(),
                None => crate::conversation::find_recent_in(
                    conn,
                    input.workspace_id,
                    conversation.contact_id,
                )?
                .and_then(|ci| ci.last_message_at),
            };
            last_message_at = contact_inbox;
        }
        // `endOfHour(lastMessageAt ?? now)`.
        let anchor = last_message_at.unwrap_or_else(now_ms);
        let end_of_hour = anchor.div_euclid(HOUR_MS) * HOUR_MS + HOUR_MS - 1;
        let result = repo_message::list_by_conversation(
            conn,
            &ListMessagesQuery {
                contact_inbox_id: input.contact_inbox_id,
                conversation_id: input.conversation_id,
                pagination: Pagination {
                    cursor: Some(input.cursor.unwrap_or(PaginationCursor {
                        created_at: end_of_hour,
                        id: None,
                        shard_id: None,
                    })),
                    limit: input.limit,
                },
                since_time: get_safe_since_time(
                    conversation.as_ref().map(|c| SinceTimeInput::Date(c.created_at)),
                    None,
                ),
                workspace_id: input.workspace_id,
            },
        )?;
        Ok(MessagePage {
            data: result.data,
            next_cursor: result.next_cursor,
        })
    })
    .await
}

/// `findForContact`: one message of the conversation (404 "Message not
/// found").
pub async fn find_for_contact(
    db: &Db,
    workspace_id: i64,
    message_id: i64,
    conversation_id: i64,
) -> Result<MessageWithAttachments> {
    tx::read(db, move |conn| {
        let missing = || BusinessError::not_found("Message not found");
        let conversation =
            conversation_in(conn, workspace_id, conversation_id)?.ok_or_else(missing)?;
        let since_time =
            get_safe_since_time(Some(SinceTimeInput::Date(conversation.created_at)), None)
                .unwrap_or(conversation.created_at);
        repo_message::find_trigger_message(
            conn,
            &FindTriggerMessageOptions {
                conversation_id,
                id: message_id,
                require_complete_results: true,
                since_time,
                workspace_id,
            },
        )?
        .filter(|m| m.message.conversation_id == conversation_id)
        .ok_or_else(missing)
    })
    .await
}

/// `CreateOutgoingInput` (the ported part).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutgoingInput {
    /// The flow branch (`{ flowId, nodeId }`).
    pub flow_id: Option<i64>,
    pub node_id: Option<String>,
    /// The products branch (`SendProductsContent`).
    pub products: Option<Value>,
    pub text: Option<String>,
    pub inbox_id: Option<i64>,
    pub client_id: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub reply_to_message_created_at: Option<i64>,
    pub is_private_reply: bool,
    pub content_attributes: Option<Value>,
    pub quick_replies: Option<Value>,
    /// `mediaFile.path` (the legacy path-based media library pick).
    pub media_file_path: Option<String>,
    /// `mediaFileId` / `mediaFileIds` (in send order).
    pub media_file_ids: Vec<i64>,
    /// `files`: device uploads (win over media library picks).
    pub files: Vec<chatbotx_files::InputFile>,
}

/// Where media library files are copied and how their URLs are built.
#[derive(Clone, Copy, Default)]
pub struct MediaStore<'a> {
    pub uploader: Option<&'a Uploader>,
    /// The tenant `storageUrl`.
    pub storage_url: &'a str,
}

/// One job `createOutgoing` adds (`queue.add(name, data, opts)`).
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingJob {
    pub queue: QueueName,
    pub name: &'static str,
    pub data: Value,
    /// `{ attempts: 1 }` (manual Threads comment replies).
    pub attempts: Option<u32>,
}

/// What `createOutgoing` did: the stored message (`None` for a flow), the
/// realtime `messageCreated` payload and the jobs to add, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub message: Option<MessageWithAttachments>,
    pub realtime: Option<Value>,
    pub jobs: Vec<OutgoingJob>,
    /// The reply to record once the jobs are added ([`record_agent_reply`]).
    pub reply: Option<AgentReply>,
}

/// `recordAgentReply` input: a sent reply of `admin_id` (an inbox member;
/// `None` = an API / system sender, which closes the window without a metric).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentReply {
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub contact_id: i64,
    pub channel: String,
    pub inbox_id: i64,
    pub at: i64,
    pub admin_id: Option<i64>,
    pub event_id: String,
}

impl AgentReply {
    fn of(
        conversation: &Conversation,
        contact_inbox: &ContactInbox,
        at: i64,
        admin_id: Option<i64>,
        event_id: String,
    ) -> AgentReply {
        AgentReply {
            workspace_id: conversation.workspace_id,
            conversation_id: conversation.id,
            contact_id: contact_inbox.contact_id,
            channel: contact_inbox.channel.clone(),
            inbox_id: contact_inbox.inbox_id,
            at,
            admin_id,
            event_id,
        }
    }
}

/// `recordAgentReply`: closes the conversation's human-reply window
/// (`markAgentReplied`) and, for a member, emits `agent:replied` with the
/// measured seconds. Never fails: the message is already queued.
pub async fn record_agent_reply(db: &Db, reply: AgentReply) {
    let (workspace_id, conversation_id, event_id) = (
        reply.workspace_id,
        reply.conversation_id,
        reply.event_id.clone(),
    );
    let payload = tx::write(db, move |conn| {
        let Some(claim) = chatbotx_db::repo::conversation::close_human_reply_window(
            conn,
            reply.workspace_id,
            reply.conversation_id,
            reply.at,
        )?
        else {
            return Ok(None);
        };
        let (Some(admin_id), Some(since)) = (reply.admin_id, claim.closed_awaiting_since) else {
            return Ok(None);
        };
        // `resolveMeasuredFrom`: an assignment to the replier after the customer
        // message (and not after the reply) starts the window.
        let measured_from = match claim.assigned_user_at {
            Some(at)
                if claim.assigned_user_id == Some(admin_id) && at <= reply.at && at > since =>
            {
                at
            }
            _ => since,
        };
        let response_seconds = ((reply.at - measured_from) / 1000).max(0);
        let schedule = crate::response_sla::load_schedule(
            conn,
            reply.workspace_id,
            Some(admin_id),
            measured_from,
            reply.at,
        )?;
        let working = schedule.as_ref().map(|s| {
            crate::response_sla::working_seconds_between(
                measured_from,
                reply.at,
                &s.intervals,
                &s.zone,
                &s.overrides,
            )
        });
        Ok(Some(json!({
            "eventType": "agent:replied",
            "eventId": reply.event_id,
            "workspaceId": reply.workspace_id.to_string(),
            "conversationId": reply.conversation_id.to_string(),
            "contactId": reply.contact_id.to_string(),
            "adminId": admin_id.to_string(),
            "occurredAt": iso(reply.at),
            "awaitingSince": iso(since),
            "measuredFrom": iso(measured_from),
            "responseSeconds": response_seconds,
            "workingResponseSeconds": working,
            "workingHoursId": schedule.map(|s| s.working_hours_id.to_string()),
            "slaTargetSeconds": claim.closed_response_sla_target_seconds,
            "slaMet": claim.closed_response_due_at.map(|due| reply.at <= due),
            "isFirstResponse": claim.previous_admin_replied_at.is_none(),
            "channel": reply.channel,
            "inboxId": reply.inbox_id.to_string(),
        })))
    })
    .await;
    match payload {
        Ok(Some(payload)) => crate::events::global::dashboard(payload).await,
        Ok(None) => {}
        Err(error) => tracing::error!(%error, workspace_id, conversation_id, event_id,
            "Failed to record agent reply after enqueue"),
    }
}

/// `createOutgoing`. `user_id` = the inbox user (`senderType` `user`); none
/// = the token API (`api`). No file storage: a media library pick fails.
pub async fn create_outgoing(
    db: &Db,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    input: OutgoingInput,
    user_id: Option<i64>,
) -> Result<Outgoing> {
    create_outgoing_with_media(
        db,
        MediaStore::default(),
        conversation,
        contact_inbox,
        input,
        user_id,
    )
    .await
}

fn media_file_not_found() -> BusinessError {
    BusinessError::exception("Media library file not found", "systemError", 400)
}

/// `resolveMediaLibraryFiles`: the workspace's rows, in the caller's order.
async fn resolve_media_library_files(
    db: &Db,
    workspace_id: i64,
    input: &OutgoingInput,
) -> Result<Vec<MediaLibraryFile>> {
    if let Some(path) = input.media_file_path.clone() {
        let file = tx::read(db, move |conn| {
            Ok(media_library_file::find_by_path(conn, workspace_id, &path)?)
        })
        .await?;
        return file.map(|f| vec![f]).ok_or_else(media_file_not_found);
    }
    if input.media_file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = input.media_file_ids.clone();
    let files = tx::read(db, move |conn| {
        Ok(media_library_file::find_by_ids(conn, workspace_id, &ids)?)
    })
    .await?;
    input
        .media_file_ids
        .iter()
        .map(|id| {
            files
                .iter()
                .find(|f| f.id == *id)
                .cloned()
                .ok_or_else(media_file_not_found)
        })
        .collect()
}

/// `assertChannelAcceptsImages` (`ImagesNotSupportedOnChannelError`); a
/// channel outside the enum has no known limits.
fn assert_channel_accepts_images(
    channel: &str,
    input: &OutgoingInput,
    files: &[MediaLibraryFile],
) -> Result<()> {
    let Some(channel_type) = ChannelType::parse(channel) else {
        return Ok(());
    };
    let kind =
        resolve_inbox_reply_kind(input.reply_to_message_id.is_some(), input.is_private_reply);
    // `countImages`: device files when there are any, else the library picks.
    let images = if input.files.is_empty() {
        files
            .iter()
            .filter(|f| is_image_mime_type(&f.mime_type))
            .count()
    } else {
        input
            .files
            .iter()
            .filter(|f| is_image_mime_type(&f.mime_type))
            .count()
    };
    let (reason, max, message) = match check_inbox_images(channel_type, kind, images) {
        InboxImageCheckResult::Ok => return Ok(()),
        InboxImageCheckResult::ImagesNotSupported { max } => (
            "imagesNotSupported",
            max,
            format!("The {channel} channel can't send images here"),
        ),
        InboxImageCheckResult::TooManyImages { max } => (
            "tooManyImages",
            max,
            format!("The {channel} channel can send at most {max} image(s) here"),
        ),
    };
    Err(BusinessError::Exception {
        message,
        code: "imagesNotSupportedOnChannel".into(),
        status: 422,
        field: None,
        data: Some(json!({ "reason": reason, "channel": channel, "max": max })),
    })
}

/// [`message_json`] with each attachment `url` set to its public storage URL
/// (`getPublicFileUrl(originPath, storageUrl)`).
pub fn message_json_with_urls(message: &MessageWithAttachments, storage_url: &str) -> Value {
    let mut out = message_json(message);
    if let Some(Value::Array(attachments)) = out.get_mut("attachments") {
        for attachment in attachments {
            let path = attachment["originPath"].as_str().unwrap_or_default();
            let url = get_public_file_url(path, storage_url)
                .unwrap_or_else(|_| format!("{storage_url}{path}"));
            attachment["url"] = Value::String(url);
        }
    }
    out
}

/// [`create_outgoing`] with file storage: media library picks are copied to
/// `public/space/<ws>/conversations/<conversation>/<id>` (the sent copy
/// outlives the library file) and sent as attachments.
pub async fn create_outgoing_with_media(
    db: &Db,
    media: MediaStore<'_>,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    input: OutgoingInput,
    user_id: Option<i64>,
) -> Result<Outgoing> {
    if let Some(flow_id) = input.flow_id {
        let workspace_id = conversation.workspace_id;
        let exists = tx::read(db, move |conn| {
            Ok(flow::exists_in_workspace(conn, workspace_id, flow_id)?)
        })
        .await?;
        if !exists {
            return Err(BusinessError::not_found("Flow not found"));
        }
        let mut data = json!({
            "conversationId": conversation_json(conversation),
            "contactInboxId": contact_inbox_json(contact_inbox),
            "flowId": sid(flow_id),
            "sendFrom": "inbox",
        });
        if let Some(node) = &input.node_id {
            data["nodeId"] = json!(node);
        }
        if let Some(user) = user_id {
            data["sentByUserId"] = sid(user);
        }
        return Ok(Outgoing {
            message: None,
            realtime: None,
            jobs: vec![OutgoingJob {
                queue: QueueName::Integration,
                name: "sendFlow",
                data: json!({ "type": "sendFlow", "data": data }),
                attempts: None,
            }],
            // An inbox "send flow" counts as a human reply.
            reply: Some(AgentReply::of(
                conversation,
                contact_inbox,
                chatbotx_db::repo::now_ms(),
                user_id,
                format!("reply:flow:{}", chatbotx_db::repo::new_id()),
            )),
        });
    }

    // A product message is a standalone DM; resolved before any row exists,
    // and a message missing some of the asked products is refused.
    let product_message = match &input.products {
        Some(products) => {
            if input.reply_to_message_id.is_some() || input.is_private_reply {
                return Err(BusinessError::validation(
                    "products",
                    "Products can't be sent as a comment reply",
                ));
            }
            Some(
                crate::product_message::service::prepare_outgoing(
                    db,
                    conversation.workspace_id,
                    &contact_inbox.channel,
                    products,
                    true,
                )
                .await?,
            )
        }
        None => None,
    };

    // Resolved and checked before any row or object exists. Device files win
    // over media library ids, which are then never looked up.
    let media_files = if input.files.is_empty() {
        resolve_media_library_files(db, conversation.workspace_id, &input).await?
    } else {
        Vec::new()
    };
    assert_channel_accepts_images(&contact_inbox.channel, &input, &media_files)?;
    let mut attachments = Vec::with_capacity(media_files.len().max(input.files.len()));
    let storage = || {
        media.uploader.ok_or_else(|| {
            BusinessError::Repo(chatbotx_db::repo::Error::Message(
                "File storage is not configured".into(),
            ))
        })
    };
    if !input.files.is_empty() {
        let prefix = format!(
            "public/space/{}/conversations/{}",
            conversation.workspace_id, conversation.id
        );
        let uploaded = chatbotx_files::upload_multiple_files(
            storage()?,
            &input.files,
            &prefix,
            chatbotx_files::DEFAULT_ACL,
        )
        .await
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::Message(e.to_string())))?;
        for file in uploaded {
            attachments.push(CreateAttachmentInput {
                workspace_id: conversation.workspace_id,
                conversation_id: conversation.id,
                file_type: file.file_type.as_str().into(),
                mime_type: file.mime_type,
                origin_path: file.origin_path,
                name: Some(file.name),
                size: Some(file.size as i64),
                width: file.width.map(i64::from),
                height: file.height.map(i64::from),
                ..CreateAttachmentInput::default()
            });
        }
    } else if !media_files.is_empty() {
        let uploader = storage()?;
        for file in &media_files {
            let origin_path = format!(
                "public/space/{}/conversations/{}/{}",
                conversation.workspace_id,
                conversation.id,
                new_id()
            );
            uploader
                .copy_object(&file.path, &origin_path)
                .await
                .map_err(|e| {
                    BusinessError::Repo(chatbotx_db::repo::Error::Message(e.to_string()))
                })?;
            attachments.push(CreateAttachmentInput {
                workspace_id: conversation.workspace_id,
                conversation_id: conversation.id,
                file_type: guess_file_type_from_mime_type(&file.mime_type)
                    .as_str()
                    .into(),
                mime_type: file.mime_type.clone(),
                origin_path,
                name: Some(file.name.clone()),
                size: Some(file.size),
                ..CreateAttachmentInput::default()
            });
        }
    }

    let is_comment = input.reply_to_message_id.is_some();
    let content_attributes = product_message
        .as_ref()
        .map(|p| p.content_attributes.clone())
        .or_else(|| input.content_attributes.clone())
        .or_else(|| {
            input
                .is_private_reply
                .then(|| json!({ "isPrivateReply": true }))
        });
    let message_input = CreateMessageInput {
        contact_inbox_id: contact_inbox.id,
        conversation_id: conversation.id,
        workspace_id: conversation.workspace_id,
        content_attributes,
        content_type: "text".into(),
        message_type: "outgoing".into(),
        sender_type: if user_id.is_some() { "user" } else { "api" }.into(),
        sender_id: user_id,
        parent_id: input.reply_to_message_id.clone(),
        text: match &product_message {
            Some(p) => p.text.clone(),
            None => input.text.clone(),
        },
        r#type: Some(if is_comment { "comment" } else { "message" }.into()),
        created_at: Some(now_ms()),
        ..CreateMessageInput::default()
    };
    let contact_inbox_id = contact_inbox.id;
    let contact_id = contact_inbox.contact_id;
    let workspace_id = conversation.workspace_id;
    let stored = tx::write(db, move |conn| {
        let stored = if attachments.is_empty() {
            MessageWithAttachments {
                message: repo_message::create(conn, &message_input)?,
                attachment_count: None,
                attachments: Vec::new(),
            }
        } else {
            repo_message::create_with_attachments(conn, &message_input, &attachments)?
        };
        let message = &stored.message;
        // `contactInboxService.updateTracking` (`firstInteractionAt` keeps
        // the earliest).
        conn.execute(
            r#"UPDATE "ContactInbox"
               SET "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1
                     THEN ?1 ELSE "firstInteractionAt" END,
                   "lastMessageAt" = ?1, "updatedAt" = ?2
               WHERE "id" = ?3 AND "contactId" = ?4
                 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?5)"#,
            params![message.created_at, now_ms(), contact_inbox_id, contact_id, workspace_id],
        )
        .map_err(repo_err)?;
        Ok(stored)
    })
    .await?;

    let mut realtime = message_json_with_urls(&stored, media.storage_url);
    if let Some(client_id) = &input.client_id {
        realtime["clientId"] = json!(client_id);
    }
    let mut job_message = realtime.clone();
    job_message["parentCreatedAt"] = opt_iso(input.reply_to_message_created_at);
    let mut send = json!({
        "conversation": conversation_json(conversation),
        "contactInbox": contact_inbox_json(contact_inbox),
        "message": job_message,
        "sendFrom": "inbox",
    });
    if let Some(quick_replies) = &input.quick_replies {
        send["quickReplies"] = quick_replies.clone();
    }
    let mut jobs = vec![OutgoingJob {
        queue: QueueName::Chat,
        name: "sendChannelMessage",
        data: json!({ "type": "sendChannelMessage", "data": send }),
        attempts: (contact_inbox.channel == "threads" && is_comment).then_some(1),
    }];
    if user_id.is_some()
        && product_message.is_none()
        && let Some(text) = stored.message.text.as_deref().filter(|t| !t.is_empty())
    {
        jobs.push(OutgoingJob {
            queue: QueueName::Chat,
            name: "checkOutboundAutomatedResponse",
            data: json!({
                "type": "checkOutboundAutomatedResponse",
                "data": {
                    "conversation": conversation_json(conversation),
                    "contactInbox": contact_inbox_json(contact_inbox),
                    "message": { "id": sid(stored.message.id), "text": text },
                },
            }),
            attempts: None,
        });
    }
    let reply = AgentReply::of(
        conversation,
        contact_inbox,
        stored.message.created_at,
        user_id,
        format!("reply:{}", stored.message.id),
    );
    Ok(Outgoing {
        message: Some(stored),
        realtime: Some(realtime),
        jobs,
        reply: Some(reply),
    })
}

/// `messageResourceWithRelations` / the `MessageWithPresignedAttachments`
/// JSON: every `Message` column, `attachments` (`url` / `fallbackUrl`
/// `null`) and, when set, `attachmentCount`.
pub fn message_json(message: &MessageWithAttachments) -> Value {
    let mut out = message_row_json(&message.message);
    out.insert(
        "attachments".into(),
        Value::Array(message.attachments.iter().map(attachment_json).collect()),
    );
    if let Some(count) = message.attachment_count {
        out.insert("attachmentCount".into(), json!(count));
    }
    Value::Object(out)
}
