//! The shared inbound pipeline every channel reuses
//! (`apps/worker/src/integration/handlers/{incoming-message,received-message,
//! message-status}.ts`): a channel webhook queues `incomingMessage` /
//! `messageStatus` on the `integration` queue with `{ integrationType,
//! integrationIdentifier, payload }`; this module resolves the integration,
//! lets the channel parse the payload ([`Channel::receive_message`] /
//! [`Channel::message_status`]), then finds or creates the contact, contact
//! inbox and conversation, stores the message (deduplicated by `sourceId`),
//! publishes `messageCreated` and routes the message (keywords → AI through
//! the `ai-agent` queue, flow postbacks / quick replies, `ref`).

use std::sync::Arc;

use chatbotx_business::message::{contact_inbox_json, conversation_json, message_json};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::message::{self as repo_message, CreateAttachmentInput, CreateMessageInput};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_jobs::options::Dedup;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use chatbotx_kv::SystemClock;
use chatbotx_kv::durable::DurableKv;
use chatbotx_rt::EventType;
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingContact, IncomingMessage, Received};
use crate::{Deps, db_fail, fail, str_of};

/// `IntegrationJobAction.incomingMessage`.
pub const INCOMING_MESSAGE: &str = "incomingMessage";
/// `IntegrationJobAction.messageStatus`.
pub const MESSAGE_STATUS: &str = "messageStatus";
/// `AIJobAction.processAutomatedResponse`.
pub const PROCESS_AUTOMATED_RESPONSE: &str = "processAutomatedResponse";

/// `AUTOMATED_RESPONSE_DELAY_SECONDS` / `AUTOMATED_RESPONSE_TTL_SECONDS`
/// (default 2 s each).
fn automated_response_timing() -> (u64, u64) {
    let read = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(2)
    };
    (
        read("AUTOMATED_RESPONSE_DELAY_SECONDS"),
        read("AUTOMATED_RESPONSE_TTL_SECONDS"),
    )
}

/// `getKey` of `@chatbotx.io/automated-response`.
pub fn automated_response_key(conversation_id: i64, contact_inbox_id: i64) -> String {
    format!("automated-response:{conversation_id}-{contact_inbox_id}:messages")
}

pub(crate) fn kv(deps: &Deps) -> DurableKv {
    DurableKv::new(Arc::clone(&deps.db), Arc::new(SystemClock))
}

/// What `receiveMessage` stored and the routing inputs (`processIncomingMessage`).
#[derive(Debug, Clone)]
pub struct Stored {
    pub inbox: InboundInbox,
    pub conversation: Conversation,
    pub contact_inbox: ContactInbox,
    /// The new message row JSON (`None` = a redelivery or nothing stored).
    pub message: Option<Value>,
    pub message_id: Option<i64>,
    pub received: Received,
    pub is_new_contact: bool,
    /// A step's Skip tap: resolved by its own `resolveStepInput` job, never
    /// routed as a typed reply.
    pub step_input_skip: bool,
}

/// `IntegrationJobAction.incomingMessage`: [`receive_message`] then the
/// routing of `processIncomingMessage`.
pub async fn incoming_message(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(stored) = receive_message(deps, data).await? else {
        return Ok(());
    };
    route_incoming_message(deps, &stored).await
}

/// `receiveMessage` (received-message.ts): parse, find or create the
/// contact and conversation, store and broadcast the message. `None` = the
/// channel had nothing to store.
pub async fn receive_message(deps: &Deps, data: &Value) -> Result<Option<Stored>, JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let identifier = data
        .get("integrationIdentifier")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let resolved = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            &integration_type,
            &identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?;
    let Some((channel, ctx, inbox)) = resolved else {
        // `IntegrationNotFoundError`: the integration was disconnected.
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
    };
    let payload = data.get("payload").cloned().unwrap_or(Value::Null);
    let received = channel
        .receive_message(&ctx, payload)
        .await
        .map_err(|e| fail(e.to_string()))?;
    let Some(mut received) = received else {
        tracing::warn!(
            integration_type,
            inbox_id = inbox.inbox_id,
            workspace_id = inbox.workspace_id,
            "Received message has no storable content, skipping"
        );
        return Ok(None);
    };
    let suppress = received
        .thread_control
        .as_ref()
        .and_then(|t| str_of(t, "delivery"))
        == Some("standby");
    // Q.D9 hook: `parseAppointmentCancelPostback(rawPostbackAction)` (skipped
    // on a standby delivery) and its `cancelAppointmentByToken` run go here.
    let skip_challenge_id = step_input_skip_challenge_id(&received);
    sanitize_actions(
        &mut received,
        &integration_type,
        skip_challenge_id.is_some(),
    );
    if is_third_party_echo(&received) {
        let (probe_inbox, contact) = (inbox.clone(), received.contact.clone());
        let known = deps
            .db
            .read(move |conn| Ok(find_existing_contact_inbox(conn, &probe_inbox, &contact)?))
            .await
            .map_err(db_fail)?;
        if known.is_none() {
            tracing::debug!(
                inbox_id = inbox.inbox_id,
                source_id = received.contact.source_id,
                "Skipping third-party echo for an unknown contact"
            );
            return Ok(None);
        }
    }
    // ponytail: the Zalo / Telegram tap label (`resolvePostbackButtonLabel`,
    // `resolveStepInputSkipLabel`), profile lookups, Google click, commerce
    // enrichment and thread-control recording are not ported: they need the
    // channel profile fetcher / ads / commerce units.
    let source = contact_source(received.referral_source.as_deref());
    // Orchestrator decision (DECISIONS 2026-11-17): the contact lookup /
    // creation, the contact updates and the message insert are one write
    // transaction, so no reader sees a new contact without its message.
    let (tx_inbox, tx_received) = (inbox.clone(), received.clone());
    let (contact_inbox, conversation, is_new_contact, saved) = deps
        .db
        .write(move |tx| {
            let (contact_inbox, conversation, is_new_contact) =
                detect_contact_and_conversation_tx(tx, &tx_inbox, &tx_received.contact, source)?;
            let mut saved = None;
            if let Some(incoming) = &tx_received.message {
                if incoming.message_type != "outgoing" {
                    if let Some(text) = incoming.text.as_deref() {
                        update_contact_from_message_tx(
                            tx,
                            &contact_inbox,
                            tx_inbox.workspace_id,
                            text,
                        );
                    }
                    unblock_if_blocked_tx(tx, contact_inbox.contact_id, tx_inbox.workspace_id);
                }
                saved = Some(save_message_tx(
                    tx,
                    &tx_inbox,
                    &contact_inbox,
                    &conversation,
                    incoming,
                    &tx_received,
                )?);
            }
            Ok((contact_inbox, conversation, is_new_contact, saved))
        })
        .await
        .map_err(db_fail)?;

    let mut message = None;
    let mut message_id = None;
    if let Some((stored, is_new)) = saved {
        let row = broadcast_saved(deps, &inbox, &conversation, &stored, is_new, None).await;
        // Also on a redelivery: the job is deduplicated and the challenge
        // claim settles it once.
        if let Some(challenge_id) = &skip_challenge_id
            && !suppress
            && workspace_is_active(deps, inbox.workspace_id).await?
        {
            let job = chatbotx_jobs::queues::resolve_step_input::build_resolve_step_input_job(
                &chatbotx_jobs::queues::resolve_step_input::ResolveStepInput {
                    conversation_id: conversation.id.to_string(),
                    contact_inbox_id: contact_inbox.id.to_string(),
                    challenge_id: challenge_id.clone(),
                    reason: chatbotx_jobs::queues::resolve_step_input::ResolveStepInputReason::UserSkipped,
                    message_id: Some(stored.message.id.to_string()),
                    message_created_at: Some(chatbotx_db::repo::iso(stored.message.created_at)),
                },
            );
            add_job(deps, QueueName::Integration, &job.name, job.data, job.opts).await?;
        }
        if is_new {
            // Q.D13 hook: `awardInboundMessageLeaderboard` goes here.
            message_id = row["id"].as_str().and_then(|s| s.parse().ok());
            message = Some(row);
        }
    }
    Ok(Some(Stored {
        step_input_skip: skip_challenge_id.is_some(),
        inbox,
        conversation,
        contact_inbox,
        message,
        message_id,
        received,
        is_new_contact,
    }))
}

