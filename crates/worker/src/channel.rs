//! The channel send interface the integration units implement (the `message`,
//! `comment` and `conversation` handler groups of `packages/sdk/src/lib/
//! integration.ts`, reached through `integration.runChannelHandler`), the
//! registry of channels, the integration lookup of
//! `packages/channel-registry/src/registry.ts`
//! (`resolveIntegrationContextFromContactInbox`) and `chat/utils/retry.ts`.
//!
//! Payloads are the JSON shapes of the original (`message` = the stored
//! message row with `attachments`, `step` = the resolved flow step), so a
//! channel port reads the same fields as its TypeScript handler.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use chatbotx_db::Db;
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::BoxFuture;
use chatbotx_rt::Realtime;
use serde_json::{Value, json};

/// `ChannelErrorCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    RateLimited,
    AuthFailed,
    UserBlocked,
    InvalidRecipient,
    PermissionDenied,
    PayloadInvalid,
    QuotaExceeded,
    NetworkError,
    Unknown,
}

/// `ChannelError`: a provider refusal or failure with its category.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelError {
    pub message: String,
    pub category: Category,
    /// `code` (`UNKNOWN_ERROR.code` when not given).
    pub code: Option<String>,
    pub http_status_code: u16,
}

impl ChannelError {
    pub fn new(message: impl Into<String>, category: Category) -> ChannelError {
        ChannelError {
            message: message.into(),
            category,
            code: None,
            http_status_code: 400,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> ChannelError {
        self.code = Some(code.into());
        self
    }

    /// `RETRYABLE_CATEGORIES`.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.category,
            Category::RateLimited | Category::NetworkError
        )
    }

    /// `PERMANENT_CATEGORIES`.
    pub fn is_permanent(&self) -> bool {
        !matches!(
            self.category,
            Category::RateLimited | Category::NetworkError | Category::Unknown
        )
    }
}

/// Why a channel handler failed: a `ChannelError`, or any other error
/// (`SdkException`, a network library error, a missing handler).
#[derive(Debug, Clone, PartialEq)]
pub enum SendFailure {
    Channel(ChannelError),
    Other(String),
}

impl SendFailure {
    pub fn other(message: impl Into<String>) -> SendFailure {
        SendFailure::Other(message.into())
    }

    pub fn message(&self) -> &str {
        match self {
            SendFailure::Channel(e) => &e.message,
            SendFailure::Other(m) => m,
        }
    }
}

impl fmt::Display for SendFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for SendFailure {}

impl From<ChannelError> for SendFailure {
    fn from(error: ChannelError) -> SendFailure {
        SendFailure::Channel(error)
    }
}

/// `OutgoingSendResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendResult {
    pub message_ids: Vec<String>,
    /// Discrete channel messages the provider accepted.
    pub sent_count: u32,
}

pub type ChannelResult<T> = Result<T, SendFailure>;

/// `IntegrationContext`: the integration row of the contact's inbox and the
/// platform services a handler may use.
#[derive(Clone)]
pub struct ChannelCtx {
    pub workspace_id: i64,
    pub channel: String,
    pub inbox_id: i64,
    /// The `Integration<Channel>` row id.
    pub integration_id: i64,
    /// The row's `auth` JSON as stored (a channel decrypts what it encrypted).
    pub auth: Value,
    pub db: Arc<Db>,
    /// `ctx.platform.publishGuestRealtimeEvent` (none when realtime is off).
    pub realtime: Option<Realtime>,
}

/// `OutgoingContact`: the contact inbox plus `sourceConversationId`.
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingContact {
    pub contact_inbox: ContactInbox,
    pub source_conversation_id: Option<String>,
}

/// `sendMessage` / `sendComment` / `sendPrivateReply` data.
#[derive(Debug, Clone, PartialEq)]
pub struct SendMessage {
    pub contact: OutgoingContact,
    /// The message row JSON (`messageJson` + `attachments`; a comment reply
    /// carries `contentAttributes.replyToCommentId` / `postId`).
    pub message: Value,
    pub quick_replies: Option<Value>,
    pub metadata: Option<Value>,
    pub send_from: Option<String>,
}

