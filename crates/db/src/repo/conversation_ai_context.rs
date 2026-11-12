//! Port of `repositories/conversation-ai-context/repository.ts`.

use rusqlite::Connection;

use super::Result;

/// `ConversationAIContextState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAiContextState {
    pub ai_context_last_message_id: Option<i64>,
    pub last_activity_at: Option<i64>,
}

/// `findConversationAIContextState`: the two columns of the conversation, scoped by
/// conversation and workspace; `None` when there is no such conversation.
pub fn find_conversation_ai_context_state(
    conn: &Connection,
    conversation_id: i64,
    workspace_id: i64,
) -> Result<Option<ConversationAiContextState>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"aiContextLastMessageId\", \"lastActivityAt\" FROM \"Conversation\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 LIMIT 1",
    )?;
    let mut rows = stmt.query([conversation_id, workspace_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(ConversationAiContextState {
            ai_context_last_message_id: row.get(0)?,
            last_activity_at: row.get(1)?,
        })),
        None => Ok(None),
    }
}
