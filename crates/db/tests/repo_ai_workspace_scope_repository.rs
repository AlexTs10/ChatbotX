//! Port of `packages/database/__tests__/ai-workspace-scope-repository.test.ts` (P3.2).
//! The mocked `findFirst` becomes a real row per scope in a migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::{
    self,
    ai_workspace_scope::{AI_WORKSPACE_SCOPES, AiWorkspaceScope, find_workspace_id},
};
use common::{fresh_db, insert_conversation, seed};

/// One row per scope (ids 1-4) of the seeded workspace 2001.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    let s = seed(&db).await;
    repo_common::setup(&db, move |tx| {
        insert_conversation(tx, s, 9001, None)?;
        ins!(tx, "AIFile", { "id" => 2_i64, "name" => "f", "path" => "p", "size" => 1_i64, "mimeType" => "text/plain", "workspaceId" => s.workspace_id })?;
        ins!(tx, "AIEmbedding", { "id" => 1_i64, "content" => "c", "workspaceId" => s.workspace_id, "aiFileId" => 2_i64 })?;
        ins!(tx, "AIConversationSource", { "id" => 4_i64, "workspaceId" => s.workspace_id, "conversationId" => 9001_i64, "messageId" => 1_i64, "sourceType" => "document", "sourceKey" => "k" })?;
        ins!(tx, "AIConversationEmbedding", { "id" => 3_i64, "sourceId" => 4_i64, "workspaceId" => s.workspace_id, "conversationId" => 9001_i64, "chunkIndex" => 0_i64, "content" => "c" })?;
        Ok(())
    })
    .await;
    db
}

fn id_of(scope: AiWorkspaceScope) -> i64 {
    match scope {
        AiWorkspaceScope::AiEmbedding => 1,
        AiWorkspaceScope::AiFile => 2,
        AiWorkspaceScope::ConversationEmbedding => 3,
        AiWorkspaceScope::ConversationSource => 4,
    }
}

async fn resolves(scope: AiWorkspaceScope) {
    let db = seeded().await;
    let id = id_of(scope);
    let ws = repo::read(&db, move |c| find_workspace_id(c, id, scope))
        .await
        .expect("read");
    assert_eq!(ws, Some(2001));
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:22 "AiWorkspaceScopeRepository > resolves the owning workspace for the %s scope" (aiEmbedding)
#[tokio::test]
async fn ai_workspace_scope_repository_resolves_the_owning_workspace_for_the_ai_embedding_scope() {
    resolves(AiWorkspaceScope::AiEmbedding).await;
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:22 "AiWorkspaceScopeRepository > resolves the owning workspace for the %s scope" (aiFile)
#[tokio::test]
async fn ai_workspace_scope_repository_resolves_the_owning_workspace_for_the_ai_file_scope() {
    resolves(AiWorkspaceScope::AiFile).await;
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:22 "AiWorkspaceScopeRepository > resolves the owning workspace for the %s scope" (conversationEmbedding)
#[tokio::test]
async fn ai_workspace_scope_repository_resolves_the_owning_workspace_for_the_conversation_embedding_scope()
 {
    resolves(AiWorkspaceScope::ConversationEmbedding).await;
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:22 "AiWorkspaceScopeRepository > resolves the owning workspace for the %s scope" (conversationSource)
#[tokio::test]
async fn ai_workspace_scope_repository_resolves_the_owning_workspace_for_the_conversation_source_scope()
 {
    resolves(AiWorkspaceScope::ConversationSource).await;
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:40 "AiWorkspaceScopeRepository > returns undefined when the record no longer exists"
#[tokio::test]
async fn ai_workspace_scope_repository_returns_undefined_when_the_record_no_longer_exists() {
    let db = seeded().await;
    let ws = repo::read(&db, |c| find_workspace_id(c, 777, AiWorkspaceScope::AiFile))
        .await
        .expect("read");
    assert_eq!(ws, None);
}

/// Port of: packages/database/__tests__/ai-workspace-scope-repository.test.ts:51 "AiWorkspaceScopeRepository > declares a model for every scope it exposes"
///
/// The TS table maps each scope to a model; here each scope must name a table that exists.
#[tokio::test]
async fn ai_workspace_scope_repository_declares_a_model_for_every_scope_it_exposes() {
    let db = fresh_db().await;
    let mut names: Vec<&str> = AI_WORKSPACE_SCOPES.iter().map(|s| s.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "aiEmbedding",
            "aiFile",
            "conversationEmbedding",
            "conversationSource"
        ]
    );
    let tables = common::table_names(&db).await;
    for scope in AI_WORKSPACE_SCOPES {
        assert!(
            tables.iter().any(|t| t == scope.table()),
            "{}",
            scope.table()
        );
    }
}