/// `sendFlowStep` data.
#[derive(Debug, Clone, PartialEq)]
pub struct SendFlowStep {
    pub contact: OutgoingContact,
    pub flow_id: String,
    pub flow_version_id: Option<String>,
    /// The resolved step (variables, links and buttons applied).
    pub step: Value,
    pub product_message: Option<Value>,
    pub quick_replies: Option<Value>,
    pub metadata: Option<Value>,
    pub rich_response: Option<Value>,
    pub send_from: Option<String>,
    pub comment_anchor: Option<Value>,
    /// The stored message row JSON (webchat pushes it to the guest).
    pub message: Value,
}

/// `sendTyping` data.
#[derive(Debug, Clone, PartialEq)]
pub struct SendTyping {
    pub contact: OutgoingContact,
    pub typing: bool,
    pub seconds: Option<f64>,
    /// WhatsApp wamid of the newest incoming message.
    pub message_source_id: Option<String>,
}

/// `IncomingAttachment`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncomingAttachment {
    pub source_id: Option<String>,
    /// `image` | `video` | `audio` | `gif` | `file`.
    pub file_type: String,
    pub mime_type: String,
    /// The stored object key, or the provider URL of a lazy attachment.
    pub origin_path: String,
    pub size: Option<i64>,
    pub name: Option<String>,
    /// A photo message (re-encoded in the background; files keep their bytes).
    pub is_photo_message: bool,
}

/// `IncomingMessage`.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingMessage {
    pub source_id: Option<String>,
    /// `incoming` | `outgoing`.
    pub message_type: String,
    /// `text` | `location`.
    pub content_type: String,
    pub text: Option<String>,
    pub content_attributes: Option<Value>,
    pub attachments: Vec<IncomingAttachment>,
    /// `message` (default) | `comment`.
    pub kind: Option<String>,
    pub parent_id: Option<String>,
}

impl IncomingMessage {
    pub fn incoming(source_id: Option<String>) -> IncomingMessage {
        IncomingMessage {
            source_id,
            message_type: "incoming".into(),
            content_type: "text".into(),
            text: None,
            content_attributes: None,
            attachments: Vec::new(),
            kind: None,
            parent_id: None,
        }
    }
}

/// `IncomingContact`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncomingContact {
    pub source_id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub avatar: Option<String>,
    pub locale: Option<String>,
    pub language: Option<String>,
    /// WhatsApp BSUID.
    pub source_user_id: Option<String>,
    pub source_parent_user_id: Option<String>,
    pub source_username: Option<String>,
    /// Keys the conversation (a post id for comments); `None` for DMs.
    pub source_conversation_id: Option<String>,
}

/// `ReceivedMessageResult`: what a channel parser read from one webhook
/// event. JSON fields keep the SDK shapes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Received {
    pub message: Option<IncomingMessage>,
    pub contact: IncomingContact,
    pub postback_action: Option<String>,
    pub quick_reply_action: Option<String>,
    pub r#ref: Option<String>,
    pub template_flow_token: Option<String>,
    pub referral: Option<Value>,
    pub referral_source: Option<String>,
    pub button_title: Option<String>,
    /// `echoOrigin` (`thirdParty` | `firstParty`).
    pub echo_origin: Option<String>,
    /// `ThreadControlReceiveInfo` (`{ delivery, ownerRole, ... }`).
    pub thread_control: Option<Value>,
    /// `IncomingCommerceEvent`.
    pub commerce: Option<Value>,
}

