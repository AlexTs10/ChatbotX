//! `apps/worker/__tests__/process-conversation-source-ensure-stored.test.ts`
//! and `context-source-workspace-scope.test.ts`, plus the producer and
//! embedding paths of the same modules. The originals mock the repositories
//! and the extractor; here the jobs run on the worker fixture with a local
//! file store and a fake embedder. The "not-ready attachment is rethrown"
//! case is n-a: Rust stores attachments on ingest (no lazy hydration).

use std::sync::Arc;

use chatbotx_business::ai::knowledge::{
    EmbedFuture, Embedder, ProviderCallError, ResolvedEmbeddingModel,
};
use chatbotx_db::rusqlite::params;
use chatbotx_files::{Backend, FilesConfig, Uploader};
use serde_json::json;

use super::Ai;
use super::conversation_source::*;
use crate::steps::test_support::Fixture;

/// Answers `[0.5; 1536]`, or the scripted error.
struct FakeEmbedder(Option<ProviderCallError>);

impl Embedder for FakeEmbedder {
    fn embed<'a>(&'a self, _: &'a ResolvedEmbeddingModel, _: &'a str) -> EmbedFuture<'a> {
        let out = self.0.clone().map_or(Ok(vec![0.5; 1536]), Err);
        Box::pin(async move { out })
    }
}

struct Env {
    fx: Fixture,
    root: std::path::PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A workspace with OpenAI connected (the embedding model), a local file
/// store and the attachment 40 (`application/pdf`) on message 30.
async fn env(embed_error: Option<ProviderCallError>) -> Env {
    let mut fx = Fixture::new().await;
    let root = std::env::temp_dir().join(format!("cx-conv-src-{}", chatbotx_db::repo::new_id()));
    std::fs::create_dir_all(&root).unwrap();
    let files = FilesConfig {
        backend: Backend::Local { root: root.clone() },
        signing_secret: None,
        url_base: "http://files.test".into(),
    };
    fx.deps.files = Some(Arc::new(Uploader::new(&files).unwrap()));
    fx.deps.ai = Arc::new(Ai {
        http: Arc::new(crate::ai::llm::ReqwestHttp::default()),
        embedder: Arc::new(FakeEmbedder(embed_error)),
    });
    fx.exec(
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (79, 2, 'openai');
           INSERT INTO "IntegrationOpenai" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
             VALUES (80, '{"authType":"secretText","secretText":"sk-test"}', 'gpt-4o', 1000, 2, 79);
           INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
             VALUES (30, 2, 6, 5, 'contact', 'incoming', 'text', '', 1000);
           INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath", "name", "createdAt")
             VALUES (40, 2, 6, 30, 1000, 'file', 'application/pdf', 'workspaces/2/doc.txt', 'menu.pdf', 1000);"#,
    )
    .await;
    Env { fx, root }
}

/// Stores `bytes` at the attachment's path; the source reads it as `mime`.
async fn store(env: &Env, mime: &str, bytes: &[u8]) {
    std::fs::create_dir_all(env.root.join("workspaces/2")).unwrap();
    std::fs::write(env.root.join("workspaces/2/doc.txt"), bytes).unwrap();
    let mime = mime.to_owned();
    env.fx
        .sql(move |tx| {
            tx.execute(
                r#"UPDATE "Attachment" SET "mimeType" = ?1 WHERE "id" = 40"#,
                [mime],
            )?;
            Ok(())
        })
        .await;
}

async fn source_row(
    fx: &Fixture,
    id: i64,
) -> (String, Option<String>, Option<String>, Option<String>) {
    fx.deps
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT "status", "errorMessage", "summary", "contentHash" FROM "AIConversationSource" WHERE "id" = ?1"#,
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn insert_source(
    fx: &Fixture,
    id: i64,
    source_type: &str,
    status: &str,
    attachment: Option<i64>,
    metadata: Option<&str>,
) {
    let (source_type, status, metadata) = (
        source_type.to_owned(),
        status.to_owned(),
        metadata.map(str::to_owned),
    );
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "AIConversationSource" ("id", "workspaceId", "conversationId", "messageId", "attachmentId", "sourceType", "status", "sourceKey", "metadata")
               VALUES (?1, 2, 6, 30, ?2, ?3, ?4, ?5, ?6)"#,
            params![id, attachment, source_type, status, format!("key-{id}"), metadata],
        )?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn rejects_an_attachment_that_is_not_a_pdf_or_docx() {
    let env = env(None).await;
    store(&env, "text/plain", b"Opening hours: nine to five.").await;
    insert_source(&env.fx, 500, "document", "pending", Some(40), None).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "500" }))
        .await
        .unwrap();
    let (status, error, ..) = source_row(&env.fx, 500).await;
    assert_eq!(
        (status.as_str(), error.as_deref()),
        ("error", Some("Unsupported document mime type"))
    );
}

