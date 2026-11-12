//! Wire protocol (spec 1.3-1.5, 3.2): event types, envelopes, stream
//! records, the `<ms>-<n>` sequence ids, close codes and the presence
//! contract. Same names and JSON shapes as `packages/realtime-protocol`.

use std::fmt;
use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `RealtimeEventType` (19 types, `schemas.ts:3-23`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    MessageCreated,
    MessageDeleted,
    MessageUpdated,
    MessageContentUpdated,
    MessageIdAssigned,
    MessageFailed,
    Typing,
    ContactBlocked,
    ContactUnblocked,
    ConversationAssigned,
    ConversationUpdated,
    WhatsappCallTransportIncoming,
    WhatsappCallTransportEnded,
    WhatsappCallClaimedElsewhere,
    WhatsappCallOutboundAnswer,
    WhatsappCallOutboundStatus,
    WhatsappCallPermissionUpdated,
    ContactInboxThreadControlUpdated,
    PipelineItemChanged,
}

impl EventType {
    /// Every type, in `EVENT_DELIVERY` order.
    pub const ALL: [EventType; 19] = [
        EventType::MessageCreated,
        EventType::MessageDeleted,
        EventType::MessageUpdated,
        EventType::MessageContentUpdated,
        EventType::MessageIdAssigned,
        EventType::MessageFailed,
        EventType::Typing,
        EventType::ContactBlocked,
        EventType::ContactUnblocked,
        EventType::ConversationAssigned,
        EventType::ConversationUpdated,
        EventType::WhatsappCallTransportIncoming,
        EventType::WhatsappCallTransportEnded,
        EventType::WhatsappCallClaimedElsewhere,
        EventType::WhatsappCallOutboundAnswer,
        EventType::WhatsappCallOutboundStatus,
        EventType::WhatsappCallPermissionUpdated,
        EventType::ContactInboxThreadControlUpdated,
        EventType::PipelineItemChanged,
    ];

    /// The wire name (`"messageCreated"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            EventType::MessageCreated => "messageCreated",
            EventType::MessageDeleted => "messageDeleted",
            EventType::MessageUpdated => "messageUpdated",
            EventType::MessageContentUpdated => "messageContentUpdated",
            EventType::MessageIdAssigned => "messageIdAssigned",
            EventType::MessageFailed => "messageFailed",
            EventType::Typing => "typing",
            EventType::ContactBlocked => "contactBlocked",
            EventType::ContactUnblocked => "contactUnblocked",
            EventType::ConversationAssigned => "conversationAssigned",
            EventType::ConversationUpdated => "conversationUpdated",
            EventType::WhatsappCallTransportIncoming => "whatsappCallTransportIncoming",
            EventType::WhatsappCallTransportEnded => "whatsappCallTransportEnded",
            EventType::WhatsappCallClaimedElsewhere => "whatsappCallClaimedElsewhere",
            EventType::WhatsappCallOutboundAnswer => "whatsappCallOutboundAnswer",
            EventType::WhatsappCallOutboundStatus => "whatsappCallOutboundStatus",
            EventType::WhatsappCallPermissionUpdated => "whatsappCallPermissionUpdated",
            EventType::ContactInboxThreadControlUpdated => "contactInboxThreadControlUpdated",
            EventType::PipelineItemChanged => "pipelineItemChanged",
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `RealtimeEventDelivery`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delivery {
    Conversation,
    Workspace,
    Targeted,
}

/// `EVENT_DELIVERY` (`schemas.ts:30-50`).
pub const fn delivery(event_type: EventType) -> Delivery {
    match event_type {
        EventType::MessageCreated
        | EventType::MessageDeleted
        | EventType::MessageUpdated
        | EventType::MessageContentUpdated
        | EventType::MessageIdAssigned
        | EventType::MessageFailed
        | EventType::ConversationAssigned
        | EventType::ConversationUpdated
        | EventType::WhatsappCallPermissionUpdated => Delivery::Conversation,
        EventType::Typing
        | EventType::ContactBlocked
        | EventType::ContactUnblocked
        | EventType::WhatsappCallTransportEnded
        | EventType::WhatsappCallClaimedElsewhere
        | EventType::ContactInboxThreadControlUpdated
        | EventType::PipelineItemChanged => Delivery::Workspace,
        EventType::WhatsappCallTransportIncoming
        | EventType::WhatsappCallOutboundAnswer
        | EventType::WhatsappCallOutboundStatus => Delivery::Targeted,
    }
}

/// `IMMEDIATE_FLUSH_EVENT_TYPES` (`realtime-broadcast.ts:42-49`).
pub const fn flushes_immediately(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::ConversationAssigned
            | EventType::WhatsappCallClaimedElsewhere
            | EventType::WhatsappCallPermissionUpdated
            | EventType::WhatsappCallTransportEnded
    )
}

