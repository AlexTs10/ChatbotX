//! Port of `packages/realtime-protocol/__tests__/schemas.test.ts` (P2.8
//! spec 4.6, R57-R93). The zod schemas are the `rt::protocol` parse
//! functions over JSON values; `test.each(Object.entries(EVENT_DELIVERY))`
//! is one Rust test per row with the row's delivery as data.

use chatbotx_rt::protocol::{
    AssignmentRoute, CallEndedStatus, CallTransportEnded, CallTransportIncoming,
    CallTransportOutboundAnswer, CallTransportOutboundStatus, ConversationRoute, Delivery,
    EventType, OutboundCallStatus, PipelineItemChanged, Route, Transport,
    WhatsappCallClaimedElsewhere, delivery, parse_envelope, parse_guest_batch, parse_member_batch,
    parse_payload, parse_workspace_batch, parse_workspace_envelope, route_for_assignment,
    route_for_conversation,
};
use serde_json::{Value, json};

const CONVERSATION_SCOPED_ROUTE_ERROR: &str = "conversation-scoped and requires a route";
const TARGETED_ONLY_EVENT_ERROR: &str = "targeted-only and cannot be workspace-broadcast";

fn route() -> Value {
    json!({ "assignedTeamIds": [], "assignedUserIds": ["user-1"] })
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:29 "realtime batch envelopes > preserves every event in one sequenced record frame"
#[test]
fn realtime_batch_envelopes_preserves_every_event_in_one_sequenced_record_frame() {
    let frame = parse_workspace_batch(&json!({
        "batch": [
            { "data": { "id": "message-1" }, "eventType": "messageCreated", "route": route() },
            { "data": { "id": "message-2" }, "eventType": "messageCreated", "route": route() },
        ],
        "seq": "123-0",
    }))
    .unwrap();

    assert_eq!(frame.batch.len(), 2);
    assert_eq!(frame.seq, "123-0");
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:42 "realtime batch envelopes > rejects a conversation-scoped event missing its route"
#[test]
fn realtime_batch_envelopes_rejects_a_conversation_scoped_event_missing_its_route() {
    let err = parse_workspace_batch(&json!({
        "batch": [{ "data": { "id": "message-1" }, "eventType": "messageCreated" }],
        "seq": "123-0",
    }))
    .unwrap_err();

    assert!(
        err.to_string().contains(CONVERSATION_SCOPED_ROUTE_ERROR),
        "{err}"
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:51 "realtime batch envelopes > allows a workspace-wide event with no route"
#[test]
fn realtime_batch_envelopes_allows_a_workspace_wide_event_with_no_route() {
    let frame = parse_workspace_batch(&json!({
        "batch": [{ "data": { "contactId": "contact-1" }, "eventType": "contactBlocked" }],
        "seq": "123-0",
    }))
    .unwrap();

    assert_eq!(frame.batch.len(), 1);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:62 "realtime batch envelopes > rejects a whatsappCallPermissionUpdated event missing its route"
#[test]
fn realtime_batch_envelopes_rejects_a_whatsappcallpermissionupdated_event_missing_its_route() {
    // Permission-mode changes are conversation-scoped.
    let err = parse_workspace_batch(&json!({
        "batch": [{
            "data": { "conversationId": "conv-1" },
            "eventType": "whatsappCallPermissionUpdated",
        }],
        "seq": "123-0",
    }))
    .unwrap_err();

    assert!(
        err.to_string().contains(CONVERSATION_SCOPED_ROUTE_ERROR),
        "{err}"
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:77 "realtime batch envelopes > allows a whatsappCallPermissionUpdated event with its route"
#[test]
fn realtime_batch_envelopes_allows_a_whatsappcallpermissionupdated_event_with_its_route() {
    let frame = parse_workspace_batch(&json!({
        "batch": [{
            "data": { "conversationId": "conv-1" },
            "eventType": "whatsappCallPermissionUpdated",
            "route": route(),
        }],
        "seq": "123-0",
    }))
    .unwrap();

    assert_eq!(frame.batch.len(), 1);
}

/// One `test.each(Object.entries(EVENT_DELIVERY))` row.
fn enforces_delivery_requirements_for(event_type: &str, expected: Delivery) {
    let parsed: EventType = serde_json::from_value(json!(event_type)).unwrap();
    assert_eq!(delivery(parsed), expected);

    let result = parse_workspace_batch(&json!({
        "batch": [{ "data": {}, "eventType": event_type }],
        "seq": "123-0",
    }));

    assert_eq!(
        result.is_ok(),
        expected == Delivery::Workspace,
        "{event_type}"
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageCreated"
#[test]
fn enforces_delivery_requirements_for_messagecreated() {
    enforces_delivery_requirements_for("messageCreated", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageDeleted"
#[test]
fn enforces_delivery_requirements_for_messagedeleted() {
    enforces_delivery_requirements_for("messageDeleted", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageUpdated"
#[test]
fn enforces_delivery_requirements_for_messageupdated() {
    enforces_delivery_requirements_for("messageUpdated", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageContentUpdated"
#[test]
fn enforces_delivery_requirements_for_messagecontentupdated() {
    enforces_delivery_requirements_for("messageContentUpdated", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageIdAssigned"
#[test]
fn enforces_delivery_requirements_for_messageidassigned() {
    enforces_delivery_requirements_for("messageIdAssigned", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for messageFailed"
#[test]
fn enforces_delivery_requirements_for_messagefailed() {
    enforces_delivery_requirements_for("messageFailed", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for typing"
#[test]
fn enforces_delivery_requirements_for_typing() {
    enforces_delivery_requirements_for("typing", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for contactBlocked"
#[test]
fn enforces_delivery_requirements_for_contactblocked() {
    enforces_delivery_requirements_for("contactBlocked", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for contactUnblocked"
#[test]
fn enforces_delivery_requirements_for_contactunblocked() {
    enforces_delivery_requirements_for("contactUnblocked", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for conversationAssigned"
#[test]
fn enforces_delivery_requirements_for_conversationassigned() {
    enforces_delivery_requirements_for("conversationAssigned", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for conversationUpdated"
#[test]
fn enforces_delivery_requirements_for_conversationupdated() {
    enforces_delivery_requirements_for("conversationUpdated", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallTransportIncoming"
#[test]
fn enforces_delivery_requirements_for_whatsappcalltransportincoming() {
    enforces_delivery_requirements_for("whatsappCallTransportIncoming", Delivery::Targeted);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallTransportEnded"
#[test]
fn enforces_delivery_requirements_for_whatsappcalltransportended() {
    enforces_delivery_requirements_for("whatsappCallTransportEnded", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallClaimedElsewhere"
#[test]
fn enforces_delivery_requirements_for_whatsappcallclaimedelsewhere() {
    enforces_delivery_requirements_for("whatsappCallClaimedElsewhere", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallOutboundAnswer"
#[test]
fn enforces_delivery_requirements_for_whatsappcalloutboundanswer() {
    enforces_delivery_requirements_for("whatsappCallOutboundAnswer", Delivery::Targeted);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallOutboundStatus"
#[test]
fn enforces_delivery_requirements_for_whatsappcalloutboundstatus() {
    enforces_delivery_requirements_for("whatsappCallOutboundStatus", Delivery::Targeted);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for whatsappCallPermissionUpdated"
#[test]
fn enforces_delivery_requirements_for_whatsappcallpermissionupdated() {
    enforces_delivery_requirements_for("whatsappCallPermissionUpdated", Delivery::Conversation);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for contactInboxThreadControlUpdated"
#[test]
fn enforces_delivery_requirements_for_contactinboxthreadcontrolupdated() {
    enforces_delivery_requirements_for("contactInboxThreadControlUpdated", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:93 "enforces delivery requirements for pipelineItemChanged"
#[test]
fn enforces_delivery_requirements_for_pipelineitemchanged() {
    enforces_delivery_requirements_for("pipelineItemChanged", Delivery::Workspace);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:104 "allows a conversation route with no assigned recipients"
#[test]
fn allows_a_conversation_route_with_no_assigned_recipients() {
    assert!(
        parse_workspace_batch(&json!({
            "batch": [{
                "data": {},
                "eventType": "messageCreated",
                "route": { "assignedTeamIds": [], "assignedUserIds": [] },
            }],
            "seq": "123-0",
        }))
        .is_ok()
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:119 "rejects unknown realtime event types"
#[test]
fn rejects_unknown_realtime_event_types() {
    assert!(parse_envelope(&json!({ "data": {}, "eventType": "unknown" })).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:128 "rejects targeted-only events from workspace batches"
#[test]
fn rejects_targeted_only_events_from_workspace_batches() {
    let err = parse_workspace_batch(&json!({
        "batch": [{
            "data": { "attemptId": "attempt-1" },
            "eventType": "whatsappCallTransportIncoming",
        }],
        "seq": "123-0",
    }))
    .unwrap_err();

    assert!(err.to_string().contains(TARGETED_ONLY_EVENT_ERROR), "{err}");
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:142 "rejects targeted-only events from guest readers"
#[test]
fn rejects_targeted_only_events_from_guest_readers() {
    let targeted_frame = json!({
        "batch": [{
            "data": { "attemptId": "attempt-1" },
            "eventType": "whatsappCallTransportIncoming",
        }],
        "seq": "123-0",
    });

    let member_frame = parse_member_batch(&targeted_frame).unwrap();
    assert_eq!(serde_json::to_value(&member_frame).unwrap(), targeted_frame);
    assert!(parse_guest_batch(&targeted_frame).is_err());
    assert!(parse_workspace_batch(&targeted_frame).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:167 "realtime guest batch envelopes > allows a conversation-scoped event with no route"
#[test]
fn realtime_guest_batch_envelopes_allows_a_conversation_scoped_event_with_no_route() {
    // Guest delivery is never route-filtered, including conversation-scoped
    // events.
    let frame = parse_guest_batch(&json!({
        "batch": [{ "data": { "id": "message-1" }, "eventType": "messageCreated" }],
        "seq": "123-0",
    }))
    .unwrap();

    assert_eq!(frame.batch.len(), 1);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:176 "realtime guest batch envelopes > allows the guest-only typing payload"
#[test]
fn realtime_guest_batch_envelopes_allows_the_guest_only_typing_payload() {
    assert!(
        parse_guest_batch(&json!({
            "batch": [{ "data": { "typing": true }, "eventType": "typing" }],
            "seq": "123-0",
        }))
        .is_ok()
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:189 "realtime event routes > includes previous and current assignees for assignment changes"
#[test]
fn realtime_event_routes_includes_previous_and_current_assignees_for_assignment_changes() {
    assert_eq!(
        route_for_assignment(&AssignmentRoute {
            assigned_inbox_team_id: Some("team-next".to_owned()),
            assigned_user_id: Some("user-next".to_owned()),
            previous_assigned_inbox_team_ids: vec![Some("team-previous".to_owned())],
            previous_assigned_user_ids: vec![Some("user-previous".to_owned())],
        }),
        Route {
            assigned_team_ids: vec!["team-previous".to_owned(), "team-next".to_owned()],
            assigned_user_ids: vec!["user-previous".to_owned(), "user-next".to_owned()],
            inbox_id: None,
        }
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:203 "realtime event routes > keeps conversation assignment and inbox routing data"
#[test]
fn realtime_event_routes_keeps_conversation_assignment_and_inbox_routing_data() {
    assert_eq!(
        route_for_conversation(
            Some(&ConversationRoute {
                assigned_inbox_team_id: Some("team-1".to_owned()),
                assigned_user_id: Some("user-1".to_owned()),
                inbox_id: Some("inbox-1".to_owned()),
            }),
            None,
        ),
        Route {
            assigned_team_ids: vec!["team-1".to_owned()],
            assigned_user_ids: vec!["user-1".to_owned()],
            inbox_id: Some("inbox-1".to_owned()),
        }
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:219 "call payload schemas > accepts a complete incoming directed offer"
#[test]
fn call_payload_schemas_accepts_a_complete_incoming_directed_offer() {
    let payload = parse_payload::<CallTransportIncoming>(&json!({
        "contactInboxId": "contact-inbox-1",
        "conversationId": "conversation-1",
        "deadlineAt": "2026-01-01T00:00:00.000Z",
        "direction": "userInitiated",
        "offer": { "sdp": "offer", "sdpType": "offer" },
        "transport": "voip",
        "wacid": "call-1",
        "whatsappCallId": "whatsapp-call-1",
    }))
    .unwrap();

    assert_eq!(payload.transport, Transport::Voip);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:234 "call payload schemas > rejects an incoming payload with a non-offer session"
#[test]
fn call_payload_schemas_rejects_an_incoming_payload_with_a_non_offer_session() {
    assert!(
        parse_payload::<CallTransportIncoming>(&json!({
            "contactInboxId": "contact-inbox-1",
            "conversationId": "conversation-1",
            "deadlineAt": "2026-01-01T00:00:00.000Z",
            "direction": "userInitiated",
            "offer": { "sdp": "answer", "sdpType": "answer" },
            "transport": "voip",
            "wacid": "call-1",
            "whatsappCallId": "whatsapp-call-1",
        }))
        .is_err()
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:249 "call payload schemas > validates terminal, claimed, and outbound call payloads"
#[test]
fn call_payload_schemas_validates_terminal_claimed_and_outbound_call_payloads() {
    let ended = parse_payload::<CallTransportEnded>(&json!({
        "status": "completed",
        "transport": "voip",
        "wacid": "call-1",
        "whatsappCallId": "whatsapp-call-1",
    }))
    .unwrap();
    assert_eq!(ended.status, CallEndedStatus::Completed);

    let claimed = parse_payload::<WhatsappCallClaimedElsewhere>(&json!({
        "answeredByUserId": "user-1",
        "wacid": "call-1",
        "whatsappCallId": "whatsapp-call-1",
    }))
    .unwrap();
    assert_eq!(claimed.answered_by_user_id, "user-1");

    let answer = parse_payload::<CallTransportOutboundAnswer>(&json!({
        "attemptId": "attempt-1",
        "session": { "sdp": "answer", "sdpType": "answer" },
        "wacid": "call-1",
        "whatsappCallId": "whatsapp-call-1",
    }))
    .unwrap();
    assert_eq!(answer.attempt_id, "attempt-1");

    let status = parse_payload::<CallTransportOutboundStatus>(&json!({
        "attemptId": "attempt-1",
        "status": "ringing",
        "wacid": "call-1",
        "whatsappCallId": "whatsapp-call-1",
    }))
    .unwrap();
    assert_eq!(status.status, OutboundCallStatus::Ringing);
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:285 "pipelineItemChanged event > is a workspace-broadcast event that needs no route"
#[test]
fn pipelineitemchanged_event_is_a_workspace_broadcast_event_that_needs_no_route() {
    let parsed = parse_workspace_envelope(&json!({
        "eventType": "pipelineItemChanged",
        "data": { "pipelineId": "1", "change": "bulk" },
    }));

    assert!(parsed.is_ok());
    assert_eq!(
        delivery(EventType::PipelineItemChanged),
        Delivery::Workspace
    );
}

/// Port of: packages/realtime-protocol/__tests__/schemas.test.ts:297 "pipelineItemChanged event > accepts only the documented change kinds and ids"
#[test]
fn pipelineitemchanged_event_accepts_only_the_documented_change_kinds_and_ids() {
    assert!(
        parse_payload::<PipelineItemChanged>(&json!({ "pipelineId": "1", "change": "moved" }))
            .is_ok()
    );
    assert!(
        parse_payload::<PipelineItemChanged>(&json!({ "pipelineId": "1", "change": "x" })).is_err()
    );
    assert!(parse_payload::<PipelineItemChanged>(&json!({ "change": "moved" })).is_err());
}
