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
}