/// The Skip tap's challenge id (`decodeStepInputSkipPostback` of the raw
/// postback, then of the raw quick reply).
fn step_input_skip_challenge_id(received: &Received) -> Option<String> {
    use chatbotx_core::flow_config::step_input_resolution::decode_step_input_skip_postback;
    decode_step_input_skip_postback(received.postback_action.as_deref())
        .or_else(|| decode_step_input_skip_postback(received.quick_reply_action.as_deref()))
}

/// `sanitizeFlowAction` of both taps, the contact-hint quick reply (its full
/// value becomes the text) and the value quick replies (a payload equal to
/// the text is a typed reply). Only Messenger accepts a bare flow id.
fn sanitize_actions(received: &mut Received, integration_type: &str, skip_tap: bool) {
    use chatbotx_core::flow_config::steps::decode_contact_hint_quick_reply_payload;
    let allow_bare = integration_type == "messenger";
    let keep = |raw: Option<String>, kind: &str| {
        let raw = raw.filter(|r| !r.is_empty())?;
        if chatbotx_core::flow_config::util::decode_button_payload(&raw, allow_bare).is_some() {
            return Some(raw);
        }
        tracing::warn!(
            kind,
            integration_type,
            action = raw,
            "Dropping undecodable flow action from channel webhook"
        );
        None
    };
    let hint = decode_contact_hint_quick_reply_payload(received.quick_reply_action.as_deref());
    let text = received.message.as_ref().and_then(|m| m.text.clone());
    if let (Some(hint), Some(message)) = (&hint, received.message.as_mut()) {
        message.text = Some(hint.clone());
    }
    let value_reply = hint.is_some()
        || (received.quick_reply_action.is_some() && received.quick_reply_action == text);
    let postback = received.postback_action.take().filter(|_| !skip_tap);
    received.postback_action = keep(postback, "postback");
    let quick_reply = received
        .quick_reply_action
        .take()
        .filter(|_| !skip_tap && !value_reply);
    received.quick_reply_action = keep(quick_reply, "quickReply");
}

/// `isThirdPartyEcho`: another app's send mirrored back (not a story reply).
fn is_third_party_echo(received: &Received) -> bool {
    received.echo_origin.as_deref() == Some("thirdParty")
        && received.message.as_ref().is_some_and(|m| {
            m.message_type == "outgoing"
                && !m.content_attributes.as_ref().is_some_and(|a| {
                    let story = if str_of(a, "type") == Some("story_reply") {
                        a.get("story")
                    } else {
                        a.get("storyReply")
                    };
                    story.is_some_and(|s| !s.is_null())
                })
        })
}

/// `resolveContactSource` without the Google click (not ported):
/// `metaReferralToContactSource(referralSource) ?? inboundMessage`.
fn contact_source(referral_source: Option<&str>) -> &'static str {
    match referral_source {
        Some("ADS") => "ads",
        Some("SHORTLINK") => "botLink",
        Some("CUSTOMER_CHAT_PLUGIN") => "chatPlugin",
        _ => "inboundMessage",
    }
}

