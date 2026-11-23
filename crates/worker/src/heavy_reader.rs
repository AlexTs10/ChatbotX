//! The heavy document reader `extractTextFromFile`
//! (`heavy/handlers/extract-text-from-file.ts` + `fallback-text-utils.ts`):
//! the stored file's text and the snippets most relevant to a query.

use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use crate::ai::extract::{is_document_mime, text_from_bytes};
use crate::ai_file::Limits;
use crate::{Deps, fail, str_of};

const FALLBACK_SNIPPET_LIMIT: usize = 3;
const CHUNK_TARGET_CHARS: usize = 500;

// ponytail: lengths count chars, not UTF-16 units as the original.
fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `chunkLargeSegment`: sentences packed up to 500 chars.
fn chunk_large_segment(segment: String) -> Vec<String> {
    if segment.chars().count() <= CHUNK_TARGET_CHARS {
        return vec![segment];
    }
    // `split(/(?<=[.!?])\s+/)`.
    let chars: Vec<(usize, char)> = segment.char_indices().collect();
    let mut sentences = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < chars.len() {
        if chars[i].1.is_whitespace() && i > 0 && matches!(chars[i - 1].1, '.' | '!' | '?') {
            sentences.push(&segment[start..chars[i].0]);
            while i < chars.len() && chars[i].1.is_whitespace() {
                i += 1;
            }
            start = chars.get(i).map_or(segment.len(), |c| c.0);
            continue;
        }
        i += 1;
    }
    sentences.push(&segment[start..]);
    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in sentences {
        let candidate = if current.is_empty() {
            sentence.to_owned()
        } else {
            format!("{current} {sentence}")
        };
        if candidate.chars().count() <= CHUNK_TARGET_CHARS {
            current = candidate;
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current = cut(sentence, CHUNK_TARGET_CHARS);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// `pickRelevantFallbackSnippets` (contents only): the three segments that
/// hold the most query terms, in text order on a tie.
pub fn pick_relevant_snippets(text: &str, query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    let terms: Vec<&str> = query
        .split_whitespace()
        .filter(|t| t.chars().count() > 2)
        .take(8)
        .collect();
    let segments: Vec<String> = text
        .split("\n\n")
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .flat_map(chunk_large_segment)
        .collect();
    let mut scored: Vec<(usize, usize, &String)> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let lowered = s.to_lowercase();
            (terms.iter().filter(|t| lowered.contains(**t)).count(), i, s)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(FALLBACK_SNIPPET_LIMIT)
        .map(|(_, _, s)| cut(s, 500))
        .collect()
}

/// `extractFallbackTextSnippets`: `{ snippets, truncated }`.
pub(crate) async fn extract_text_from_file(deps: &Deps, data: &Value) -> Result<Value, JobError> {
    let mime = str_of(data, "mimeType")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if !is_document_mime(&mime) {
        return Err(JobError::unrecoverable("Unsupported document type"));
    }
    let path = str_of(data, "originPath").unwrap_or("");
    let limits = Limits::from_env();
    let Some(files) = &deps.files else {
        return Err(fail("File storage is not configured"));
    };
    let too_large = || JobError::unrecoverable("Document is too large for document reader");
    match files.head_object(path).await {
        Ok(head) if head.content_length > limits.max_file_bytes => return Err(too_large()),
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(err = %e, origin_path = path, "[document-reader] headObject failed, falling back to byte check");
        }
    }
    let bytes = files
        .get_object(path)
        .await
        .map_err(|e| fail(e.to_string()))?;
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(too_large());
    }
    let text = text_from_bytes(&bytes, &mime, limits.max_text_chars).map_err(fail)?;
    let truncated = text.chars().count() >= limits.max_text_chars;
    let snippets = pick_relevant_snippets(&text, str_of(data, "query").unwrap_or(""));
    Ok(json!({ "snippets": snippets, "truncated": truncated }))
}

/// `heavy/handlers/analyze-image.ts`: the stored image and the prompt to
/// the reply model of `providerInfo`; `{ analysis }` is the trimmed answer.
pub(crate) async fn analyze_image(deps: &Deps, data: &Value) -> Result<Value, JobError> {
    use base64::Engine as _;

    use crate::ai::llm::{self, Part, Request, Turn};

    let max = crate::heavy::max_image_bytes();
    let too_large = || JobError::unrecoverable("Image is too large for image reader");
    if data.get("sizeBytes").and_then(Value::as_f64).unwrap_or(0.0) > max as f64 {
        return Err(too_large());
    }
    let path = str_of(data, "originPath").unwrap_or("");
    let Some(files) = &deps.files else {
        return Err(fail("File storage is not configured"));
    };
    match files.head_object(path).await {
        Ok(head) if head.content_length > max => return Err(too_large()),
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(err = %e, origin_path = path, "[image-reader] headObject failed, falling back to byte check");
        }
    }
    let image = files
        .get_object(path)
        .await
        .map_err(|e| fail(e.to_string()))?;
    if image.len() as u64 > max {
        return Err(too_large());
    }
    let workspace_id = data
        .get("workspaceId")
        .and_then(crate::id_of)
        .unwrap_or_default();
    let provider_info = data.get("providerInfo").cloned().unwrap_or(Value::Null);
    let model = crate::ai::agent_model(&deps.db, workspace_id, &provider_info)
        .await
        .map_err(fail)?
        .ok_or_else(|| JobError::unrecoverable("Image reader model is not available"))?;
    let mime = str_of(data, "mimeType").unwrap_or("").to_owned();
    let url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&image)
    );
    let mut request = Request::new(
        None,
        vec![Turn::User(vec![
            Part::Text(str_of(data, "prompt").unwrap_or("").to_owned()),
            Part::File {
                url,
                media_type: mime,
            },
        ])],
    );
    request.max_output_tokens = Some(IMAGE_READER_MAX_OUTPUT_TOKENS);
    request.temperature = Some(0.2);
    let generated = llm::generate(&*deps.ai.http, &model, &request, &llm::NoTools)
        .await
        .map_err(|e| fail(e.message))?;
    Ok(json!({ "analysis": generated.text().trim() }))
}