#[tokio::test]
async fn a_broken_pdf_errors_the_source_with_the_parse_failure() {
    let env = env(None).await;
    store(&env, "application/pdf", b"%PDF-1.4 not really").await;
    insert_source(&env.fx, 500, "document", "pending", Some(40), None).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "500" }))
        .await
        .unwrap();
    let (status, error, ..) = source_row(&env.fx, 500).await;
    assert_eq!(
        (status.as_str(), error.as_deref()),
        ("error", Some("PDF parsing failed"))
    );
    assert!(
        env.fx
            .jobs
            .named(PROCESS_CONVERSATION_SOURCE_EMBEDDING)
            .is_empty()
    );
}

#[tokio::test]
async fn a_stored_pdf_becomes_chunks_a_summary_and_a_hash() {
    let env = env(None).await;
    store(
        &env,
        "application/pdf",
        &minimal_pdf("Opening hours are nine to five every weekday"),
    )
    .await;
    insert_source(&env.fx, 500, "document", "pending", Some(40), None).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "500" }))
        .await
        .unwrap();
    let (status, error, summary, hash) = source_row(&env.fx, 500).await;
    assert_eq!(
        (status.as_str(), error),
        ("success", None),
        "summary: {summary:?}"
    );
    assert!(summary.unwrap().contains("Opening hours"));
    assert_eq!(hash.unwrap().len(), 64);
    let jobs = env.fx.jobs.named(PROCESS_CONVERSATION_SOURCE_EMBEDDING);
    assert_eq!(jobs.len(), 1);
    let chunk_id = jobs[0].data["data"]["conversationEmbeddingId"].clone();

    // processConversationSourceEmbedding
    process_embedding(
        &env.fx.deps,
        &json!({ "conversationEmbeddingId": chunk_id }),
    )
    .await
    .unwrap();
    let status = env
        .fx
        .text(r#"SELECT "status" || ':' || length("embedding") FROM "AIConversationEmbedding""#)
        .await;
    assert_eq!(status.as_deref(), Some("success:6144"));
    // A finished chunk is left alone.
    process_embedding(
        &env.fx.deps,
        &json!({ "conversationEmbeddingId": chunk_id }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn marks_the_source_as_errored_when_the_attachment_is_unavailable() {
    let env = env(None).await;
    // No object at the origin path.
    insert_source(&env.fx, 500, "document", "pending", Some(40), None).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "500" }))
        .await
        .unwrap();
    assert_eq!(source_row(&env.fx, 500).await.0, "error");
    // No attachment row at all.
    insert_source(&env.fx, 501, "document", "pending", None, None).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "501" }))
        .await
        .unwrap();
    assert_eq!(
        source_row(&env.fx, 501).await.1.as_deref(),
        Some("Attachment not found for document source")
    );
    assert!(
        process_source(&env.fx.deps, &json!({ "sourceId": "999" }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_url_source_rejects_a_private_address_and_a_missing_url() {
    let env = env(None).await;
    insert_source(
        &env.fx,
        500,
        "url",
        "pending",
        None,
        Some(r#"{"url":"http://127.0.0.1/admin"}"#),
    )
    .await;
    process_source(&env.fx.deps, &json!({ "sourceId": "500" }))
        .await
        .unwrap();
    let (status, error, ..) = source_row(&env.fx, 500).await;
    assert_eq!(status, "error");
    assert!(error.unwrap().contains("URL source"));
    insert_source(&env.fx, 501, "url", "pending", None, Some("{}")).await;
    process_source(&env.fx.deps, &json!({ "sourceId": "501" }))
        .await
        .unwrap();
    assert_eq!(
        source_row(&env.fx, 501).await.1.as_deref(),
        Some("URL missing in source metadata")
    );
}

#[tokio::test]
async fn an_embedding_provider_error_marks_the_chunk_errored() {
    let env = env(Some(ProviderCallError {
        status: Some(400),
        body: None,
        message: "boom".into(),
    }))
    .await;
    insert_source(&env.fx, 500, "document", "success", Some(40), None).await;
    env.fx
        .exec(
            r#"INSERT INTO "AIConversationEmbedding" ("id", "sourceId", "workspaceId", "conversationId", "chunkIndex", "content")
               VALUES (600, 500, 2, 6, 0, 'hello')"#,
        )
        .await;
    process_embedding(&env.fx.deps, &json!({ "conversationEmbeddingId": "600" }))
        .await
        .unwrap();
    let row = env
        .fx
        .text(r#"SELECT "status" || ':' || "errorMessage" FROM "AIConversationEmbedding" WHERE "id" = 600"#)
        .await;
    assert_eq!(row.as_deref(), Some("error:boom"));
}

#[tokio::test]
async fn resolves_a_document_source_once_and_queues_its_processing_job() {
    let env = env(None).await;
    let source = resolve_document_source(&env.fx.deps, 2, 6, Some(30), Some("MENU"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (source.source_key.as_str(), source.status.as_str()),
        ("attachment:40", "pending")
    );
    assert_eq!(source.title.as_deref(), Some("menu.pdf"));
    let jobs = env.fx.jobs.named(PROCESS_CONVERSATION_SOURCE);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].opts.job_id,
        Some(format!(
            "processConversationSource-{}-{}",
            source.id, source.updated_at
        ))
    );
    assert_eq!(jobs[0].data["data"]["sourceId"], source.id.to_string());
    // Processed: found again, nothing queued.
    let id = source.id;
    env.fx
        .sql(move |tx| {
            tx.execute(
                r#"UPDATE "AIConversationSource" SET "status" = 'success' WHERE "id" = ?1"#,
                [id],
            )?;
            Ok(())
        })
        .await;
    let again = resolve_document_source(&env.fx.deps, 2, 6, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.id, source.id);
    assert_eq!(env.fx.jobs.named(PROCESS_CONVERSATION_SOURCE).len(), 1);
    // No document in another conversation of the workspace.
    assert!(
        resolve_document_source(&env.fx.deps, 2, 999, None, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_errored_document_source_is_set_pending_and_queued_again() {
    let env = env(None).await;
    env.fx
        .exec(
            r#"INSERT INTO "AIConversationSource" ("id", "workspaceId", "conversationId", "messageId", "attachmentId", "sourceType", "status", "sourceKey", "errorMessage")
               VALUES (500, 2, 6, 30, 40, 'document', 'error', 'attachment:40', 'old')"#,
        )
        .await;
    let source = resolve_document_source(&env.fx.deps, 2, 6, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (source.status.as_str(), source.error_message),
        ("pending", None)
    );
    assert_eq!(env.fx.jobs.named(PROCESS_CONVERSATION_SOURCE).len(), 1);
}

#[tokio::test]
async fn resolves_a_url_source_and_drops_the_older_one() {
    let env = env(None).await;
    assert!(
        resolve_url_source(&env.fx.deps, 2, 6, None, Some("https://a.example"))
            .await
            .unwrap()
            .is_none()
    );
    let first = resolve_url_source(&env.fx.deps, 2, 6, Some(30), Some(" https://a.example "))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.source_key, "url:https://a.example");
    assert_eq!(
        first.metadata.unwrap().0,
        json!({ "url": "https://a.example" })
    );
    let second = resolve_url_source(&env.fx.deps, 2, 6, Some(30), Some("https://b.example"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.source_key, "url:https://b.example");
    assert_eq!(
        env.fx
            .int(r#"SELECT count(*) FROM "AIConversationSource""#)
            .await,
        Some(1)
    );
    // No hint: the latest source.
    let latest = resolve_url_source(&env.fx.deps, 2, 6, Some(30), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, second.id);
}

/// `context-source-workspace-scope.test.ts`: listing and vector search only
/// read the chunks of the source's workspace.
#[tokio::test]
async fn chunk_retrieval_is_scoped_to_the_source_workspace() {
    let env = env(None).await;
    insert_source(&env.fx, 500, "document", "success", Some(40), None).await;
    let vector = format!("[{}]", vec!["0.5"; 1536].join(","));
    env.fx
        .sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (9, 'Other', 1)"#,
                [],
            )?;
            for (id, ws, content) in [(600, 2, "mine"), (601, 9, "foreign")] {
                tx.execute(
                    r#"INSERT INTO "AIConversationEmbedding" ("id", "sourceId", "workspaceId", "conversationId", "chunkIndex", "content", "status", "embedding")
                       VALUES (?1, 500, ?2, 6, ?1, ?3, 'success', vector32(?4))"#,
                    params![id, ws, content, vector],
                )?;
            }
            Ok(())
        })
        .await;
    let source = chatbotx_db::repo::read(&env.fx.deps.db, |c| {
        chatbotx_db::repo::find_one::<chatbotx_db::repo::models::AiConversationSource>(
            c,
            r#""id" = 500"#,
            &[],
        )
    })
    .await
    .unwrap()
    .unwrap();
    for query in ["", "opening hours"] {
        let snippets = retrieve_chunks(&env.fx.deps, &source, query, None)
            .await
            .unwrap();
        let contents: Vec<_> = snippets.iter().map(|s| s.content.as_str()).collect();
        assert_eq!(contents, ["mine"], "query {query:?}");
    }
}

/// A one-page PDF whose content stream shows `text`.
fn minimal_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 712 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