/// `resolveSourceScopedIdentityMatch` over `contactInboxRepository.findWithContact`:
/// `sourceId`, then `sourceUserId`, then `sourceParentUserId`.
fn find_existing_contact_inbox(
    conn: &Connection,
    inbox: &InboundInbox,
    contact: &IncomingContact,
) -> chatbotx_db::rusqlite::Result<Option<(ContactInbox, &'static str)>> {
    let probes: [(&'static str, Option<&str>); 3] = [
        ("sourceId", Some(contact.source_id.as_str())),
        ("sourceUserId", contact.source_user_id.as_deref()),
        (
            "sourceParentUserId",
            contact.source_parent_user_id.as_deref(),
        ),
    ];
    for (column, value) in probes {
        let Some(value) = value else { continue };
        let found = conn
            .query_row(
                &format!(
                    r#"{} WHERE "inboxId" = ?1 AND "channel" = ?2 AND "{column}" = ?3
                       AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = "ContactInbox"."contactId")
                       LIMIT 1"#,
                    ContactInbox::select_sql()
                ),
                params![inbox.inbox_id, inbox.channel, value],
                ContactInbox::from_row,
            )
            .optional()?;
        if let Some(found) = found {
            return Ok(Some((found, column)));
        }
    }
    Ok(None)
}

/// `conversationService.findOrCreate` (inside the caller's transaction).
pub(crate) fn find_or_create_conversation(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    source_id: Option<&str>,
    channel_conversation_id: Option<&str>,
) -> chatbotx_db::rusqlite::Result<Conversation> {
    let find = |conn: &Connection| {
        conn.query_row(
            &format!(
                r#"{} WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS ?3 LIMIT 1"#,
                Conversation::select_sql()
            ),
            params![workspace_id, contact_id, source_id],
            Conversation::from_row,
        )
        .optional()
    };
    let channel_conversation_id = channel_conversation_id.filter(|id| !id.is_empty());
    if let Some(existing) = find(conn)? {
        // `syncChannelConversationId`.
        let Some(channel_id) = channel_conversation_id else {
            return Ok(existing);
        };
        let mut attributes = existing
            .additional_attributes
            .as_ref()
            .map(|a| a.0.clone())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if attributes["channelConversationId"] == channel_id {
            return Ok(existing);
        }
        attributes["channelConversationId"] = json!(channel_id);
        conn.execute(
            r#"UPDATE "Conversation" SET "additionalAttributes" = ?2 WHERE "id" = ?1"#,
            params![existing.id, attributes.to_string()],
        )?;
        return find(conn)?.ok_or(chatbotx_db::rusqlite::Error::QueryReturnedNoRows);
    }
    let attributes =
        channel_conversation_id.map(|id| json!({ "channelConversationId": id }).to_string());
    conn.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "additionalAttributes")
           VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
        params![new_id(), workspace_id, contact_id, source_id, attributes],
    )?;
    find(conn)?.ok_or(chatbotx_db::rusqlite::Error::QueryReturnedNoRows)
}

/// [`detect_contact_and_conversation_tx`] in its own write transaction
/// (for callers outside [`receive_message`]).
pub(crate) async fn detect_contact_and_conversation(
    deps: &Deps,
    inbox: &InboundInbox,
    contact: &IncomingContact,
    source: &'static str,
) -> Result<(ContactInbox, Conversation, bool), JobError> {
    let inbox = inbox.clone();
    let contact = contact.clone();
    deps.db
        .write(move |tx| detect_contact_and_conversation_tx(tx, &inbox, &contact, source))
        .await
        .map_err(db_fail)
}

/// `detectContactAndConversation`: the existing contact inbox (identity
/// sync not ported) or a new contact + contact inbox + conversation in one
/// transaction (the caller's). Returns `(contactInbox, conversation, isNewContact)`.
fn detect_contact_and_conversation_tx(
    tx: &Connection,
    inbox: &InboundInbox,
    contact: &IncomingContact,
    source: &'static str,
) -> Result<(ContactInbox, Conversation, bool), chatbotx_db::DbError> {
    let source_conversation_id = contact.source_conversation_id.as_deref();
    if let Some((existing, _matched_by)) = find_existing_contact_inbox(tx, inbox, contact)? {
        // ponytail: `syncExistingContactIdentity` (BSUID / username
        // learning) is not ported.
        let conversation = find_or_create_conversation(
            tx,
            inbox.workspace_id,
            existing.contact_id,
            source_conversation_id,
            contact.channel_conversation_id.as_deref(),
        )?;
        return Ok((existing, conversation, false));
    }
    // ponytail: the MAC quota gate, `getProfile` lookups and the
    // locale / timezone inference (`finalizeContactProfile`) are not
    // ported; the contact row takes the webhook's fields.
    let contact_id = new_id();
    let now = now_ms();
    let phone_number = contact.phone_number.clone().or_else(|| {
        (inbox.channel == "whatsapp"
            && contact.source_user_id.as_deref() != Some(contact.source_id.as_str())
            && !contact.source_id.is_empty())
        .then(|| format!("+{}", contact.source_id.trim_start_matches('+')))
    });
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName",
                     "lastName", "email", "phoneNumber", "avatar", "locale")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
        params![
            contact_id,
            now,
            inbox.workspace_id,
            contact.first_name,
            contact.last_name,
            contact.email,
            phone_number,
            contact.avatar,
            contact.locale,
        ],
    )?;
    let contact_inbox_id = new_id();
    tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "createdAt", "updatedAt", "inboxId", "contactId",
                     "originalContactId", "source", "sourceId", "sourceUserId", "sourceParentUserId",
                     "sourceUsername", "channel", "language")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?4, ?11, ?5, ?6, ?7, ?8, ?9, ?10)"#,
                params![
                    contact_inbox_id,
                    now,
                    inbox.inbox_id,
                    contact_id,
                    contact.source_id,
                    contact.source_user_id,
                    contact.source_parent_user_id,
                    contact.source_username,
                    inbox.channel,
                    contact.language,
                    source,
                ],
            )?;
    let contact_inbox = tx.query_row(
        &format!(r#"{} WHERE "id" = ?1"#, ContactInbox::select_sql()),
        params![contact_inbox_id],
        ContactInbox::from_row,
    )?;
    let conversation = find_or_create_conversation(
        tx,
        inbox.workspace_id,
        contact_id,
        source_conversation_id,
        contact.channel_conversation_id.as_deref(),
    )?;
    Ok((contact_inbox, conversation, true))
}