const IMAGE_READER_MAX_OUTPUT_TOKENS: i64 = 800;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_segments_with_the_most_query_terms_in_text_order() {
        let text = "Opening hours are nine to five.\n\nShipping takes three days.\n\nReturns: shipping is free for returns.";
        assert_eq!(
            pick_relevant_snippets(text, "free shipping returns"),
            vec![
                "Returns: shipping is free for returns.",
                "Shipping takes three days.",
                "Opening hours are nine to five.",
            ]
        );
    }

    #[test]
    fn without_query_terms_the_first_three_segments() {
        let text = "a\n\nb\n\nc\n\nd";
        assert_eq!(pick_relevant_snippets(text, "is a"), vec!["a", "b", "c"]);
    }

    #[test]
    fn a_long_paragraph_is_cut_at_sentence_ends() {
        let sentence = format!("{}.", "x".repeat(299));
        let text = format!("{sentence} {sentence}");
        let chunks = chunk_large_segment(text);
        assert_eq!(chunks, vec![sentence.clone(), sentence]);
    }

    /// `heavy-analyze-image.test.ts`, on a local file store and a scripted
    /// provider.
    mod analyze {
        use std::sync::Arc;

        use chatbotx_files::{Backend, FilesConfig, Uploader};

        use super::super::analyze_image;
        use crate::ai::Ai;
        use crate::ai::llm::fake::FakeHttp;
        use crate::steps::test_support::Fixture;
        use serde_json::{Value, json};

        async fn env(answers: Vec<(u16, Value)>) -> (Fixture, Arc<FakeHttp>, std::path::PathBuf) {
            let mut fx = Fixture::new().await;
            let root =
                std::env::temp_dir().join(format!("cx-analyze-{}", chatbotx_db::repo::new_id()));
            std::fs::create_dir_all(root.join("workspace")).unwrap();
            std::fs::write(root.join("workspace/image.jpg"), [1u8, 2, 3]).unwrap();
            let files = FilesConfig {
                backend: Backend::Local { root: root.clone() },
                signing_secret: None,
                url_base: "http://files.test".into(),
            };
            fx.deps.files = Some(Arc::new(Uploader::new(&files).unwrap()));
            let http = Arc::new(FakeHttp::new(answers));
            fx.deps.ai = Arc::new(Ai {
                http: http.clone(),
                embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
            });
            fx.exec(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (70, 2, 'muse');
                   INSERT INTO "IntegrationMuse" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model", "autoReply")
                     VALUES (72, '{"authType":"secretText","secretText":"k"}', 2, 70, 500, 'muse-spark-1.3', 1);"#,
            )
            .await;
            (fx, http, root)
        }

        fn job(size: u64) -> Value {
            json!({ "mimeType": "image/jpeg", "originPath": "workspace/image.jpg", "prompt": "Describe the image",
                    "providerInfo": { "model": "muse-spark-1.3", "provider": "muse" }, "sizeBytes": size, "workspaceId": "2" })
        }

        #[tokio::test]
        async fn sends_the_image_as_a_file_part_with_its_exact_mime_type() {
            let (fx, http, root) = env(vec![FakeHttp::openai_text("  A photo  ")]).await;
            let out = analyze_image(&fx.deps, &job(3)).await.unwrap();
            assert_eq!(out, json!({ "analysis": "A photo" }));
            let body = &http.requests()[0].1;
            assert_eq!(
                body["messages"][0],
                json!({ "role": "user", "content": [
                    { "type": "text", "text": "Describe the image" },
                    { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,AQID" } }] })
            );
            assert_eq!(
                (body["max_tokens"].clone(), body["temperature"].clone()),
                (json!(800 + 1024), json!(0.2)),
                "800 tokens plus the Muse reasoning headroom"
            );
            let _ = std::fs::remove_dir_all(root);
        }

        #[tokio::test]
        async fn a_too_large_image_or_a_missing_model_is_unrecoverable() {
            let (fx, http, root) = env(vec![]).await;
            let err = analyze_image(&fx.deps, &job(u64::MAX >> 1))
                .await
                .unwrap_err();
            assert!(format!("{err:?}").contains("Image is too large for image reader"));
            fx.exec(r#"DELETE FROM "IntegrationMuse""#).await;
            let err = analyze_image(&fx.deps, &job(3)).await.unwrap_err();
            assert!(format!("{err:?}").contains("Image reader model is not available"));
            assert!(http.requests().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
