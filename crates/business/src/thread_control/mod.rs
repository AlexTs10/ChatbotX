//! Port of `packages/business/src/thread-control` (`constants.ts`, `errors.ts`,
//! `service.ts`) and of the registry wrapper
//! `packages/channel-registry/src/thread-control.ts`
//! (`requestConversationThreadControl`, `syncConversationThreadOwner`). The channel
//! calls are in [`channel`]. Shared by the API (`conversations.threadControl`, the
//! inbox action) and the worker (archive release, routing webhooks).
//!
//! Realtime events go to the given `Realtime` (none = not published). No contact-inbox
//! cache to bust (P3.6-D1).

pub mod channel;

use chatbotx_db::Db;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::partials::thread_control::{
    THREAD_CONTROL_SEEN_REFRESH_MS, ThreadControlInput, is_inbox_thread_control_active,
    resolve_thread_control_state, to_thread_control_timestamp, transition,
};
use chatbotx_db::repo::{self, contact_inbox as ci_repo, iso, now_ms};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use chatbotx_rt::{Envelope, EventType, Realtime};
use serde_json::{Value, json};

pub use channel::{ChannelFailure, ChannelResult, ThreadOwner};

use crate::errors::BusinessError;
use crate::tx;

/// Thread-control roles (`threadControlRoles`).
pub mod roles {
    pub use chatbotx_db::repo::partials::thread_control::THREAD_CONTROL_ROLES as ALL;

    /// `parseThreadControlRole`: a known role, else `None`.
    pub fn parse(value: Option<&str>) -> Option<&'static str> {
        let value = value?;
        ALL.iter().copied().find(|r| *r == value)
    }
}

/// Why a take / release / pass / sync failed.
#[derive(Debug)]
pub enum ThreadControlError {
    /// `ThreadControlUnsupportedError` (the channel has no thread-control handler).
    Unsupported(String),
    /// The channel's `ChannelError` (state unchanged).
    Channel(ChannelFailure),
    /// A `ChatbotXException` / repository error.
    Business(BusinessError),
}

impl From<BusinessError> for ThreadControlError {
    fn from(error: BusinessError) -> Self {
        ThreadControlError::Business(error)
    }
}

impl std::fmt::Display for ThreadControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadControlError::Unsupported(channel) => {
                write!(f, "Channel {channel} has no thread-control handler")
            }
            ThreadControlError::Channel(failure) => f.write_str(&failure.message),
            ThreadControlError::Business(error) => write!(f, "{error}"),
        }
    }
}

impl ThreadControlError {
    /// `isPermanentThreadControlFailure`.
    pub fn is_permanent(&self) -> bool {
        match self {
            ThreadControlError::Channel(f) => f.permanent,
            ThreadControlError::Unsupported(_) => true,
            ThreadControlError::Business(BusinessError::Repo(_)) => false,
            ThreadControlError::Business(_) => true,
        }
    }
}

type Result<T> = std::result::Result<T, ThreadControlError>;

/// The English copy of `mapThreadControlError`'s translations.
pub struct ErrorCopy {
    pub unsupported: &'static str,
    pub action_failed: &'static str,
}

/// `THREAD_CONTROL_ERROR_COPY` of the public API.
pub const PUBLIC_COPY: ErrorCopy = ErrorCopy {
    unsupported: "This channel does not support thread control.",
    action_failed: "The channel could not apply the thread-control action.",
};

/// `conversationRouting.errors.*` of `messages/en.json` (the inbox action).
pub const INBOX_COPY: ErrorCopy = ErrorCopy {
    unsupported: "Conversation routing is not available on this channel",
    action_failed: "Couldn't update conversation routing. Please try again.",
};

/// `THREAD_CONTROL_NOT_ESCALATION`.
pub const NOT_ESCALATION: &str = "notEscalation";

/// `mapThreadControlError` (English copy of the public API and the inbox action): a
/// refused `take` is `Ok(notEscalation)`; an unsupported channel / channel failure a 400
/// exception; anything else is returned unchanged.
pub fn map_error(
    error: ThreadControlError,
    action: &str,
    copy: &ErrorCopy,
) -> std::result::Result<&'static str, BusinessError> {
    match error {
        ThreadControlError::Unsupported(_) => Err(BusinessError::exception(
            copy.unsupported,
            "threadControlUnsupported",
            400,
        )),
        ThreadControlError::Business(error) => Err(error),
        ThreadControlError::Channel(failure) if action == "take" && failure.take_refused => {
            Ok(NOT_ESCALATION)
        }
        ThreadControlError::Channel(failure) => {
            let fallback = copy.action_failed;
            let message =
                if failure.message.is_empty() || crate::errors::is_query_dump(&failure.message) {
                    fallback.to_owned()
                } else {
                    Some(crate::errors::sanitize_public_text(&failure.message))
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| fallback.to_owned())
                };
            Err(BusinessError::exception(
                message,
                "threadControlFailed",
                400,
            ))
        }
    }
}