/// `messageStatus` data of a channel (`handleMessageStatus` result).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageStatus {
    /// The provider message id (the stored `sourceId`).
    pub message_id: String,
    /// `delivered` | `read` | `failed`.
    pub status: String,
    /// Unix ms.
    pub timestamp: Option<i64>,
    /// The provider error text of a `failed` status.
    pub error: Option<String>,
    /// The recipient (phone / BSUID) the status is for.
    pub recipient: Option<String>,
}

fn missing<T: Send + 'static>(
    ctx: &ChannelCtx,
    group: &str,
    name: &str,
) -> BoxFuture<'static, ChannelResult<T>> {
    let message = format!(
        "Channel handler {group}.{name} is not available for {}",
        ctx.channel
    );
    Box::pin(std::future::ready(Err(SendFailure::Other(message))))
}

/// One channel (`Integration` with its channel handlers). Every handler
/// the channel does not implement fails like `runChannelHandler` with no
/// handler.
pub trait Channel: Send + Sync {
    /// `deliveryReceipts`: the channel reports deliveries itself (then the
    /// worker does not record "Last Delivered" at send time).
    fn delivery_receipts(&self) -> bool {
        false
    }

    /// `message.receiveMessage`: parse one queued webhook payload
    /// (`IntegrationJobReceiveMessage.data.payload`). `None` = nothing to
    /// store (the job is dropped).
    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let _ = payload;
        missing(ctx, "message", "receiveMessage")
    }

    /// `message.handleMessageStatus`: parse one queued status payload.
    fn message_status(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<MessageStatus>>> {
        let _ = payload;
        missing(ctx, "message", "handleMessageStatus")
    }

    /// `message.sendMessage`.
    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let _ = data;
        missing(ctx, "message", "sendMessage")
    }

    /// `message.sendFlowStep`.
    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let _ = data;
        missing(ctx, "message", "sendFlowStep")
    }

    /// `comment.sendComment`.
    fn send_comment(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let _ = data;
        missing(ctx, "comment", "sendComment")
    }

    /// `comment.sendPrivateReply`.
    fn send_private_reply(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let _ = data;
        missing(ctx, "comment", "sendPrivateReply")
    }

    /// `comment.deleteComment`.
    fn delete_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let _ = comment_id;
        missing(ctx, "comment", "deleteComment")
    }

    /// `comment.editComment`.
    fn edit_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        new_text: String,
        new_attachment_url: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let _ = (comment_id, new_text, new_attachment_url);
        missing(ctx, "comment", "editComment")
    }

    /// `comment.likeComment`.
    fn like_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        liked: bool,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let _ = (comment_id, liked);
        missing(ctx, "comment", "likeComment")
    }

    /// `comment.hideComment` (`post_id` for TikTok).
    fn hide_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        hidden: bool,
        post_id: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let _ = (comment_id, hidden, post_id);
        missing(ctx, "comment", "hideComment")
    }

    /// `conversation.sendTyping`.
    fn send_typing(
        &self,
        ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let _ = data;
        missing(ctx, "conversation", "sendTyping")
    }
}

/// The registered channels (`allIntegrations` / `getRegisteredIntegration`).
#[derive(Clone, Default)]
pub struct Channels {
    by_name: HashMap<String, Arc<dyn Channel>>,
}

impl Channels {
    pub fn new() -> Channels {
        Channels::default()
    }

    /// The channels of this crate (webchat, API channel).
    pub fn builtin() -> Channels {
        Channels::new()
            .with("webchat", Arc::new(crate::channels::webchat::Webchat))
            .with("api", Arc::new(crate::channels::api::ApiChannel::new()))
            .with(
                "whatsapp",
                Arc::new(crate::channels::whatsapp::Whatsapp::new()),
            )
    }

    pub fn with(mut self, channel: &str, implementation: Arc<dyn Channel>) -> Channels {
        self.by_name.insert(channel.to_owned(), implementation);
        self
    }

    pub fn get(&self, channel: &str) -> Option<Arc<dyn Channel>> {
        self.by_name.get(channel).cloned()
    }