/// `updateContactFromMessage`: a phone / email the contact typed overwrites
/// the contact's columns. Failures are logged.
fn update_contact_from_message_tx(
    tx: &Connection,
    contact_inbox: &ContactInbox,
    workspace_id: i64,
    text: &str,
) {
    use chatbotx_business::contact::extract::{ExtractOptions, extract_contact_info};
    let info = extract_contact_info(Some(text), None, ExtractOptions::default());
    if info.phone_number.is_none() && info.email.is_none() {
        return;
    }
    let contact_id = contact_inbox.contact_id;
    let result = tx.execute(
        r#"UPDATE "Contact" SET "phoneNumber" = COALESCE(?1, "phoneNumber"),
             "email" = COALESCE(?2, "email"), "updatedAt" = ?3
           WHERE "id" = ?4 AND "workspaceId" = ?5"#,
        params![
            info.phone_number,
            info.email,
            now_ms(),
            contact_id,
            workspace_id
        ],
    );
    if let Err(err) = result {
        tracing::warn!(err = %err, contact_id, "Contact update from message text failed");
    }
}

/// `contactService.unblockIfBlocked`.
pub(crate) async fn unblock_if_blocked(deps: &Deps, contact_id: i64, workspace_id: i64) {
    let _ = deps
        .db
        .write(move |tx| {
            unblock_if_blocked_tx(tx, contact_id, workspace_id);
            Ok(())
        })
        .await
        .inspect_err(
            |err| tracing::warn!(err = %err, contact_id, "Auto-unblock on inbound message failed"),
        );
}

fn unblock_if_blocked_tx(tx: &Connection, contact_id: i64, workspace_id: i64) {
    let result = tx.execute(
        r#"UPDATE "Contact" SET "blockedAt" = NULL, "updatedAt" = ?1
           WHERE "id" = ?2 AND "workspaceId" = ?3 AND "blockedAt" IS NOT NULL"#,
        params![now_ms(), contact_id, workspace_id],
    );
    if let Err(err) = result {
        tracing::warn!(err = %err, contact_id, "Auto-unblock on inbound message failed");
    }
}

/// `isInboundConversationMessage`.
fn is_inbound_conversation_message(message: &IncomingMessage) -> bool {
    message.message_type == "incoming" && message.kind.as_deref().unwrap_or("message") == "message"
}

/// `saveAndBroadcastMessage` / `persistMessage`: the deduplicated insert,
/// the activity tracking of a new row, the realtime event. Returns the
/// stored row JSON and whether it is new.
pub(crate) async fn save_message(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    incoming: &IncomingMessage,
    received: &Received,
    client_id: Option<&str>,
) -> Result<(Value, bool), JobError> {
    let (i, ci, c, m, r) = (
        inbox.clone(),
        contact_inbox.clone(),
        conversation.clone(),
        incoming.clone(),
        received.clone(),
    );
    let (stored, is_new) = deps
        .db
        .write(move |tx| save_message_tx(tx, &i, &ci, &c, &m, &r))
        .await
        .map_err(db_fail)?;
    Ok((
        broadcast_saved(deps, inbox, conversation, &stored, is_new, client_id).await,
        is_new,
    ))
}

/// The insert + activity tracking of [`save_message`] on the caller's
/// transaction.
fn save_message_tx(
    tx: &Connection,
    inbox: &InboundInbox,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    incoming: &IncomingMessage,
    received: &Received,
) -> Result<(repo_message::MessageWithAttachments, bool), chatbotx_db::DbError> {
    let inbound = incoming.message_type != "outgoing";
    let created_at = now_ms();
    let input = CreateMessageInput {
        id: Some(new_id()),
        contact_inbox_id: contact_inbox.id,
        conversation_id: conversation.id,
        workspace_id: inbox.workspace_id,
        content_attributes: incoming.content_attributes.clone(),
        content_type: incoming.content_type.clone(),
        message_type: incoming.message_type.clone(),
        sender_type: if inbound { "contact" } else { "user" }.into(),
        sender_id: inbound.then_some(contact_inbox.contact_id),
        source_id: incoming.source_id.clone(),
        parent_id: incoming.parent_id.clone(),
        text: incoming.text.clone(),
        r#type: Some(incoming.kind.clone().unwrap_or_else(|| "message".into())),
        created_at: Some(created_at),
        updated_at: None,
    };
    let attachments: Vec<CreateAttachmentInput> = incoming
        .attachments
        .iter()
        .map(|a| CreateAttachmentInput {
            id: None,
            workspace_id: inbox.workspace_id,
            conversation_id: conversation.id,
            message_id: 0,
            message_created_at: 0,
            file_type: a.file_type.clone(),
            mime_type: a.mime_type.clone(),
            origin_path: a.origin_path.clone(),
            created_at: None,
            height: a.height,
            width: a.width,
            keep_original: Some(
                !(a.is_photo_message && matches!(a.file_type.as_str(), "image" | "gif")),
            ),
            name: a.name.clone(),
            size: a.size,
            source_id: a.source_id.clone(),
            thumbnail_path: None,
        })
        .collect();
    let tracking_inbound = is_inbound_conversation_message(incoming);
    let last_btn_title = received.button_title.clone();
    let user_input = incoming.text.clone();
    let user_input_type = (incoming.content_type == "location")
        .then(|| "location".to_owned())
        .or_else(|| incoming.text.is_some().then(|| "text".to_owned()));
    let referral = received.referral.as_ref().map(Value::to_string);
    let ci = contact_inbox;
    let conversation_id = conversation.id;
    let workspace_id = inbox.workspace_id;
    let (stored, is_new) = if attachments.is_empty() {
        let r = repo_message::create_or_update(tx, &input).map_err(crate::repo_err)?;
        (
            repo_message::MessageWithAttachments {
                message: r.message,
                attachment_count: None,
                attachments: Vec::new(),
            },
            r.is_new,
        )
    } else {
        let r = repo_message::create_or_update_with_attachments(tx, &input, &attachments)
            .map_err(crate::repo_err)?;
        (r.result, r.is_new)
    };
    if !is_new {
        return Ok((stored, false));
    }
    let at = stored.message.created_at;
    // `recordInboundActivity`: contact-inbox tracking + the
    // conversation's activity and waiting window.
    tx.execute(
                r#"UPDATE "ContactInbox" SET
                     "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1 THEN ?1 ELSE "firstInteractionAt" END,
                     "lastMessageAt" = ?1,
                     "lastIncomingMessageAt" = CASE WHEN ?2 THEN ?1 ELSE "lastIncomingMessageAt" END,
                     "lastUserInput" = CASE WHEN ?2 AND ?3 IS NOT NULL THEN ?3 ELSE "lastUserInput" END,
                     "lastUserInputType" = CASE WHEN ?2 AND ?4 IS NOT NULL THEN ?4 ELSE "lastUserInputType" END,
                     "lastBtnTitle" = COALESCE(?5, "lastBtnTitle"),
                     "referral" = CASE WHEN ?6 IS NULL THEN "referral"
                                       ELSE json_patch(COALESCE("referral", '{}'), ?6) END,
                     "updatedAt" = ?7
                   WHERE "id" = ?8 AND "contactId" = ?9"#,
                params![
                    at,
                    tracking_inbound,
                    user_input,
                    user_input_type,
                    last_btn_title,
                    referral,
                    now_ms(),
                    ci.id,
                    ci.contact_id
                ],
            )?;
    tx.execute(
                r#"UPDATE "Conversation" SET
                     "lastActivityAt" = ?1,
                     "contactRepliedAt" = CASE WHEN ?2 THEN ?1 ELSE "contactRepliedAt" END,
                     "awaitingHumanReplySince" = CASE WHEN ?2 THEN COALESCE("awaitingHumanReplySince", ?1)
                                                      ELSE "awaitingHumanReplySince" END,
                     "updatedAt" = ?3
                   WHERE "id" = ?4 AND "workspaceId" = ?5"#,
                params![at, inbound, now_ms(), conversation_id, workspace_id],
            )?;
    Ok((stored, true))
}