// ---- constants ------------------------------------------------------------

pub const ACTIVITY_TYPE: &str = "threadControl";
pub const CONTEXT_TYPE: &str = "threadControlContext";
pub const SOURCE_ID_PREFIX: &str = "thread-control";
pub const CONTEXT_SOURCE_ID_PREFIX: &str = "thread-control-context";
pub const DELIVERY_KEY: &str = "threadControlDelivery";
pub const STANDBY_DELIVERY: &str = "standby";
pub const PROMOTED_KEY: &str = "threadControlPromoted";

/// `isRoutingTraffic`.
pub fn is_routing_traffic(event: &str, has_context: bool) -> bool {
    !matches!(event, "inboundReceived" | "serviceSent") || has_context
}

/// `resolveThreadControlActivityText`.
pub fn activity_text(event: &str, channel: &str) -> &'static str {
    if channel == "whatsapp" && event == "passed" {
        return "Conversation passed to the escalation partner";
    }
    match event {
        "standbyReceived" => "Another app is now handling this conversation",
        "controlPassed" => "Conversation handed to this app",
        "controlTaken" => "Another app took over this conversation",
        "taken" => "This app took over this conversation",
        "released" => "Conversation released",
        "passed" => "Conversation passed to another app",
        "serviceRejected" => "Message not sent: another app is handling this conversation",
        // inboundReceived, serviceSent
        _ => "This app is now handling this conversation",
    }
}

/// Own action -> its recorded event (`ACTION_EVENTS`).
pub fn action_event(action: &str) -> Option<&'static str> {
    match action {
        "take" => Some("taken"),
        "release" => Some("released"),
        "pass" => Some("passed"),
        _ => None,
    }
}

// ---- snapshot -------------------------------------------------------------

/// `ThreadControlSnapshot` (the public / realtime JSON).
pub fn snapshot_json(row: &ContactInbox) -> Value {
    json!({
        "contactInboxId": row.id.to_string(),
        "threadControlState": row.thread_control_state,
        "threadOwnerRole": row.thread_owner_role,
        "threadOwnerAppId": row.thread_owner_app_id,
        "threadControlUpdatedAt": row.thread_control_updated_at.map(iso),
        "threadOwnerExpiresAt": row.thread_owner_expires_at.map(iso),
        "threadControlLastEvent": row.thread_control_last_event,
    })
}

fn stored_state(row: &ContactInbox, now: i64) -> Option<String> {
    resolve_thread_control_state(&ThreadControlInput {
        state: row.thread_control_state.as_deref(),
        last_incoming_message_at: row.last_incoming_message_at,
        thread_control_updated_at: row.thread_control_updated_at,
        thread_owner_expires_at: row.thread_owner_expires_at,
        now,
    })
    .map(str::to_owned)
}

// ---- recordEvent ----------------------------------------------------------

/// `RecordThreadControlEventInput`.
#[derive(Debug, Clone, Default)]
pub struct RecordEventInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    /// `inbox.threadControlSeenAt` (`None` = never loaded).
    pub inbox_seen_at: Option<i64>,
    pub contact_inbox: Option<ContactInbox>,
    pub conversation_id: i64,
    pub event: String,
    pub owner_role: Option<String>,
    pub previous_owner_role: Option<String>,
    pub owner_app_id: Option<String>,
    pub previous_owner_app_id: Option<String>,
    /// `None` = absent (derived), `Some(None)` = clear.
    pub thread_owner_expires_at: Option<Option<i64>>,
    pub occurred_at: i64,
    pub supersedes_standby_at: Option<i64>,
    pub context: Option<Value>,
    pub handover_note: Option<String>,
}

impl RecordEventInput {
    fn ci(&self) -> &ContactInbox {
        self.contact_inbox
            .as_ref()
            .expect("RecordEventInput.contact_inbox is required")
    }
}

/// `RecordThreadControlEventResult`.
#[derive(Debug, Clone)]
pub struct RecordEventResult {
    pub event_applied: bool,
    pub state_changed: bool,
    pub is_redelivery: bool,
    pub row: Option<ContactInbox>,
}

