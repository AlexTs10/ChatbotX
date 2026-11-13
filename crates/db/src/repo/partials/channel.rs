//! Port of `partials/channel.ts` (`resolveChannelConversationId`).

use serde_json::Value;

/// `resolveChannelConversationId`: `additionalAttributes.channelConversationId` when it is
/// a non-empty string, else the legacy `sourceId`.
pub fn resolve_channel_conversation_id(
    source_id: Option<&str>,
    additional_attributes: Option<&Value>,
) -> Option<String> {
    let _ = (source_id, additional_attributes);
    todo!("P3.2b")
}
