//! Port of `partials/channel.ts` (`resolveChannelConversationId`).

use serde_json::Value;

/// `resolveChannelConversationId`: `additionalAttributes.channelConversationId` when it is
/// a non-empty string, else the legacy `sourceId`.
pub fn resolve_channel_conversation_id(
    source_id: Option<&str>,
    additional_attributes: Option<&Value>,
) -> Option<String> {
    let stored = additional_attributes
        .and_then(|attributes| attributes.get("channelConversationId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    stored.or(source_id).map(str::to_owned)
}