fn is_same_event(row: &ContactInbox, event: &str, occurred_at: i64) -> bool {
    row.thread_control_last_event.as_deref() == Some(event)
        && row.thread_control_updated_at == Some(occurred_at)
}

/// `asOriginalOfRepeatedAction`: `Some(original time)` for a repeat of the stored
/// own-action transition.
fn original_of_repeated_action(
    input: &RecordEventInput,
    next_state: &str,
    owner_role: Option<&str>,
) -> Option<i64> {
    let row = input.ci();
    if !matches!(input.event.as_str(), "taken" | "released" | "passed") || input.context.is_some() {
        return None;
    }
    let same = row.thread_control_last_event.as_deref() == Some(input.event.as_str())
        && stored_state(row, now_ms()).as_deref() == Some(next_state)
        && row.thread_owner_role.as_deref() == owner_role
        && row.thread_owner_app_id == input.owner_app_id;
    if same {
        row.thread_control_updated_at
    } else {
        None
    }
}

/// `resolvePreviousOwnerAppId`.
fn previous_owner_app_id(input: &RecordEventInput) -> Option<String> {
    if let Some(prev) = input
        .previous_owner_app_id
        .as_ref()
        .filter(|p| !p.is_empty())
    {
        return Some(prev.clone());
    }
    if !matches!(input.event.as_str(), "controlPassed" | "controlTaken") {
        return None;
    }
    let row = input.ci();
    if is_same_event(row, &input.event, input.occurred_at) {
        row.thread_previous_owner_app_id.clone()
    } else {
        row.thread_owner_app_id.clone()
    }
}

/// `deriveThreadOwnerExpiresAt`.
fn derive_expires(input: &RecordEventInput, next_state: &str) -> Option<Option<i64>> {
    if input.thread_owner_expires_at.is_some() {
        return input.thread_owner_expires_at;
    }
    if next_state != "standby" {
        return Some(None);
    }
    let row = input.ci();
    let same_session = stored_state(row, input.occurred_at).as_deref() == Some("standby")
        && row.thread_owner_app_id == input.owner_app_id;
    if same_session { None } else { Some(None) }
}

async fn write_transition(
    db: &Db,
    input: &RecordEventInput,
    owner_role: Option<&str>,
    expires: Option<Option<i64>>,
) -> Result<Option<ContactInbox>> {
    let row = input.ci();
    let (id, ws) = (row.id, input.workspace_id);
    let normal = ci_repo::ThreadControlTransition {
        id,
        workspace_id: ws,
        event: input.event.clone(),
        owner_role: owner_role.map(str::to_owned),
        owner_app_id: input.owner_app_id.clone(),
        previous_owner_app_id: previous_owner_app_id(input),
        thread_owner_expires_at: expires,
        occurred_at: input.occurred_at,
    };
    let promote = input.supersedes_standby_at;
    let promote_prev = input.previous_owner_app_id.clone();
    Ok(tx::write(db, move |conn| {
        if let Some(at) = promote
            && let Some(row) = ci_repo::promote_standby_to_owner_delivery(
                conn,
                normal.id,
                normal.workspace_id,
                normal.owner_role.as_deref(),
                promote_prev.as_deref(),
                normal.thread_owner_expires_at,
                at,
            )?
        {
            return Ok(Some(row));
        }
        Ok(ci_repo::apply_thread_control_transition(conn, &normal)?)
    })
    .await?)
}