    /// `resolveIntegrationContextFromContactInbox`: the channel and the
    /// integration row of the contact's inbox.
    pub async fn resolve(
        &self,
        db: &Arc<Db>,
        realtime: Option<&Realtime>,
        workspace_id: i64,
        contact_inbox: &ContactInbox,
    ) -> ChannelResult<(Arc<dyn Channel>, ChannelCtx)> {
        let channel_name = contact_inbox.channel.clone();
        let Some(channel) = self.get(&channel_name) else {
            return Err(SendFailure::Other(format!(
                "No integration registered for channel: {channel_name}"
            )));
        };
        let Some(table) = integration_table(&channel_name) else {
            return Err(ChannelError::new(
                format!("Unsupported integration channel: {channel_name}"),
                Category::AuthFailed,
            )
            .with_code("unsupported_channel")
            .into());
        };
        let inbox_id = contact_inbox.inbox_id;
        let row = db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        &format!(
                            r#"SELECT "id", "auth" FROM "{table}" WHERE "inboxId" = ?1 LIMIT 1"#
                        ),
                        params![inbox_id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                    )
                    .optional()?)
            })
            .await
            .map_err(|e| SendFailure::Other(e.to_string()))?;
        let Some((integration_id, auth)) = row else {
            return Err(ChannelError::new(
                format!("Unable to find integration auth for channel: {channel_name}"),
                Category::AuthFailed,
            )
            .with_code("integration_auth_missing")
            .into());
        };
        let ctx = ChannelCtx {
            workspace_id,
            channel: channel_name,
            inbox_id,
            integration_id,
            auth: serde_json::from_str(&auth).unwrap_or(Value::Null),
            db: Arc::clone(db),
            realtime: realtime.cloned(),
        };
        Ok((channel, ctx))
    }
}

/// The inbox an inbound job is for (`identifyInboxAndIntegrationAuthFromIdentifier`).
#[derive(Debug, Clone, PartialEq)]
pub struct InboundInbox {
    pub inbox_id: i64,
    pub workspace_id: i64,
    /// `Inbox.channel`.
    pub channel: String,
}

impl Channels {
    /// `identifyInboxAndIntegrationAuthFromIdentifier`: the channel, the
    /// integration row found by the channel's identifier column and its
    /// inbox. `Ok(None)` = no such integration (`IntegrationNotFoundError`).
    pub async fn resolve_by_identifier(
        &self,
        db: &Arc<Db>,
        realtime: Option<&Realtime>,
        integration_type: &str,
        identifier: &str,
    ) -> ChannelResult<Option<(Arc<dyn Channel>, ChannelCtx, InboundInbox)>> {
        let Some(channel) = self.get(integration_type) else {
            return Err(SendFailure::Other(format!(
                "Unsupported integration: {integration_type}"
            )));
        };
        let Some((table, column)) = identifier_column(integration_type) else {
            return Err(SendFailure::Other(format!(
                "Unsupported integration: {integration_type}"
            )));
        };
        let identifier = identifier.to_owned();
        let row = db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        &format!(
                            r#"SELECT t."id", t."auth", i."id", i."workspaceId", i."channel"
                               FROM "{table}" t JOIN "Inbox" i ON i."id" = t."inboxId"
                               WHERE t."{column}" = ?1 LIMIT 1"#
                        ),
                        params![identifier],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                InboundInbox {
                                    inbox_id: r.get(2)?,
                                    workspace_id: r.get(3)?,
                                    channel: r.get(4)?,
                                },
                            ))
                        },
                    )
                    .optional()?)
            })
            .await
            .map_err(|e| SendFailure::Other(e.to_string()))?;
        Ok(row.map(|(integration_id, auth, inbox)| {
            let ctx = ChannelCtx {
                workspace_id: inbox.workspace_id,
                channel: integration_type.to_owned(),
                inbox_id: inbox.inbox_id,
                integration_id,
                auth: serde_json::from_str(&auth).unwrap_or(Value::Null),
                db: Arc::clone(db),
                realtime: realtime.cloned(),
            };
            (channel, ctx, inbox)
        }))
    }
}

