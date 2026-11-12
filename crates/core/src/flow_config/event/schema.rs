//! Port of `packages/flow-config/src/event/schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn message_event_type_schema = z::enum_(&["message:sent", "message:delivered", "message:seen", "message:failed", "message:received"]));

schema!(pub fn flow_event_type_schema = z::enum_(&["flow:clicked", "flow:ref"]));

schema!(pub fn event_context_schema = z::object(vec![("workspaceId", z::string()),
("contactId", z::string()),
("conversationId", z::string()),
("channel", z::string()),
("contactInboxId", z::string().optional()),
("inboxId", z::string().optional()),
("sequenceStepId", z::string().optional()),
("sourceId", z::string().optional())]));

schema!(pub fn message_action_schema = z::object(vec![("flowId", z::string().optional()),
("flowVersionId", z::string().optional()),
("sourceId", z::string().optional()),
("messageId", z::string().optional()),
("messageDetail", z::record(z::string(), z::unknown()).optional())]));

schema!(pub fn click_type_schema = z::enum_(&["button", "quick_reply", "magic_link"]));

schema!(pub fn flow_click_action_schema = z::object(vec![("flowId", z::string()),
("buttonId", z::string().optional()),
("nodeId", z::string().optional()),
("broadcastId", z::string().optional()),
("sequenceStepId", z::string().optional()),
("commentAutomationId", z::string().optional()),
("magicLinkId", z::string().optional()),
("clickType", click_type_schema())]));

schema!(pub fn ref_link_action_schema = z::object(vec![("refId", z::string()),
("refType", z::enum_(&["entryPoint"]))]));

schema!(pub fn sent_payload_schema = z::object(vec![("context", event_context_schema()),
("action", message_action_schema()),
("stepId", z::string().optional()),
("nodeId", z::string().optional()),
("occurredAt", z::date()),
("metadata", z::custom(sent_payload_schema_metadata_custom, None).optional())]));

schema!(pub fn failed_payload_schema = z::object(vec![("context", event_context_schema()),
("action", message_action_schema()),
("stepId", z::string().optional()),
("nodeId", z::string().optional()),
("occurredAt", z::date()),
("metadata", z::custom(sent_payload_schema_metadata_custom, None).optional()),
("errorData", z::unknown()),
("errorStack", z::string().optional()),
("willRetry", z::boolean().optional())]));

schema!(pub fn delivered_payload_schema = z::object(vec![("context", event_context_schema()),
("action", message_action_schema()),
("stepId", z::string().optional()),
("nodeId", z::string().optional()),
("occurredAt", z::date()),
("metadata", z::custom(sent_payload_schema_metadata_custom, None).optional()),
("providerOccurredAt", z::date().optional())]));

schema!(pub fn seen_payload_schema = z::object(vec![("context", event_context_schema()),
("action", message_action_schema()),
("stepId", z::string().optional()),
("nodeId", z::string().optional()),
("occurredAt", z::date()),
("metadata", z::custom(sent_payload_schema_metadata_custom, None).optional())]));

schema!(pub fn received_payload_schema = z::object(vec![("workspaceId", z::string()),
("contactId", z::string()),
("contactInboxId", z::string()),
("channel", z::string()),
("inboxId", z::string()),
("sourceId", z::string().optional()),
("occurredAt", z::date()),
("origin", z::literal(json!("inbound")).optional()),
("messageId", z::string().optional()),
("isFirstIncomingMessage", z::boolean().optional())]));

schema!(pub fn clicked_payload_schema = z::object(vec![("context", event_context_schema()),
("action", flow_click_action_schema()),
("stepId", z::string().optional()),
("nodeId", z::string().optional()),
("occurredAt", z::date())]));

schema!(pub fn ref_link_payload_schema = z::object(vec![("context", event_context_schema()),
("action", ref_link_action_schema()),
("occurredAt", z::date())]));

schema!(pub fn contact_sender_type_schema = z::enum_(&["bot", "human"]));

schema!(pub fn bot_message_response_type_schema = z::enum_(&["automated_response", "ai_agent", "flow", "none"]));

schema!(pub fn bot_message_route_type_schema = z::enum_(&["flow", "agent", "fallback"]));

schema!(pub fn bot_message_result_schema = z::enum_(&["success", "fallback"]));

schema!(pub fn bot_message_payload_schema = z::object(vec![("workspaceId", z::string()),
("messageId", z::string()),
("conversationId", z::string()),
("occurredAt", z::union(vec![z::date(),
z::string(),
z::number()])),
("hasResponse", z::boolean()),
("responseType", bot_message_response_type_schema().optional()),
("routeType", bot_message_route_type_schema().optional()),
("result", bot_message_result_schema().optional()),
("aiProvider", z::string().optional())]));

fn sent_payload_schema_metadata_custom(_value: &Value) -> bool {
    true
}

/// `messageEventSchemas`: payload schema by event type.
pub fn message_event_schemas() -> Vec<(&'static str, Schema)> {
    vec![
        ("message:sent", sent_payload_schema()),
        ("message:failed", failed_payload_schema()),
        ("message:delivered", delivered_payload_schema()),
        ("message:seen", seen_payload_schema()),
        ("message:received", received_payload_schema()),
    ]
}

/// `flowEventSchemas`.
pub fn flow_event_schemas() -> Vec<(&'static str, Schema)> {
    vec![
        ("flow:clicked", clicked_payload_schema()),
        ("flow:ref", ref_link_payload_schema()),
    ]
}
