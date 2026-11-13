//! Port of `packages/database/__tests__/channel-partial.test.ts` (P3.2b, ported: the
//! conversation object is the two arguments; `additionalAttributes` is the stored JSON).

use chatbotx_db::repo::partials::channel::resolve_channel_conversation_id;
use serde_json::json;

#[test]
fn resolve_channel_conversation_id_reads_the_id_from_additional_attributes() {
    let attributes = json!({ "channelConversationId": "conv-1" });
    assert_eq!(
        resolve_channel_conversation_id(None, Some(&attributes)).as_deref(),
        Some("conv-1")
    );
}

#[test]
fn resolve_channel_conversation_id_prefers_additional_attributes_over_a_legacy_source_id() {
    let attributes = json!({ "channelConversationId": "conv-1" });
    assert_eq!(
        resolve_channel_conversation_id(Some("legacy-conv"), Some(&attributes)).as_deref(),
        Some("conv-1")
    );
}

#[test]
fn resolve_channel_conversation_id_falls_back_to_source_id_for_rows_the_backfill_has_not_reached() {
    assert_eq!(
        resolve_channel_conversation_id(Some("legacy-conv"), None).as_deref(),
        Some("legacy-conv")
    );
}

#[test]
fn resolve_channel_conversation_id_ignores_a_non_string_or_empty_stored_value() {
    let number = json!({ "channelConversationId": 42 });
    assert_eq!(
        resolve_channel_conversation_id(Some("legacy-conv"), Some(&number)).as_deref(),
        Some("legacy-conv")
    );
    let empty = json!({ "channelConversationId": "" });
    assert_eq!(
        resolve_channel_conversation_id(Some("legacy-conv"), Some(&empty)).as_deref(),
        Some("legacy-conv")
    );
}

#[test]
fn resolve_channel_conversation_id_returns_null_for_a_dm_conversation_that_carries_neither() {
    let attributes = json!({});
    assert_eq!(
        resolve_channel_conversation_id(None, Some(&attributes)),
        None
    );
}