/// `guestRealtimeEventTypes`: the 6 message types and `typing`.
pub const fn guest_allowed(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageCreated
            | EventType::MessageDeleted
            | EventType::MessageUpdated
            | EventType::MessageContentUpdated
            | EventType::MessageIdAssigned
            | EventType::MessageFailed
            | EventType::Typing
    )
}

/// `RealtimeEventRoute`. Field order = the TS stable-stringify order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    #[serde(default)]
    pub assigned_team_ids: Vec<String>,
    pub assigned_user_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_id: Option<String>,
}

/// `RealtimeEventEnvelope`: `{data, eventType, route?}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    #[serde(default)]
    pub data: Value,
    pub event_type: EventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
}

impl Envelope {
    /// An envelope without a route.
    pub fn new(event_type: EventType, data: Value) -> Envelope {
        Envelope {
            data,
            event_type,
            route: None,
        }
    }

    /// An envelope with a route.
    pub fn routed(event_type: EventType, data: Value, route: Route) -> Envelope {
        Envelope {
            data,
            event_type,
            route: Some(route),
        }
    }
}

/// A rejected envelope, frame, record or payload. The message carries the
/// same texts as the zod refinements (`"<type>" is conversation-scoped and
/// requires a route`, `"<type>" is targeted-only and cannot be
/// workspace-broadcast`, `Event type is not allowed for guest delivery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub message: String,
}