/// `recordEvent`: the guarded write, the divider (state change or redelivery), the
/// realtime snapshot and the optional context card.
pub async fn record_event(
    db: &Db,
    rt: Option<&Realtime>,
    requested: RecordEventInput,
) -> Result<RecordEventResult> {
    let owner_role = roles::parse(requested.owner_role.as_deref());
    let Some(next_state) = transition(&requested.event) else {
        return Err(BusinessError::exception(
            format!("unknown thread control event {}", requested.event),
            "systemError",
            500,
        )
        .into());
    };
    let expires = derive_expires(&requested, next_state);
    // `applyTransition`: a repeated own action first at its original time.
    let (input, row, remap_stale) =
        match original_of_repeated_action(&requested, next_state, owner_role) {
            Some(at) if at != requested.occurred_at => {
                let remapped = RecordEventInput {
                    occurred_at: at,
                    ..requested.clone()
                };
                match write_transition(db, &remapped, owner_role, expires).await? {
                    Some(row) => (remapped, Some(row), false),
                    None => {
                        let row = write_transition(db, &requested, owner_role, expires).await?;
                        let stale = row.is_some();
                        (requested, row, stale)
                    }
                }
            }
            _ => {
                let row = write_transition(db, &requested, owner_role, expires).await?;
                (requested, row, false)
            }
        };

    if is_routing_traffic(&input.event, input.context.is_some()) {
        touch_inbox_seen(db, &input).await?;
    }
    let Some(row) = row else {
        return Ok(RecordEventResult {
            event_applied: false,
            state_changed: false,
            is_redelivery: false,
            row: None,
        });
    };
    let before = input.ci();
    let state_changed = remap_stale
        || stored_state(before, now_ms()).as_deref() != Some(next_state)
        || before.thread_owner_role.as_deref() != owner_role
        || before.thread_owner_app_id != input.owner_app_id;
    let is_redelivery = is_same_event(before, &input.event, input.occurred_at);

    let divider = if state_changed || is_redelivery {
        write_message(db, rt, &input, false, owner_role, is_redelivery).await
    } else {
        Ok(false)
    };
    announce(rt, input.workspace_id, input.conversation_id, &row).await;
    divider?;
    if input.context.is_some() || input.handover_note.is_some() {
        write_message(db, rt, &input, true, owner_role, is_redelivery).await?;
    }
    Ok(RecordEventResult {
        event_applied: true,
        state_changed,
        is_redelivery,
        row: Some(row),
    })
}

async fn touch_inbox_seen(db: &Db, input: &RecordEventInput) -> Result<()> {
    let now = now_ms();
    if input
        .inbox_seen_at
        .is_some_and(|seen| now - seen < THREAD_CONTROL_SEEN_REFRESH_MS)
    {
        return Ok(());
    }
    let (ws, inbox) = (input.workspace_id, input.inbox_id);
    tx::write(db, move |conn| {
        Ok(repo::inbox::touch_thread_control_seen(
            conn, ws, inbox, now,
        )?)
    })
    .await?;
    Ok(())
}

/// `announceStateChange`: `contactInboxThreadControlUpdated`.
async fn announce(
    rt: Option<&Realtime>,
    workspace_id: i64,
    conversation_id: i64,
    row: &ContactInbox,
) {
    let Some(rt) = rt else { return };
    let mut data = snapshot_json(row);
    data["conversationId"] = json!(conversation_id.to_string());
    rt.queue_workspace_event(
        &workspace_id.to_string(),
        Envelope::new(EventType::ContactInboxThreadControlUpdated, data),
    )
    .await;
}

/// `writeMessage`: one routing message (divider or context card); `true` when new.
async fn write_message(
    db: &Db,
    rt: Option<&Realtime>,
    input: &RecordEventInput,
    is_context: bool,
    owner_role: Option<&str>,
    is_redelivery: bool,
) -> Result<bool> {
    let row = input.ci();
    let (prefix, text, attributes) = if is_context {
        let mut attrs = json!({ "type": CONTEXT_TYPE });
        if let Some(context) = &input.context {
            attrs["context"] = context.clone();
        }
        if let Some(note) = &input.handover_note {
            attrs["handoverNote"] = json!(note);
        }
        (CONTEXT_SOURCE_ID_PREFIX, None, attrs)
    } else {
        let stored_previous = if is_redelivery {
            None
        } else {
            roles::parse(row.thread_owner_role.as_deref())
        };
        let previous_role = roles::parse(input.previous_owner_role.as_deref()).or(stored_previous);
        let previous_app = input.previous_owner_app_id.clone().or_else(|| {
            if is_redelivery {
                None
            } else {
                row.thread_owner_app_id.clone()
            }
        });
        let mut attrs =
            json!({ "type": ACTIVITY_TYPE, "event": input.event, "ownerRole": owner_role });
        if let Some(role) = previous_role {
            attrs["previousOwnerRole"] = json!(role);
        }
        if let Some(app) = previous_app.filter(|a| !a.is_empty()) {
            attrs["previousOwnerAppId"] = json!(app);
        }
        (
            SOURCE_ID_PREFIX,
            Some(activity_text(&input.event, &row.channel).to_owned()),
            attrs,
        )
    };
    let message = repo::message::CreateMessageInput {
        id: None,
        contact_inbox_id: row.id,
        conversation_id: input.conversation_id,
        workspace_id: input.workspace_id,
        content_attributes: Some(attributes),
        content_type: "text".into(),
        message_type: "activity".into(),
        sender_type: "system".into(),
        sender_id: None,
        source_id: Some(format!(
            "{prefix}:{}:{}:{}",
            row.id, input.event, input.occurred_at
        )),
        parent_id: None,
        text,
        r#type: None,
        created_at: Some(input.occurred_at),
        updated_at: None,
    };
    let (ws, conversation_id, ci_id, at) = (
        input.workspace_id,
        input.conversation_id,
        row.id,
        input.occurred_at,
    );
    let first = row.first_interaction_at.unwrap_or(at);
    let written = tx::write(db, move |conn| {
        let written = repo::message::create_or_update(conn, &message)?;
        if written.is_new {
            // `bumpLastMessageAt`: GREATEST, never backwards.
            conn.execute(
                "UPDATE \"ContactInbox\" SET \"lastMessageAt\" = max(coalesce(\"lastMessageAt\", 0), ?2), \
                 \"firstInteractionAt\" = coalesce(\"firstInteractionAt\", ?3) WHERE \"id\" = ?1",
                [ci_id, at, first],
            )
            .map_err(repo::Error::from)?;
        }
        let conversation = crate::conversation_list::find_in(conn, ws, conversation_id)?;
        Ok((written, conversation))
    })
    .await?;
    let (written, conversation) = written;
    if !written.is_new {
        return Ok(false);
    }
    if let Some(rt) = rt {
        publish_message_created(rt, conversation.as_ref(), row.inbox_id, &written.message).await;
    }
    Ok(true)
}

