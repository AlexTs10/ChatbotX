//! Port of `packages/database/__tests__/conversation-ai-context-repository.test.ts` (P3.2).
//! The mocked `findFirst` becomes a real Conversation row in a migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::{
    self,
    conversation_ai_context::{ConversationAiContextState, find_conversation_ai_context_state},
};
use common::{fresh_db, insert_conversation, seed};
use repo_common::ms;

/// Port of: packages/database/__tests__/conversation-ai-context-repository.test.ts:5 "findConversationAIContextState > scopes the state read by conversation and workspace"
#[tokio::test]
async fn find_conversation_ai_context_state_scopes_the_state_read_by_conversation_and_workspace() {
    let db = fresh_db().await;
    let s = seed(&db).await;
    let at = ms("2026-06-01T00:00:00Z");
    repo_common::setup(&db, move |tx| {
        insert_conversation(tx, s, 9001, None)?;
        tx.execute(
            r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 10, "lastActivityAt" = ?1 WHERE "id" = 9001"#,
            [at],
        )?;
        Ok(())
    })
    .await;
    let state = repo::read(&db, |c| find_conversation_ai_context_state(c, 9001, 2001))
        .await
        .expect("read");
    assert_eq!(
        state,
        Some(ConversationAiContextState {
            ai_context_last_message_id: Some(10),
            last_activity_at: Some(at),
        })
    );
}

/// Port of: packages/database/__tests__/conversation-ai-context-repository.test.ts:28 "findConversationAIContextState > returns null when the scoped conversation does not exist"
#[tokio::test]
async fn find_conversation_ai_context_state_returns_null_when_the_scoped_conversation_does_not_exist()
 {
    let db = fresh_db().await;
    let s = seed(&db).await;
    repo_common::setup(&db, move |tx| {
        insert_conversation(tx, s, 9001, None)?;
        Ok(())
    })
    .await;
    // The conversation exists, but in another workspace than the one asked for.
    let state = repo::read(&db, |c| find_conversation_ai_context_state(c, 9001, 9999))
        .await
        .expect("read");
    assert_eq!(state, None);
}
