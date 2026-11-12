//! Port of `packages/database/__tests__/ai-file-embedding-id.test.ts` (P3.2).

use chatbotx_db::repo::ai_file_embedding_id::create_ai_file_embedding_id;

/// Port of: packages/database/__tests__/ai-file-embedding-id.test.ts:8 "createAIFileEmbeddingId > keeps the id of a known chunk stable"
#[test]
fn create_ai_file_embedding_id_keeps_the_id_of_a_known_chunk_stable() {
    assert_eq!(
        create_ai_file_embedding_id("123", 4, "Giá gói Pro: 499.000").to_string(),
        "322835431141440608"
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-id.test.ts:18 "createAIFileEmbeddingId > depends on file, position and content"
#[test]
fn create_ai_file_embedding_id_depends_on_file_position_and_content() {
    let id = create_ai_file_embedding_id("1", 0, "a");
    assert_ne!(create_ai_file_embedding_id("2", 0, "a"), id);
    assert_ne!(create_ai_file_embedding_id("1", 1, "a"), id);
    assert_ne!(create_ai_file_embedding_id("1", 0, "b"), id);
    assert_eq!(create_ai_file_embedding_id("1", 0, "a"), id);
}

/// Port of: packages/database/__tests__/ai-file-embedding-id.test.ts:29 "createAIFileEmbeddingId > is a positive bigint string"
#[test]
fn create_ai_file_embedding_id_is_a_positive_bigint_string() {
    let text = create_ai_file_embedding_id("1", 0, "").to_string();
    assert!(!text.starts_with('0') && !text.is_empty());
    assert!(text.bytes().all(|b| b.is_ascii_digit()), "{text}");
}