async fn publish_message_created(
    rt: &Realtime,
    conversation: Option<&Conversation>,
    inbox_id: i64,
    message: &repo::models::Message,
) {
    let Some(conversation) = conversation else {
        return;
    };
    let payload = crate::message::message_json(&repo::message::MessageWithAttachments {
        message: message.clone(),
        attachment_count: None,
        attachments: Vec::new(),
    });
    let route = chatbotx_rt::protocol::route_for_conversation(
        Some(&chatbotx_rt::protocol::ConversationRoute {
            assigned_inbox_team_id: conversation.assigned_inbox_team_id.map(|v| v.to_string()),
            assigned_user_id: conversation.assigned_user_id.map(|v| v.to_string()),
            inbox_id: Some(inbox_id.to_string()),
        }),
        None,
    );
    rt.queue_workspace_event(
        &conversation.workspace_id.to_string(),
        Envelope::routed(EventType::MessageCreated, payload, route),
    )
    .await;
}

// ---- requestAction / syncThreadOwner ----------------------------------------

async fn find_contact_inbox(db: &Db, workspace_id: i64, id: i64) -> Result<Option<ContactInbox>> {
    Ok(tx::read(db, move |conn| {
        Ok(ci_repo::find_model_by_id_for_workspace(
            conn,
            id,
            workspace_id,
        )?)
    })
    .await?)
}

/// One take / release / pass request.
#[derive(Debug, Clone)]
pub struct ActionRequest<'a> {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub action: &'a str,
    pub target_role: Option<&'a str>,
    pub metadata: Option<&'a str>,
    /// `expectedThreadControlUpdatedAt` (`None` = not checked).
    pub expected_updated_at: Option<Option<i64>>,
}

/// `requestThreadControlAction` (`threadControlService.requestAction` + `runOnChannel`):
/// the channel call first (a failure leaves the state untouched), then the event.
pub async fn request_action(
    db: &Db,
    rt: Option<&Realtime>,
    base_url: &str,
    request: ActionRequest<'_>,
) -> Result<ContactInbox> {
    let ws = request.workspace_id;
    let Some(contact_inbox) = find_contact_inbox(db, ws, request.contact_inbox_id).await? else {
        return Err(BusinessError::not_found("Contact inbox not found").into());
    };
    if let Some(expected) = request.expected_updated_at
        && contact_inbox.thread_control_updated_at != expected
    {
        return Ok(contact_inbox);
    }
    let Some(event) = action_event(request.action) else {
        return Err(BusinessError::validation("action", "Invalid action").into());
    };
    let occurred_at = to_thread_control_timestamp(now_ms());
    let result = channel::update_on_channel(
        db,
        base_url,
        ws,
        &contact_inbox,
        request.action,
        request.target_role,
        request.metadata,
    )
    .await?;
    let recorded = record_event(
        db,
        rt,
        RecordEventInput {
            workspace_id: ws,
            inbox_id: contact_inbox.inbox_id,
            inbox_seen_at: None,
            conversation_id: request.conversation_id,
            event: event.into(),
            owner_role: result.owner_role.map(str::to_owned),
            owner_app_id: result.owner_app_id,
            previous_owner_app_id: contact_inbox.thread_owner_app_id.clone(),
            occurred_at,
            contact_inbox: Some(contact_inbox.clone()),
            ..RecordEventInput::default()
        },
    )
    .await?;
    if let Some(row) = recorded.row {
        return Ok(row);
    }
    Ok(find_contact_inbox(db, ws, request.contact_inbox_id)
        .await?
        .unwrap_or(contact_inbox))
}

