//! Port of `packages/database/__tests__/ai-file-chunking-config.test.ts` (P3.2b, ported:
//! `undefined` is `None`, a stored value is `Some(&json)`; each `test.each` row is one
//! test).

use chatbotx_db::repo::partials::ai::{
    AiFileChunkStrategy, AiFileChunkingConfigStrategy, ChunkCountInput,
    ResolvedAiFileChunkingConfig, estimate_ai_file_chunk_count, parse_ai_file_chunking_config,
    resolve_ai_file_chunking_config,
};
use serde_json::{Value, json};

fn resolve(value: Value) -> ResolvedAiFileChunkingConfig {
    resolve_ai_file_chunking_config(Some(&value))
}

#[test]
fn resolve_ai_file_chunking_config_null_and_undefined_resolve_to_today_s_defaults() {
    let defaults = ResolvedAiFileChunkingConfig {
        strategy: AiFileChunkingConfigStrategy::Auto,
        chunk_size: 1000,
        overlap_size: 200,
        tabular_header_row: 1,
    };
    assert_eq!(resolve(Value::Null), defaults);
    assert_eq!(resolve_ai_file_chunking_config(None), defaults);
}

#[test]
fn resolve_ai_file_chunking_config_keeps_explicit_values() {
    assert_eq!(
        resolve(json!({
            "strategy": "prose",
            "chunkSize": 500,
            "overlapSize": 50,
            "tabularHeaderRow": 3,
        })),
        ResolvedAiFileChunkingConfig {
            strategy: AiFileChunkingConfigStrategy::Prose,
            chunk_size: 500,
            overlap_size: 50,
            tabular_header_row: 3,
        }
    );
}

#[test]
fn resolve_ai_file_chunking_config_clamps_overlap_to_half_of_the_chunk_size_including_the_default()
{
    assert_eq!(
        resolve(json!({ "chunkSize": 300, "overlapSize": 500 })).overlap_size,
        150
    );
    assert_eq!(resolve(json!({ "chunkSize": 200 })).overlap_size, 100);
}

#[test]
fn resolve_ai_file_chunking_config_a_malformed_stored_value_falls_back_to_the_defaults() {
    assert_eq!(resolve(json!({ "chunkSize": "huge" })).chunk_size, 1000);
    assert_eq!(resolve(json!("nope")).overlap_size, 200);
}

fn assert_rejects(value: Value) {
    assert!(parse_ai_file_chunking_config(&value).is_err(), "{value}");
}

#[test]
fn ai_file_chunking_config_rejects_chunk_size_199() {
    assert_rejects(json!({ "chunkSize": 199 }));
}

#[test]
fn ai_file_chunking_config_rejects_chunk_size_2001() {
    assert_rejects(json!({ "chunkSize": 2001 }));
}

#[test]
fn ai_file_chunking_config_rejects_overlap_size_501() {
    assert_rejects(json!({ "overlapSize": 501 }));
}

#[test]
fn ai_file_chunking_config_rejects_overlap_size_minus_1() {
    assert_rejects(json!({ "overlapSize": -1 }));
}

#[test]
fn ai_file_chunking_config_rejects_tabular_header_row_0() {
    assert_rejects(json!({ "tabularHeaderRow": 0 }));
}

#[test]
fn ai_file_chunking_config_rejects_tabular_header_row_21() {
    assert_rejects(json!({ "tabularHeaderRow": 21 }));
}

#[test]
fn ai_file_chunking_config_rejects_strategy_markdown() {
    assert_rejects(json!({ "strategy": "markdown" }));
}

#[test]
fn ai_file_chunking_config_rejects_chunk_size_1000_5() {
    assert_rejects(json!({ "chunkSize": 1000.5 }));
}

#[test]
fn ai_file_chunking_config_rejects_unknown_1() {
    assert_rejects(json!({ "unknown": 1 }));
}

#[test]
fn ai_file_chunking_config_accepts_the_limits_and_an_empty_object() {
    assert!(parse_ai_file_chunking_config(&json!({})).is_ok());
    assert!(parse_ai_file_chunking_config(&json!({ "chunkSize": 200, "overlapSize": 0 })).is_ok());
    assert!(
        parse_ai_file_chunking_config(&json!({ "chunkSize": 2000, "tabularHeaderRow": 20 }))
            .is_ok()
    );
}

fn estimate(strategy: AiFileChunkStrategy, extracted_chars: i64, overlap_size: i64) -> i64 {
    estimate_ai_file_chunk_count(ChunkCountInput {
        strategy,
        extracted_chars,
        chunk_size: 1000,
        overlap_size,
    })
}

#[test]
fn estimate_ai_file_chunk_count_prose_slides_a_window_by_chunk_size_minus_overlap() {
    assert_eq!(estimate(AiFileChunkStrategy::Prose, 2600, 200), 3);
}

#[test]
fn estimate_ai_file_chunk_count_tabular_and_markdown_ignore_overlap() {
    assert_eq!(estimate(AiFileChunkStrategy::Tabular, 2600, 200), 3);
}

#[test]
fn estimate_ai_file_chunk_count_is_0_for_no_text_and_at_least_1_otherwise() {
    assert_eq!(estimate(AiFileChunkStrategy::Prose, 0, 0), 0);
    assert_eq!(estimate(AiFileChunkStrategy::Markdown, 5, 0), 1);
}