/// The realtime `messageCreated` event of a new row; returns the row JSON.
async fn broadcast_saved(
    deps: &Deps,
    inbox: &InboundInbox,
    conversation: &Conversation,
    stored: &repo_message::MessageWithAttachments,
    is_new: bool,
    client_id: Option<&str>,
) -> Value {
    let row = message_json(stored);
    if is_new {
        let mut event = row.clone();
        if let Some(client_id) = client_id {
            event["clientId"] = json!(client_id);
        }
        crate::chat::queue_event(
            deps,
            conversation,
            inbox.inbox_id,
            EventType::MessageCreated,
            event,
        )
        .await;
        // ponytail: the incoming notification job, the `message:received`
        // event (flow triggers, Q.W2) and the photo re-encode are not ported.
    }
    row
}

/// `processIncomingMessage`: route a new contact message to its automation.
pub async fn route_incoming_message(deps: &Deps, stored: &Stored) -> Result<(), JobError> {
    let Some(message) = &stored.message else {
        return Ok(());
    };
    let Some(message_id) = stored.message_id else {
        return Ok(());
    };
    let received = &stored.received;
    let conversation = &stored.conversation;
    let workspace_active = workspace_is_active(deps, conversation.workspace_id).await?;
    let suppress = received
        .thread_control
        .as_ref()
        .and_then(|t| str_of(t, "delivery"))
        == Some("standby");
    let can_automate = workspace_active && !suppress;
    if stored.step_input_skip {
        return Ok(());
    }

    // Flow postbacks / quick replies (`automatedResponseService.enqueueFlowAction`).
    let action = received
        .postback_action
        .as_ref()
        .map(|a| ("postback", a))
        .or_else(|| {
            received
                .quick_reply_action
                .as_ref()
                .map(|a| ("quickReply", a))
        });
    if let Some((kind, action)) = action {
        if can_automate {
            enqueue_flow_action(deps, stored, kind, action, message_id).await?;
        }
        return Ok(());
    }
    if let Some(r#ref) = received.r#ref.as_ref().filter(|_| can_automate) {
        // ponytail: `runRef` (ref links) has no Rust handler yet; queued so
        // the job is visible.
        add_job(
            deps,
            QueueName::Integration,
            "runRef",
            json!({ "type": "runRef", "data": {
                "conversationId": conversation_json(conversation),
                "contactInboxId": contact_inbox_json(&stored.contact_inbox),
                "ref": r#ref,
                "messageId": message_id.to_string(),
                "isNewContact": stored.is_new_contact,
            }}),
            JobOptions::default(),
        )
        .await?;
    }
    if !can_automate {
        return Ok(());
    }
    let from_contact = str_of(message, "senderType") == Some("contact");
    let text = str_of(message, "text")
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    let attachment_types: Vec<String> = message["attachments"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| str_of(x, "fileType").map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let is_location = str_of(message, "contentType") == Some("location");
    let has_keyword_media = attachment_types
        .iter()
        .any(|t| chatbotx_business::keyword_match::is_keyword_media_file_type(t));
    let additional = conversation
        .additional_attributes
        .as_ref()
        .map(|j| j.0.clone())
        .unwrap_or(Value::Null);
    let routing = crate::integration::resolve_incoming_text_routing(
        &additional,
        from_contact && (text.is_some() || !attachment_types.is_empty() || is_location),
        from_contact && text.is_some(),
        from_contact && has_keyword_media,
        async || ensure_active(deps, conversation).await,
    )
    .await;
    use crate::integration::IncomingRouting;
    match routing {
        IncomingRouting::AutomatedResponse => {
            enqueue_automated_response(deps, stored, message_id, text, attachment_types).await?;
        }
        IncomingRouting::Challenge(challenge) => {
            add_job(
                deps,
                QueueName::Integration,
                "runChallenge",
                json!({ "type": "runChallenge", "data": {
                    "conversationId": conversation.id.to_string(),
                    "contactInboxId": stored.contact_inbox.id.to_string(),
                    "messageId": message_id.to_string(),
                    "messageCreatedAt": message["createdAt"],
                    "challenge": challenge,
                }}),
                JobOptions {
                    job_id: Some(format!(
                        "questionnaire-challenge-{}-{message_id}",
                        conversation.id
                    )),
                    ..JobOptions::default()
                },
            )
            .await?;
        }
        // ponytail: handoff re-entry needs the AI handover port (Q.D6).
        IncomingRouting::HandoffReentry | IncomingRouting::None => {}
    }
    Ok(())
}

pub(crate) async fn add_job(
    deps: &Deps,
    queue: QueueName,
    name: &str,
    data: Value,
    opts: JobOptions,
) -> Result<(), JobError> {
    deps.jobs
        .add(queue, name, data, opts)
        .await
        .map(|_| ())
        .map_err(|e| fail(e.to_string()))
}

/// `workspaceService.isActiveNow(workspace)`.
pub(crate) async fn workspace_is_active(deps: &Deps, workspace_id: i64) -> Result<bool, JobError> {
    let row = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "isActive", "startTime", "endTime", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    params![workspace_id],
                    |r| {
                        Ok((
                            r.get::<_, bool>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    Ok(row.is_some_and(|(active, start, end, tz)| {
        chatbotx_business::workspace::is_active_now(
            active,
            start.as_deref(),
            end.as_deref(),
            &tz,
            now_ms(),
        )
    }))
}

/// `conversationService.ensureActive`: bot on, or a passed `botResumeAt`
/// turns it back on.
pub(crate) async fn ensure_active(deps: &Deps, conversation: &Conversation) -> bool {
    if conversation.bot_enabled {
        return true;
    }
    let Some(resume_at) = conversation.bot_resume_at.filter(|at| *at <= now_ms()) else {
        return false;
    };
    let _ = resume_at;
    let (id, workspace_id) = (conversation.id, conversation.workspace_id);
    let result = deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Conversation" SET "botEnabled" = 1, "botResumeAt" = NULL, "updatedAt" = ?1
                   WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                params![now_ms(), id, workspace_id],
            )?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, conversation_id = id, "Bot auto-resume failed");
    }
    true
}

/// `automatedResponseService.enqueue` (`enqueueMessage`): the delayed,
/// debounced `processAutomatedResponse` job plus the pending-message list.
/// A media-only message is queued only when a keyword rule matches it.
pub(crate) async fn enqueue_automated_response(
    deps: &Deps,
    stored: &Stored,
    message_id: i64,
    text: Option<String>,
    attachment_file_types: Vec<String>,
) -> Result<(), JobError> {
    use chatbotx_business::keyword_match::{MatchInput, matches_any_keyword_rule};
    let conversation_id = stored.conversation.id;
    let contact_inbox_id = stored.contact_inbox.id;
    let workspace_id = stored.conversation.workspace_id;
    let has_input = text.is_some() || !attachment_file_types.is_empty();
    if text.is_none() && has_input {
        let rules = crate::ai_agent::inbound_rules(deps, workspace_id).await?;
        let input = MatchInput {
            text: None,
            attachment_file_types,
        };
        if !matches_any_keyword_rule(
            &input,
            &rules.iter().map(|r| r.1.clone()).collect::<Vec<_>>(),
        ) {
            return Ok(());
        }
    }
    // ponytail: Smart Delay (`smartResponseDelaySeconds`) only applies with
    // an AI agent / Global Router (Q.D6); until then the default timing.
    let (delay_secs, ttl_secs) = automated_response_timing();
    let key = automated_response_key(conversation_id, contact_inbox_id);
    let opts = JobOptions {
        job_id: Some(format!("automated-response-{message_id}")),
        delay_ms: Some(delay_secs * 1000),
        dedup: Some(Dedup {
            id: key.clone(),
            ttl_ms: Some(ttl_secs * 1000),
            extend: true,
            replace: true,
            keep_last_if_active: false,
        }),
        ..JobOptions::default()
    };
    let data = json!({ "type": PROCESS_AUTOMATED_RESPONSE, "data": {
        "conversationId": conversation_id.to_string(),
        "contactInboxId": contact_inbox_id.to_string(),
        "messageId": message_id.to_string(),
    }});
    // The list is written first so the job always finds its message.
    if let Err(err) = kv(deps)
        .push(&key, &message_id.to_string(), ttl_secs * 5)
        .await
    {
        tracing::error!(err = %err, "Unable to trigger automated response");
        return Ok(());
    }
    if let Err(err) = deps
        .jobs
        .add(QueueName::AiAgent, PROCESS_AUTOMATED_RESPONSE, data, opts)
        .await
    {
        tracing::error!(err = %err, "Unable to trigger automated response");
    }
    Ok(())
}

/// `automatedResponseService.enqueueFlowAction`: a button tap starts the
/// flow node it names (`runFlowPostback` / `runFlowQuickReply`).
pub(crate) async fn enqueue_flow_action(
    deps: &Deps,
    stored: &Stored,
    kind: &str,
    action: &str,
    message_id: i64,
) -> Result<(), JobError> {
    let name = if kind == "postback" {
        "runFlowPostback"
    } else {
        "runFlowQuickReply"
    };
    let mut payload = json!({});
    if let Some(flow_response) = stored
        .received
        .message
        .as_ref()
        .and_then(|m| m.content_attributes.as_ref())
        .and_then(|a| a.get("flowResponse"))
    {
        payload["waFlowResponse"] = flow_response.clone();
    }
    // ponytail: the per-action debounce key and the run handlers
    // (`runFlowPostback` / `runFlowQuickReply`) are the flow-action unit's
    // (Q.W3); the job is queued with the original payload.
    add_job(
        deps,
        QueueName::Integration,
        name,
        json!({ "type": name, "data": {
            "conversationId": conversation_json(&stored.conversation),
            "contactInboxId": contact_inbox_json(&stored.contact_inbox),
            "action": action,
            "ref": stored.received.r#ref,
            "messageId": message_id.to_string(),
            "payload": payload,
        }}),
        JobOptions::default(),
    )
    .await
}

/// `IntegrationJobAction.messageStatus` (`message-status.ts`): the channel
/// reads the status; a `failed` status stores the send error, `delivered` /
/// `read` stamp the contact inbox.
pub async fn message_status(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let identifier = str_of(data, "integrationIdentifier")
        .unwrap_or("")
        .to_owned();
    let Some((channel, ctx, inbox)) = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            &integration_type,
            &identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
    else {
        return Ok(());
    };
    let payload = data.get("payload").cloned().unwrap_or(Value::Null);
    let Some(status) = channel
        .message_status(&ctx, payload)
        .await
        .map_err(|e| fail(e.to_string()))?
    else {
        return Ok(());
    };
    let workspace_id = inbox.workspace_id;
    let source_id = status.message_id.clone();
    let found = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT m."id", m."createdAt", m."conversationId", m."contactInboxId", m."contentAttributes"
                       FROM "Message" m WHERE m."workspaceId" = ?1 AND m."sourceId" = ?2
                       ORDER BY m."createdAt" DESC LIMIT 1"#,
                    params![workspace_id, source_id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some((message_id, created_at, conversation_id, contact_inbox_id, attributes)) = found
    else {
        return Ok(());
    };
    let Some(conversation) = crate::load_conversation(&deps.db, conversation_id).await? else {
        return Ok(());
    };
    // `message:delivered` / `message:failed` with the send's metadata (the
    // broadcast-ops listener). ponytail: `message:seen` and
    // `message:received` feed analytics only (Q.D13).
    if matches!(status.status.as_str(), "delivered" | "failed")
        && let Some(contact_inbox) =
            crate::load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    {
        let attributes: Value = attributes
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or(Value::Null);
        let metadata = attributes
            .get("metadata")
            .filter(|m| m.is_object())
            .cloned()
            .unwrap_or_else(|| json!({ "type": "updateStatus" }));
        let mut action = json!({ "messageId": message_id.to_string() });
        for key in ["flowId", "flowVersionId"] {
            if let Some(value) = attributes.get(key).filter(|v| v.is_string()) {
                action[key] = value.clone();
            }
        }
        let at = status.timestamp.unwrap_or_else(now_ms);
        let ctx = crate::events::MessageContext::of(&conversation, &contact_inbox);
        let mut extra = json!({ "nodeId": attributes.get("nodeId").cloned().unwrap_or(json!("")) });
        let event_type = if status.status == "delivered" {
            extra["providerOccurredAt"] = json!(chatbotx_db::repo::iso(at));
            "message:delivered"
        } else {
            extra["errorData"] = status
                .error
                .as_ref()
                .map_or(json!({}), |e| json!({ "message": e }));
            extra["willRetry"] = json!(false);
            "message:failed"
        };
        crate::events::emit_message_event(
            deps.events.as_ref(),
            event_type,
            &ctx,
            action,
            Some(&metadata),
            extra,
        )
        .await;
    }
    match status.status.as_str() {
        "failed" => {
            crate::chat::persist_send_error(
                deps,
                &conversation,
                inbox.inbox_id,
                (message_id, created_at, None),
                Some(status.error.unwrap_or_else(|| "Message failed".into())),
                false,
            )
            .await;
        }
        "delivered" | "read" => {
            // ponytail: the message delivery / read state and `markReadByContact`
            // are not ported; the "Last Delivered" field is.
            let at = status.timestamp.unwrap_or_else(now_ms);
            let contact_id = conversation.contact_id;
            let result = deps
                .db
                .write(move |tx| {
                    chatbotx_db::repo::contact_activity::upsert_last_delivered(
                        tx,
                        workspace_id,
                        &[(contact_id, at)],
                    )
                    .map_err(crate::repo_err)?;
                    Ok(())
                })
                .await;
            if let Err(err) = result {
                tracing::warn!(err = %err, contact_inbox_id, "Recording the delivery failed");
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The API channel through the shared pipeline: contact, conversation
    //! and message rows, the redelivery dedup, the debounced keyword job and
    //! its text reply (`41-keywords-trigger` in small).
    use std::sync::Mutex;

    use chatbotx_db::Db;
    use chatbotx_jobs::{AddResult, Enqueuer, JobsError, NewJob};

    use super::*;

    #[derive(Default)]
    struct Recording(Mutex<Vec<(QueueName, String, Value, JobOptions)>>);

    impl Enqueuer for Recording {
        fn add(
            &self,
            queue: QueueName,
            name: &str,
            data: Value,
            opts: JobOptions,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<AddResult, JobsError>> {
            self.0
                .lock()
                .unwrap()
                .push((queue, name.into(), data, opts));
            Box::pin(async {
                Ok(AddResult {
                    job_id: "1".into(),
                    created: true,
                })
            })
        }

        fn add_bulk(
            &self,
            _: QueueName,
            _: Vec<NewJob>,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    async fn setup() -> (Deps, Arc<Recording>) {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-inbound-{}-{}",
            std::process::id(),
            new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(dir.join("t.db")).unwrap());
        db.migrate().await.unwrap();
        db.write(|tx| {
            for sql in [
                r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Api', 'api', '4')"#,
                r#"INSERT INTO "IntegrationApi" ("id", "auth", "name", "tokenHash", "tokenPrefix", "workspaceId", "inboxId") VALUES (5, '{}', 'Api', 'h', 'p', 2, 4)"#,
                r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "text", "type") VALUES (6, 2, '["hello"]', 1, 'Hi there', 'inbound')"#,
            ] {
                tx.execute(sql, [])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let jobs = Arc::new(Recording::default());
        let deps = Deps {
            db,
            jobs: jobs.clone(),
            waiter: chatbotx_jobs::wait::CompletionRegistry::new(),
            realtime: None,
            channels: crate::Channels::builtin(),
            app_url: "http://app.test".into(),
            variables: None,
            ai: Arc::new(crate::ai::Ai::live()),
            files: None,
            webhook_http: Arc::new(crate::webhook::RecordingWebhookHttp::default()),
            events: None,
            push: None,
        };
        (deps, jobs)
    }

    fn payload() -> Value {
        json!({ "integrationType": "api", "integrationIdentifier": "4", "payload": {
            "contact": { "sourceId": "c-1", "firstName": "Ada" },
            "message": { "sourceId": "m-1", "text": "hello", "contentType": "text" },
        }})
    }

    async fn count(deps: &Deps, sql: &'static str) -> i64 {
        deps.db
            .read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn api_message_creates_rows_once_and_the_keyword_rule_replies() {
        let (deps, jobs) = setup().await;
        incoming_message(&deps, &payload()).await.unwrap();
        incoming_message(&deps, &payload()).await.unwrap();

        assert_eq!(
            count(
                &deps,
                r#"SELECT count(*) FROM "Contact" WHERE "firstName" = 'Ada'"#
            )
            .await,
            1
        );
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "ContactInbox" WHERE "sourceId" = 'c-1' AND "channel" = 'api' AND "lastIncomingMessageAt" IS NOT NULL"#).await,
            1
        );
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "Conversation""#).await,
            1
        );
        assert_eq!(
            count(
                &deps,
                r#"SELECT count(*) FROM "Message" WHERE "sourceId" = 'm-1' AND "senderType" = 'contact' AND "messageType" = 'incoming'"#
            )
            .await,
            1
        );
        let queued: Vec<_> = jobs.0.lock().unwrap().drain(..).collect();
        assert_eq!(queued.len(), 1, "a redelivery queues nothing");
        let (queue, name, data, opts) = &queued[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::AiAgent, PROCESS_AUTOMATED_RESPONSE)
        );
        assert_eq!(opts.delay_ms, Some(2000));
        let dedup = opts.dedup.as_ref().unwrap();
        assert!(dedup.id.starts_with("automated-response:") && dedup.extend && dedup.replace);
        assert_eq!(
            opts.job_id.as_deref(),
            Some(
                format!(
                    "automated-response-{}",
                    data["data"]["messageId"].as_str().unwrap()
                )
                .as_str()
            )
        );

        crate::ai_agent::process_automated_response(&deps, &data["data"])
            .await
            .unwrap();
        let replies: Vec<_> = jobs.0.lock().unwrap().drain(..).collect();
        assert_eq!(replies.len(), 1);
        let (queue, name, data, _) = &replies[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::Chat, "sendChatMessage")
        );
        assert_eq!(data["data"]["text"], "Hi there");
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "ContactActivity" WHERE "activityType" = 'automatedResponseMatched' AND "referenceId" = 6"#).await,
            1
        );
        // The pending list is cleared: a second run sends nothing.
        crate::ai_agent::process_automated_response(
            &deps,
            &json!({
                "conversationId": queued[0].2["data"]["conversationId"],
                "contactInboxId": queued[0].2["data"]["contactInboxId"],
            }),
        )
        .await
        .unwrap();
        assert!(jobs.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_identifier_is_an_unrecoverable_failure() {
        let (deps, _) = setup().await;
        let mut data = payload();
        data["integrationIdentifier"] = json!("999");
        assert!(incoming_message(&deps, &data).await.is_err());
    }

    /// `sanitizeFlowAction`, the Skip tap, the contact hint, the value quick
    /// reply and `isThirdPartyEcho`.
    #[test]
    fn taps_and_echoes() {
        use chatbotx_core::flow_config::step_input_resolution::encode_step_input_skip_postback;
        let tap = |postback: Option<&str>, quick: Option<&str>, text: &str| Received {
            postback_action: postback.map(str::to_owned),
            quick_reply_action: quick.map(str::to_owned),
            message: Some(IncomingMessage {
                text: Some(text.to_owned()),
                ..IncomingMessage::incoming(None)
            }),
            ..Received::default()
        };
        let mut r = tap(Some("1161247330962636"), Some("junk"), "hi");
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(
            (r.postback_action.as_deref(), r.quick_reply_action),
            (Some("1161247330962636"), None)
        );
        let mut r = tap(Some("1161247330962636"), None, "hi");
        sanitize_actions(&mut r, "telegram", false);
        assert_eq!(r.postback_action, None);

        let skip = encode_step_input_skip_postback("11612473309626368");
        let mut r = tap(Some(&skip), None, "Skip");
        assert_eq!(
            step_input_skip_challenge_id(&r).as_deref(),
            Some("11612473309626368")
        );
        sanitize_actions(&mut r, "messenger", true);
        assert_eq!(r.postback_action, None);

        let mut r = tap(
            None,
            Some("GET_USER_DATA_CONTACT_HINT|+84901234567"),
            "+849012…",
        );
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(r.quick_reply_action, None);
        assert_eq!(r.message.unwrap().text.as_deref(), Some("+84901234567"));
        let mut r = tap(None, Some("84901234567"), "84901234567");
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(r.quick_reply_action, None);

        let mut echo = tap(None, None, "hello");
        echo.echo_origin = Some("thirdParty".into());
        assert!(!is_third_party_echo(&echo));
        echo.message.as_mut().unwrap().message_type = "outgoing".into();
        assert!(is_third_party_echo(&echo));
        echo.message.as_mut().unwrap().content_attributes =
            Some(json!({ "type": "story_reply", "story": { "id": "s1" } }));
        assert!(!is_third_party_echo(&echo));
    }
}