/// `classifyThreadOwner`.
pub fn classify_owner(
    owner_app_id: Option<&str>,
    own_app_id: Option<&str>,
    ai_agent_app_id: Option<&str>,
) -> &'static str {
    let Some(owner) = owner_app_id else {
        return "none";
    };
    let Some(own) = own_app_id else {
        return "unknown";
    };
    if owner == own {
        "self"
    } else if Some(owner) == ai_agent_app_id {
        "aiHandover"
    } else {
        "partner"
    }
}

/// `syncThreadOwner`: reconciles the stored owner with the channel's answer.
pub async fn sync_thread_owner(
    db: &Db,
    rt: Option<&Realtime>,
    base_url: &str,
    workspace_id: i64,
    contact_inbox: ContactInbox,
    conversation_id: i64,
) -> Result<ContactInbox> {
    let occurred_at = to_thread_control_timestamp(now_ms());
    let Some(owner) = channel::get_thread_owner(db, base_url, workspace_id, &contact_inbox).await?
    else {
        return Ok(contact_inbox);
    };
    let kind = classify_owner(
        owner.owner_app_id.as_deref(),
        owner.own_app_id.as_deref(),
        owner.ai_agent_app_id.as_deref(),
    );
    let resolved = stored_state(&contact_inbox, occurred_at);
    let stored_app = contact_inbox.thread_owner_app_id.clone();
    // `OWNER_SYNC_PLANS`: (event, owner role, keeps the owner app id).
    let (event, owner_role, keeps_app) = match kind {
        "self" => ("inboundReceived", None, false),
        "aiHandover" => ("standbyReceived", Some("ai_agent"), true),
        "partner" => ("standbyReceived", None, true),
        "none" => ("released", None, false),
        _ => return Ok(contact_inbox),
    };
    let owner_app_id = if keeps_app {
        owner.owner_app_id.clone()
    } else {
        None
    };
    let is_standby = event == "standbyReceived";
    let confirmed = match kind {
        "self" => resolved.as_deref() == Some("owned"),
        "none" => matches!(resolved.as_deref(), Some("idle") | None),
        _ => resolved.as_deref() == Some("standby") && stored_app == owner_app_id,
    };
    if confirmed {
        if !is_standby || owner.expires_at == contact_inbox.thread_owner_expires_at {
            return Ok(contact_inbox);
        }
        // `refineStandbyExpiry`.
        let (id, observed, app, expires) = (
            contact_inbox.id,
            contact_inbox.thread_control_updated_at,
            owner_app_id.clone(),
            owner.expires_at,
        );
        let row = tx::write(db, move |conn| {
            Ok(ci_repo::set_standby_thread_owner_expires_at(
                conn,
                id,
                workspace_id,
                app.as_deref(),
                observed,
                expires,
            )?)
        })
        .await?;
        let Some(row) = row else {
            return Ok(contact_inbox);
        };
        announce(rt, workspace_id, conversation_id, &row).await;
        return Ok(row);
    }
    let recorded = record_event(
        db,
        rt,
        RecordEventInput {
            workspace_id,
            inbox_id: contact_inbox.inbox_id,
            conversation_id,
            event: event.into(),
            owner_role: owner_role.map(str::to_owned),
            owner_app_id,
            previous_owner_app_id: stored_app,
            thread_owner_expires_at: is_standby.then_some(owner.expires_at),
            occurred_at,
            contact_inbox: Some(contact_inbox.clone()),
            ..RecordEventInput::default()
        },
    )
    .await?;
    Ok(recorded.row.unwrap_or(contact_inbox))
}

const CONTACT_INBOX_NOT_FOUND: &str = "Contact inbox not found for this conversation";

async fn conversation_contact_inbox(
    db: &Db,
    workspace_id: i64,
    conversation: &Conversation,
    contact_inbox_id: i64,
) -> Result<ContactInbox> {
    find_contact_inbox(db, workspace_id, contact_inbox_id)
        .await?
        .filter(|row| row.contact_id == conversation.contact_id)
        .ok_or_else(|| BusinessError::not_found(CONTACT_INBOX_NOT_FOUND).into())
}