/// The identifier column of each channel's integration table.
fn identifier_column(channel: &str) -> Option<(&'static str, &'static str)> {
    Some(match channel {
        "whatsapp" => ("IntegrationWhatsapp", "phoneNumberId"),
        "telegram" => ("IntegrationTelegram", "botId"),
        "messenger" => ("IntegrationMessenger", "pageId"),
        "zalo" => ("IntegrationZalo", "oaId"),
        "instagram" | "instagramFacebook" => ("IntegrationInstagram", "igId"),
        "tiktok" => ("IntegrationTiktok", "openId"),
        "threads" => ("IntegrationThreads", "threadsUserId"),
        "webchat" => ("IntegrationWebchat", "inboxId"),
        "api" => ("IntegrationApi", "inboxId"),
        _ => return None,
    })
}

/// `integrationTableByChannel` (+ Threads).
fn integration_table(channel: &str) -> Option<&'static str> {
    Some(match channel {
        "api" => "IntegrationApi",
        "instagram" => "IntegrationInstagram",
        "messenger" => "IntegrationMessenger",
        "smtp" => "IntegrationSmtp",
        "telegram" => "IntegrationTelegram",
        "tiktok" => "IntegrationTiktok",
        "webchat" => "IntegrationWebchat",
        "whatsapp" => "IntegrationWhatsapp",
        "zalo" => "IntegrationZalo",
        "threads" => "IntegrationThreads",
        _ => return None,
    })
}

/// `NO_QUEUE_RETRY_CHANNELS`: non-idempotent sends (a retry can post twice).
const NO_QUEUE_RETRY_CHANNELS: [&str; 2] = ["messenger", "instagram"];

/// `shouldSuppressRetryableChannelError`.
pub fn should_suppress_retryable_channel_error(error: &SendFailure, channel: &str) -> bool {
    let SendFailure::Channel(error) = error else {
        return false;
    };
    if !error.is_retryable() {
        return true;
    }
    NO_QUEUE_RETRY_CHANNELS.contains(&channel) && error.category == Category::NetworkError
}

/// `willSendRetry`.
pub fn will_send_retry(error: &SendFailure, channel: &str, will_retry_on_throw: bool) -> bool {
    will_retry_on_throw && !should_suppress_retryable_channel_error(error, channel)
}

/// One call a [`RecordingChannel`] received: handler name and its data.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCall {
    pub handler: &'static str,
    pub data: Value,
}

/// Test double: records every call; answers `{ messageIds: ["mid-<n>"],
/// sentCount: 1 }` unless a failure was queued with
/// [`RecordingChannel::fail_next`].
#[derive(Default)]
pub struct RecordingChannel {
    calls: Mutex<Vec<RecordedCall>>,
    failures: Mutex<Vec<SendFailure>>,
    pub receipts: bool,
}

impl RecordingChannel {
    pub fn new() -> Arc<RecordingChannel> {
        Arc::new(RecordingChannel::default())
    }

    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn fail_next(&self, failure: SendFailure) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(failure);
    }

    fn record<T: Send + 'static>(
        &self,
        handler: &'static str,
        data: Value,
        ok: impl FnOnce(usize) -> T,
    ) -> BoxFuture<'static, ChannelResult<T>> {
        let n = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push(RecordedCall { handler, data });
            calls.len()
        };
        let failure = {
            let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
            (!failures.is_empty()).then(|| failures.remove(0))
        };
        let result = match failure {
            Some(failure) => Err(failure),
            None => Ok(ok(n)),
        };
        Box::pin(std::future::ready(result))
    }

    fn sent(n: usize) -> SendResult {
        SendResult {
            message_ids: vec![format!("mid-{n}")],
            sent_count: 1,
        }
    }
}

