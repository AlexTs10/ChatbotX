//! Port of `packages/database/__tests__/ai-conversation-embedding-repository.test.ts` (P3.2).
//! The original captures the SQL text / the `findMany` where object; the port seeds a decoy
//! chunk of the same source but another workspace and asserts that it is not returned
//! (adapted: observable effect of the workspace filter).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::{self, ai_conversation_embedding as emb};
use chatbotx_db::rusqlite::Connection;
use common::{fresh_db, insert_conversation, seed, seed_workspace};

/// `[x, 0, 0, ...]` (1536 dims) as `vector32` text and as the stored blob.
fn vector(x: f32) -> (String, Vec<u8>) {
    let mut values = vec![0.0_f32; 1536];
    values[0] = x;
    values[1] = 1.0;
    let text = format!(
        "[{}]",
        values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let blob = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    (text, blob)
}

fn chunk(tx: &Connection, id: i64, workspace: i64, index: i64, content: &str) {
    ins!(tx, "AIConversationEmbedding", {
        "id" => id, "sourceId" => 50_i64, "workspaceId" => workspace, "conversationId" => 9001_i64,
        "chunkIndex" => index, "content" => content.to_owned(), "status" => "success",
        "embedding" => vector(0.5).1,
    })
    .expect("chunk");
}

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    let s = seed(&db).await;
    seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, move |tx| {
        insert_conversation(tx, s, 9001, None)?;
        ins!(tx, "AIConversationSource", { "id" => 50_i64, "workspaceId" => s.workspace_id, "conversationId" => 9001_i64, "messageId" => 1_i64, "sourceType" => "document", "sourceKey" => "k" })?;
        chunk(tx, 1, 2001, 0, "own chunk");
        chunk(tx, 2, 2002, 1, "other workspace chunk");
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/ai-conversation-embedding-repository.test.ts:9 "ConversationEmbeddingRepository workspace scoping > vector search filters by workspace as well as source"
#[tokio::test]
async fn conversation_embedding_repository_workspace_scoping_vector_search_filters_by_workspace_as_well_as_source()
 {
    let db = seeded().await;
    let query = vector(0.5).0;
    let rows = repo::read(&db, move |c| emb::vector_search(c, &query, 50, 3, 2001))
        .await
        .expect("search");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].content, "own chunk");
    assert_eq!(rows[0].chunk_index, Some(0));
}

/// Port of: packages/database/__tests__/ai-conversation-embedding-repository.test.ts:35 "ConversationEmbeddingRepository workspace scoping > listing chunks by source filters by workspace"
#[tokio::test]
async fn conversation_embedding_repository_workspace_scoping_listing_chunks_by_source_filters_by_workspace()
 {
    let db = seeded().await;
    let rows = repo::read(&db, |c| emb::find_many_by_source(c, 50, 2001, 5))
        .await
        .expect("list");
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
}