impl ProtocolError {
    pub fn new(message: impl Into<String>) -> ProtocolError {
        ProtocolError {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// `realtimeWorkspaceEventEnvelopeSchema` refinements on a typed envelope.
pub fn validate_workspace_envelope(envelope: &Envelope) -> Result<(), ProtocolError> {
    match delivery(envelope.event_type) {
        Delivery::Targeted => Err(ProtocolError::new(format!(
            "\"{}\" is targeted-only and cannot be workspace-broadcast",
            envelope.event_type
        ))),
        Delivery::Conversation if envelope.route.is_none() => Err(ProtocolError::new(format!(
            "\"{}\" is conversation-scoped and requires a route",
            envelope.event_type
        ))),
        _ => Ok(()),
    }
}

/// `realtimeGuestEventEnvelopeSchema` refinement on a typed envelope.
pub fn validate_guest_envelope(envelope: &Envelope) -> Result<(), ProtocolError> {
    if guest_allowed(envelope.event_type) {
        Ok(())
    } else {
        Err(ProtocolError::new(
            "eventType: Event type is not allowed for guest delivery",
        ))
    }
}

/// `realtimeEventEnvelopeSchema.parse` (any type, route optional).
pub fn parse_envelope(value: &Value) -> Result<Envelope, ProtocolError> {
    serde_json::from_value(value.clone()).map_err(|err| ProtocolError::new(err.to_string()))
}

/// `realtimeWorkspaceEventEnvelopeSchema.parse`.
pub fn parse_workspace_envelope(value: &Value) -> Result<Envelope, ProtocolError> {
    let envelope = parse_envelope(value)?;
    validate_workspace_envelope(&envelope)?;
    Ok(envelope)
}

/// `realtimeGuestEventEnvelopeSchema.parse`.
pub fn parse_guest_envelope(value: &Value) -> Result<Envelope, ProtocolError> {
    let envelope = parse_envelope(value)?;
    validate_guest_envelope(&envelope)?;
    Ok(envelope)
}

/// A `{"batch":[...],"seq":"<ms>-<n>"}` frame.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BatchFrame {
    pub batch: Vec<Envelope>,
    pub seq: String,
}

/// `realtimeBatchEnvelopeSchema.parse` (workspace readers).
pub fn parse_workspace_batch(value: &Value) -> Result<BatchFrame, ProtocolError> {
    parse_batch(value, parse_workspace_envelope)
}

/// `realtimeMemberBatchEnvelopeSchema.parse` (targeted types allowed).
pub fn parse_member_batch(value: &Value) -> Result<BatchFrame, ProtocolError> {
    parse_batch(value, parse_envelope)
}

/// `realtimeGuestBatchEnvelopeSchema.parse`.
pub fn parse_guest_batch(value: &Value) -> Result<BatchFrame, ProtocolError> {
    parse_batch(value, parse_guest_envelope)
}

/// `{"batch":[...],"seq":"..."}` with the envelopes as given (key order
/// `batch`, `seq`).
pub fn encode_batch(events: &[&Envelope], seq: &str) -> String {
    let mut frame = String::from(r#"{"batch":["#);
    for (index, event) in events.iter().enumerate() {
        if index > 0 {
            frame.push(',');
        }
        frame.push_str(&to_json(event));
    }
    frame.push_str(r#"],"seq":"#);
    frame.push_str(&to_json(seq));
    frame.push('}');
    frame
}

/// The exact `workspace-events` record JSON of `realtime-broadcast.ts`
/// (`{"events":[..],"kind":"workspace-events","workspaceId":".."}`); its
/// byte length is the 256 KiB coalescing bound.
pub fn encode_workspace_record(workspace_id: &str, events: &[Envelope]) -> String {
    let mut record = String::from(r#"{"events":["#);
    for (index, event) in events.iter().enumerate() {
        if index > 0 {
            record.push(',');
        }
        record.push_str(&to_json(event));
    }
    record.push_str(r#"],"kind":"workspace-events","workspaceId":"#);
    record.push_str(&to_json(workspace_id));
    record.push('}');
    record
}

/// The conversation fields `routeForConversation` reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationRoute {
    pub assigned_inbox_team_id: Option<String>,
    pub assigned_user_id: Option<String>,
    pub inbox_id: Option<String>,
}

/// `routeForConversation(conversation, inboxId)`.
pub fn route_for_conversation(
    conversation: Option<&ConversationRoute>,
    inbox_id: Option<&str>,
) -> Route {
    let non_empty = |id: Option<&str>| id.filter(|id| !id.is_empty()).map(str::to_owned);
    let team = conversation.and_then(|c| non_empty(c.assigned_inbox_team_id.as_deref()));
    let user = conversation.and_then(|c| non_empty(c.assigned_user_id.as_deref()));
    // `inboxId ?? conversation?.inboxId`, then a truthy check.
    let inbox = match inbox_id {
        Some(inbox_id) => Some(inbox_id),
        None => conversation.and_then(|c| c.inbox_id.as_deref()),
    };
    Route {
        assigned_team_ids: team.into_iter().collect(),
        assigned_user_ids: user.into_iter().collect(),
        inbox_id: non_empty(inbox),
    }
}

/// The arguments of `routeForAssignment`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssignmentRoute {
    pub assigned_inbox_team_id: Option<String>,
    pub assigned_user_id: Option<String>,
    pub previous_assigned_inbox_team_ids: Vec<Option<String>>,
    pub previous_assigned_user_ids: Vec<Option<String>>,
}

/// `routeForAssignment`: previous + current ids, deduplicated in order,
/// empty ids dropped.
pub fn route_for_assignment(assignment: &AssignmentRoute) -> Route {
    fn merged(previous: &[Option<String>], current: &Option<String>) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for id in previous.iter().chain(std::iter::once(current)).flatten() {
            if !id.is_empty() && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    }
    Route {
        assigned_team_ids: merged(
            &assignment.previous_assigned_inbox_team_ids,
            &assignment.assigned_inbox_team_id,
        ),
        assigned_user_ids: merged(
            &assignment.previous_assigned_user_ids,
            &assignment.assigned_user_id,
        ),
        inbox_id: None,
    }
}

// ---- typed `data` payloads (producers and the schema tests) ----

/// The literal `"voip"`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Voip,
}

/// `realtimeCallDirectionSchema`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CallDirection {
    UserInitiated,
    BusinessInitiated,
}

/// The literal `"offer"`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OfferSdpType {
    Offer,
}

/// The literal `"answer"`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnswerSdpType {
    Answer,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SdpOffer {
    pub sdp_type: OfferSdpType,
    pub sdp: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SdpAnswer {
    pub sdp_type: AnswerSdpType,
    pub sdp: String,
}

/// `realtimeCallTransportIncomingSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallTransportIncoming {
    pub transport: Transport,
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub direction: CallDirection,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_name: Option<String>,
    pub offer: SdpOffer,
    pub deadline_at: String,
}

/// `realtimeCallEndedStatusSchema`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CallEndedStatus {
    Completed,
    Rejected,
    Failed,
}

/// `realtimeCallTransportEndedSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallTransportEnded {
    pub transport: Transport,
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub status: CallEndedStatus,
}

/// `whatsappCallClaimedElsewhereSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappCallClaimedElsewhere {
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub answered_by_user_id: String,
}

/// `realtimeCallTransportOutboundAnswerVoipSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallTransportOutboundAnswer {
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub attempt_id: String,
    pub session: SdpAnswer,
}

/// The outbound call status values.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutboundCallStatus {
    Ringing,
    Accepted,
}

/// `realtimeCallTransportOutboundStatusVoipSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallTransportOutboundStatus {
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub attempt_id: String,
    pub status: OutboundCallStatus,
}

/// `whatsappCallPermissionUpdatedSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappCallPermissionUpdated {
    pub conversation_id: String,
}

/// `threadControlState`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThreadControlState {
    Owned,
    Standby,
    Idle,
}

/// `contactInboxThreadControlUpdatedSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContactInboxThreadControlUpdated {
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub thread_control_state: Option<ThreadControlState>,
    pub thread_owner_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_owner_app_id: Option<String>,
    pub thread_control_updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_owner_expires_at: Option<String>,
    pub thread_control_last_event: Option<String>,
}

/// `pipelineItemChanged.change`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PipelineChange {
    Created,
    Updated,
    Moved,
    Deleted,
    Bulk,
}

/// `pipelineItemChangedSchema`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineItemChanged {
    pub pipeline_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    pub change: PipelineChange,
}

/// Validates an event `data` value against its typed schema (`T::parse`).
pub fn parse_payload<T: DeserializeOwned>(value: &Value) -> Result<T, ProtocolError> {
    serde_json::from_value(value.clone()).map_err(|err| ProtocolError::new(err.to_string()))
}

// ---- sequence ids (`stream.ts:8,44-64`) ----

/// A stream id `<ms>-<n>`. Ordered by `ms`, then `n`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Seq {
    pub ms: u64,
    pub n: u64,
}

impl Seq {
    /// The cursor-less sentinel `0-0`.
    pub const ZERO: Seq = Seq { ms: 0, n: 0 };

    pub const fn new(ms: u64, n: u64) -> Seq {
        Seq { ms, n }
    }
}

/// A string that is not `^\d+-\d+$` (or does not fit in `u64` parts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSeq;

impl fmt::Display for InvalidSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a stream id")
    }
}

impl std::error::Error for InvalidSeq {}

impl FromStr for Seq {
    type Err = InvalidSeq;

    fn from_str(s: &str) -> Result<Seq, InvalidSeq> {
        let (ms, n) = s.split_once('-').ok_or(InvalidSeq)?;
        if !is_digits(ms) || !is_digits(n) {
            return Err(InvalidSeq);
        }
        Ok(Seq {
            ms: ms.parse().map_err(|_| InvalidSeq)?,
            n: n.parse().map_err(|_| InvalidSeq)?,
        })
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.ms, self.n)
    }
}