/// `requestConversationThreadControl`: refuses a contact inbox of another contact (404).
#[allow(clippy::too_many_arguments)]
pub async fn request_conversation_action(
    db: &Db,
    rt: Option<&Realtime>,
    base_url: &str,
    conversation: &Conversation,
    contact_inbox_id: i64,
    action: &str,
    target_role: Option<&str>,
    metadata: Option<&str>,
) -> Result<ContactInbox> {
    let ws = conversation.workspace_id;
    conversation_contact_inbox(db, ws, conversation, contact_inbox_id).await?;
    request_action(
        db,
        rt,
        base_url,
        ActionRequest {
            workspace_id: ws,
            contact_inbox_id,
            conversation_id: conversation.id,
            action,
            target_role,
            metadata,
            expected_updated_at: None,
        },
    )
    .await
}

/// `syncConversationThreadOwner`.
pub async fn sync_conversation_owner(
    db: &Db,
    rt: Option<&Realtime>,
    base_url: &str,
    conversation: &Conversation,
    contact_inbox_id: i64,
) -> Result<ContactInbox> {
    let ws = conversation.workspace_id;
    let row = conversation_contact_inbox(db, ws, conversation, contact_inbox_id).await?;
    sync_thread_owner(db, rt, base_url, ws, row, conversation.id).await
}

// ---- inbound deliveries -----------------------------------------------------

/// `recordInboundDelivery`: `None` when nothing is written (see the TS strategy map).
pub async fn record_inbound_delivery(
    db: &Db,
    rt: Option<&Realtime>,
    mut input: RecordEventInput,
    delivery: &str,
) -> Result<Option<RecordEventResult>> {
    let now = now_ms();
    let row = input.ci();
    let resolved = stored_state(row, now);
    let record = match delivery {
        "owner" => {
            resolved.as_deref() != Some("owned")
                && (resolved.is_some() || is_inbox_thread_control_active(input.inbox_seen_at, now))
        }
        _ => {
            resolved.as_deref() != Some("standby")
                || input
                    .owner_app_id
                    .as_ref()
                    .is_some_and(|app| Some(app) != row.thread_owner_app_id.as_ref())
        }
    } || input.context.is_some();
    if !record {
        return Ok(None);
    }
    let standby = delivery == STANDBY_DELIVERY;
    input.event = if standby {
        "standbyReceived"
    } else {
        "inboundReceived"
    }
    .into();
    if !standby {
        input.owner_app_id = None;
        input.owner_role = None;
    }
    input.previous_owner_role = None;
    input.previous_owner_app_id = None;
    input.handover_note = None;
    Ok(Some(record_event(db, rt, input).await?))
}

/// `promoteStandbyDelivery`: claims the standby copy once its owner delivery arrives;
/// `true` for exactly one of concurrent claims.
pub async fn promote_standby_delivery(
    db: &Db,
    workspace_id: i64,
    message: &repo::models::Message,
) -> std::result::Result<bool, BusinessError> {
    if !is_unpromoted_standby_copy(message.content_attributes.as_ref().map(|j| &j.0)) {
        return Ok(false);
    }
    let mut overlay = serde_json::Map::new();
    overlay.insert(PROMOTED_KEY.into(), Value::Bool(true));
    let params = repo::message::ClaimContentAttributesParams {
        message_id: message.id,
        workspace_id,
        created_at: message.created_at,
        guard_key: PROMOTED_KEY.into(),
        overlay,
    };
    let claimed = tx::write(db, move |conn| {
        Ok(repo::message::claim_content_attributes(conn, &params)?)
    })
    .await?;
    Ok(claimed.is_some())
}

/// `refreshForRouting`: re-reads a thread that has observed routing (no query for a
/// never-routed one).
pub async fn refresh_for_routing(
    db: &Db,
    workspace_id: i64,
    contact_inbox: ContactInbox,
) -> std::result::Result<ContactInbox, BusinessError> {
    if contact_inbox.thread_control_state.is_none() {
        return Ok(contact_inbox);
    }
    let id = contact_inbox.id;
    let current = tx::read(db, move |conn| {
        Ok(ci_repo::find_model_by_id_for_workspace(
            conn,
            id,
            workspace_id,
        )?)
    })
    .await?;
    Ok(current.unwrap_or(contact_inbox))
}