fn contact_json(contact: &OutgoingContact) -> Value {
    json!({
        "id": contact.contact_inbox.id.to_string(),
        "sourceId": contact.contact_inbox.source_id,
        "sourceConversationId": contact.source_conversation_id,
    })
}

impl Channel for RecordingChannel {
    fn delivery_receipts(&self) -> bool {
        self.receipts
    }

    fn send_message(
        &self,
        _ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let value = json!({
            "contact": contact_json(&data.contact),
            "message": data.message,
            "quickReplies": data.quick_replies,
            "metadata": data.metadata,
            "sendFrom": data.send_from,
        });
        self.record("sendMessage", value, RecordingChannel::sent)
    }

    fn send_flow_step(
        &self,
        _ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let value = json!({
            "contact": contact_json(&data.contact),
            "flowId": data.flow_id,
            "flowVersionId": data.flow_version_id,
            "step": data.step,
            "quickReplies": data.quick_replies,
            "metadata": data.metadata,
            "commentAnchor": data.comment_anchor,
        });
        self.record("sendFlowStep", value, RecordingChannel::sent)
    }

    fn send_comment(
        &self,
        _ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let value = json!({ "contact": contact_json(&data.contact), "message": data.message });
        self.record("sendComment", value, RecordingChannel::sent)
    }

    fn send_private_reply(
        &self,
        _ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let value = json!({ "contact": contact_json(&data.contact), "message": data.message });
        self.record("sendPrivateReply", value, RecordingChannel::sent)
    }

    fn delete_comment(
        &self,
        _ctx: &ChannelCtx,
        comment_id: String,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        self.record("deleteComment", json!({ "commentId": comment_id }), |_| ())
    }

    fn edit_comment(
        &self,
        _ctx: &ChannelCtx,
        comment_id: String,
        new_text: String,
        new_attachment_url: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let value = json!({
            "commentId": comment_id,
            "newText": new_text,
            "newAttachmentUrl": new_attachment_url,
        });
        self.record("editComment", value, |_| ())
    }

    fn like_comment(
        &self,
        _ctx: &ChannelCtx,
        comment_id: String,
        liked: bool,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let value = json!({ "commentId": comment_id, "liked": liked });
        self.record("likeComment", value, |_| ())
    }

    fn hide_comment(
        &self,
        _ctx: &ChannelCtx,
        comment_id: String,
        hidden: bool,
        post_id: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let value = json!({ "commentId": comment_id, "hidden": hidden, "postId": post_id });
        self.record("hideComment", value, |_| ())
    }

    fn send_typing(
        &self,
        _ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let value = json!({
            "contact": contact_json(&data.contact),
            "typing": data.typing,
            "seconds": data.seconds,
            "messageSourceId": data.message_source_id,
        });
        self.record("sendTyping", value, |_| ())
    }
}

#[cfg(test)]
mod tests {
    //! `apps/worker/__tests__/will-send-retry.test.ts`.
    use super::*;

    fn network_error() -> SendFailure {
        ChannelError::new("socket hang up", Category::NetworkError).into()
    }

    fn rate_limited() -> SendFailure {
        ChannelError::new("slow down", Category::RateLimited).into()
    }

    #[test]
    fn is_false_once_nothing_follows_this_emission() {
        assert!(!will_send_retry(&rate_limited(), "whatsapp", false));
    }

    #[test]
    fn is_true_while_an_attempt_is_still_in_hand() {
        assert!(will_send_retry(&rate_limited(), "whatsapp", true));
    }

    #[test]
    fn is_false_for_a_messenger_network_error_which_is_suppressed_not_rethrown() {
        let error = network_error();
        let SendFailure::Channel(channel_error) = &error else {
            unreachable!()
        };
        assert!(channel_error.is_retryable());
        assert!(!will_send_retry(&error, "messenger", true));
    }

    #[test]
    fn keeps_retrying_the_same_network_error_on_a_channel_that_is_safe_to_retry() {
        assert!(will_send_retry(&network_error(), "whatsapp", true));
    }
}