/// `STREAM_ID_PATTERN.test(s)`.
pub fn is_stream_id(s: &str) -> bool {
    s.split_once('-')
        .is_some_and(|(ms, n)| is_digits(ms) && is_digits(n))
}

/// `isRealtimeSeqAfter`: `candidate` strictly newer than `previous`;
/// `false` for malformed ids. Compares the numeric values of any length.
pub fn is_realtime_seq_after(candidate: &str, previous: &str) -> bool {
    let (Some((candidate_ms, candidate_n)), Some((previous_ms, previous_n))) =
        (candidate.split_once('-'), previous.split_once('-'))
    else {
        return false;
    };
    if !(is_stream_id(candidate) && is_stream_id(previous)) {
        return false;
    }
    match compare_digits(candidate_ms, previous_ms) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            compare_digits(candidate_n, previous_n) == std::cmp::Ordering::Greater
        }
    }
}

// ---- stream records (`stream.ts:66-112`) ----

/// `member-revoke.reason`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RevokeReason {
    /// The member was removed: close 4001 `revoked`, no reconnect.
    Deleted,
    /// Permissions or teams changed: close 4004 `reauth`, reconnect.
    Reauth,
}

impl RevokeReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            RevokeReason::Deleted => "deleted",
            RevokeReason::Reauth => "reauth",
        }
    }
}

/// `realtimeStreamRecordSchema` (the 4 kinds). In one process a record is
/// a direct call; the type stays for the wire contract and the tests.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StreamRecord {
    #[serde(rename_all = "camelCase")]
    WorkspaceEvents {
        events: Vec<Envelope>,
        workspace_id: String,
    },
    #[serde(rename_all = "camelCase")]
    GuestEvent {
        event: Envelope,
        guest_conversation_id: String,
        workspace_id: String,
    },
    #[serde(rename_all = "camelCase")]
    MemberSend {
        event: Envelope,
        user_id: String,
        workspace_id: String,
    },
    #[serde(rename_all = "camelCase")]
    MemberRevoke {
        reason: RevokeReason,
        revoked_at_ms: u64,
        user_id: String,
        workspace_id: String,
    },
}

/// `realtimeStreamRecordSchema.parse` (with the per-kind envelope rules).
pub fn parse_stream_record(value: &Value) -> Result<StreamRecord, ProtocolError> {
    let record: StreamRecord =
        serde_json::from_value(value.clone()).map_err(|err| ProtocolError::new(err.to_string()))?;
    let require = |field: &str, id: &str| {
        if id.is_empty() {
            Err(ProtocolError::new(format!("{field}: must not be empty")))
        } else {
            Ok(())
        }
    };
    match &record {
        StreamRecord::WorkspaceEvents {
            events,
            workspace_id,
        } => {
            require("workspaceId", workspace_id)?;
            if events.is_empty() {
                return Err(ProtocolError::new("events: must hold at least one event"));
            }
            for event in events {
                validate_workspace_envelope(event)?;
            }
        }
        StreamRecord::GuestEvent {
            event,
            guest_conversation_id,
            workspace_id,
        } => {
            require("guestConversationId", guest_conversation_id)?;
            require("workspaceId", workspace_id)?;
            validate_guest_envelope(event)?;
        }
        StreamRecord::MemberSend {
            user_id,
            workspace_id,
            ..
        }
        | StreamRecord::MemberRevoke {
            user_id,
            workspace_id,
            ..
        } => {
            require("userId", user_id)?;
            require("workspaceId", workspace_id)?;
        }
    }
    Ok(record)
}

/// `getRealtimeMemberRevokedKey`: `realtime:revoked:<ws>:<user>`.
pub fn member_revoked_key(workspace_id: &str, user_id: &str) -> String {
    format!("realtime:revoked:{workspace_id}:{user_id}")
}