/// `isUnpromotedStandbyCopy`.
pub fn is_unpromoted_standby_copy(attributes: Option<&Value>) -> bool {
    attributes.is_some_and(|a| {
        a.get(DELIVERY_KEY).and_then(Value::as_str) == Some(STANDBY_DELIVERY)
            && a.get(PROMOTED_KEY).is_none()
    })
}

// ---- archive release ----------------------------------------------------------

/// `IntegrationJobAction.threadControlAction`.
pub const RELEASE_JOB: &str = "threadControlAction";

/// `supportsArchiveRelease(channel)`.
pub fn supports_archive_release(channel: &str) -> bool {
    channel == "whatsapp"
}

/// `releaseOwnedThreadsForContacts`: one deduplicated release job per thread still
/// owned on a channel whose archive releases. No channel call here.
pub async fn release_owned_threads_for_contacts(
    db: &Db,
    jobs: &dyn Enqueuer,
    workspace_id: i64,
    conversations: &[(i64, i64)],
    archived_at: i64,
) -> std::result::Result<(), BusinessError> {
    let mut by_contact: Vec<(i64, i64)> = Vec::new();
    for &(conversation_id, contact_id) in conversations {
        if !by_contact.iter().any(|(c, _)| *c == contact_id) {
            by_contact.push((contact_id, conversation_id));
        }
    }
    if by_contact.is_empty() {
        return Ok(());
    }
    let contact_ids: Vec<i64> = by_contact.iter().map(|(c, _)| *c).collect();
    let rows = tx::read(db, move |conn| {
        Ok(ci_repo::list_thread_controlled_by_contact_ids(
            conn,
            workspace_id,
            &contact_ids,
        )?)
    })
    .await?;
    let now = now_ms();
    for row in rows {
        let state = resolve_thread_control_state(&ThreadControlInput {
            state: row.thread_control_state.as_deref(),
            last_incoming_message_at: row.last_incoming_message_at,
            thread_control_updated_at: row.thread_control_updated_at,
            thread_owner_expires_at: row.thread_owner_expires_at,
            now,
        });
        let conversation = by_contact
            .iter()
            .find(|(c, _)| *c == row.contact_id)
            .map(|(_, conv)| *conv);
        let (Some(conversation_id), Some("owned")) = (conversation, state) else {
            continue;
        };
        if !supports_archive_release(&row.channel) {
            continue;
        }
        let version = row.thread_control_updated_at.unwrap_or(0);
        let data = json!({
            "type": RELEASE_JOB,
            "data": {
                "workspaceId": workspace_id.to_string(),
                "contactInboxId": row.id.to_string(),
                "conversationId": conversation_id.to_string(),
                "action": "release",
                "threadControlUpdatedAt": row.thread_control_updated_at.map(iso),
            },
        });
        let opts = JobOptions {
            job_id: Some(format!("thread-release-{}-{version}-{archived_at}", row.id)),
            remove_on_fail: Some(chatbotx_jobs::Retention::Remove),
            ..JobOptions::default()
        };
        if let Err(error) = jobs
            .add(QueueName::Integration, RELEASE_JOB, data, opts)
            .await
        {
            return Err(BusinessError::exception(
                format!("thread release enqueue failed: {error}"),
                "systemError",
                500,
            ));
        }
    }
    Ok(())
}

/// `resolveCurrentState`: (state, threadControlUpdatedAt) as they are now.
pub async fn resolve_current_state(
    db: &Db,
    workspace_id: i64,
    contact_inbox_id: i64,
) -> Result<Option<(String, Option<i64>)>> {
    let row = find_contact_inbox(db, workspace_id, contact_inbox_id).await?;
    Ok(row.and_then(|row| {
        stored_state(&row, now_ms()).map(|state| (state, row.thread_control_updated_at))
    }))
}

/// The service futures run inside axum handlers and jobs, so they must be `Send`.
#[cfg(test)]
#[allow(dead_code)]
mod send_checks {
    use super::*;

    fn assert_send<T: Send>(_: T) {}

    fn service_futures_are_send(
        db: &Db,
        rt: Option<&Realtime>,
        conversation: &Conversation,
        row: ContactInbox,
        jobs: &dyn Enqueuer,
    ) {
        assert_send(request_conversation_action(
            db,
            rt,
            "",
            conversation,
            0,
            "take",
            None,
            None,
        ));
        assert_send(sync_conversation_owner(db, rt, "", conversation, 0));
        assert_send(record_event(db, rt, RecordEventInput::default()));
        assert_send(release_owned_threads_for_contacts(db, jobs, 0, &[], 0));
        assert_send(refresh_for_routing(db, 0, row));
    }
}