// ---- close codes (`realtime-socket.ts:1-13`) ----

/// `REALTIME_CLOSE_CODE`.
pub mod close {
    /// Client side: no frame for 60 s.
    pub const HEARTBEAT_TIMEOUT: u16 = 4000;
    /// Member removed; the client must not reconnect.
    pub const REVOKED: u16 = 4001;
    /// The client dropped events: resync, then reconnect.
    pub const RESYNC: u16 = 4002;
    /// Overloaded or shutting down: reconnect after `retryAfter`.
    pub const OVERLOADED: u16 = 4003;
    /// Reconnect with a new token.
    pub const REAUTH: u16 = 4004;
}

/// The 4002 / 4004 / 4001 reason texts Rust still sends.
pub mod reason {
    pub const INVALID_LAST_SEQ: &str = "invalid-last-seq";
    pub const REPLAY_CURSOR_AHEAD: &str = "replay-cursor-ahead";
    pub const REPLAY_WINDOW_EXPIRED: &str = "replay-window-expired";
    pub const REPLAY_WINDOW_TOO_LARGE: &str = "replay-window-too-large";
    pub const BACKPRESSURE_DROP: &str = "backpressure-drop";
    pub const SEND_FAILED: &str = "send-failed";
    pub const REVOKED: &str = "revoked";
    pub const REAUTH: &str = "reauth";
    pub const CONNECTION_LIFETIME_EXCEEDED: &str = "connection-lifetime-exceeded";
}

/// `{"retryAfter":<ms>}`, the 4003 close reason.
pub fn retry_after_reason(retry_after_ms: u64) -> String {
    format!(r#"{{"retryAfter":{retry_after_ms}}}"#)
}

/// The `{"hb":1}` heartbeat frame.
pub const HEARTBEAT_FRAME: &str = r#"{"hb":1}"#;

// ---- presence contract (`presence.ts`) ----

/// How long one presence report keeps a reported user online.
pub const PRESENCE_TTL_MS: u64 = 20_000;
/// Half the TTL: one slow report never flaps a member offline.
pub const PRESENCE_REPORT_INTERVAL_MS: u64 = 10_000;
/// Cap on user ids per report (truncate, never reject).
pub const MAX_PRESENCE_USER_IDS_PER_REPORT: usize = 5000;

const _: () = assert!(PRESENCE_REPORT_INTERVAL_MS * 2 <= PRESENCE_TTL_MS);

/// `truncatePresenceUserIds`: the first 5 000 ids.
pub fn truncate_presence_user_ids(user_ids: &[String]) -> Vec<String> {
    user_ids
        .iter()
        .take(MAX_PRESENCE_USER_IDS_PER_REPORT)
        .cloned()
        .collect()
}

/// The presence lease key `workspace:presence:<ws>`.
pub fn workspace_presence_key(workspace_id: &str) -> String {
    format!("workspace:presence:{workspace_id}")
}

// ---- helpers ----

/// JSON text of a value whose serialization cannot fail (strings, typed
/// envelopes with `Value` data).
pub(crate) fn to_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// `batchOf(eventSchema)`: `{batch: eventSchema[], seq: /^\d+-\d+$/}`.
fn parse_batch(
    value: &Value,
    parse_event: fn(&Value) -> Result<Envelope, ProtocolError>,
) -> Result<BatchFrame, ProtocolError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::new("expected an object"))?;
    let batch = object
        .get("batch")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::new("batch: expected an array"))?;
    let seq = object
        .get("seq")
        .and_then(Value::as_str)
        .filter(|seq| is_stream_id(seq))
        .ok_or_else(|| ProtocolError::new("seq: expected a stream id"))?;
    let batch = batch
        .iter()
        .enumerate()
        .map(|(index, event)| {
            parse_event(event).map_err(|err| ProtocolError::new(format!("batch.{index}: {err}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BatchFrame {
        batch,
        seq: seq.to_owned(),
    })
}

/// `\d+` (ASCII digits only, as the JS regex).
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Numeric order of two digit strings of any length (`BigInt` compare).
fn compare_digits(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
