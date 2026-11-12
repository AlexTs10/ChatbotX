//! Port of `packages/database/__tests__/sharded-message-repository.test.ts` (P3.3).
//! The TS test mocks the shard manager and the Drizzle chains; each port asserts the
//! observable effect on real rows of a migrated SQLite file (one file, no shards). Shard
//! routing, transport-retry and microsecond cases are not-applicable (P3.3-test-map.tsv).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo5.rs"]
mod repo5;

use chatbotx_db::repo::message::*;
use chatbotx_db::repo::{self, now_ms};
use common::fresh_db;
use repo_common::{err_message, ms, ok, setup};
use repo5::*;
use serde_json::{Map, Value, json};

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("object")
}

fn with_id(id: i64) -> CreateMessageInput {
    CreateMessageInput {
        id: Some(id),
        ..make_message()
    }
}

// ---------------------------------------------------------------------------
// ShardedMessageRepository.bulkCreate
// ---------------------------------------------------------------------------

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:90 "ShardedMessageRepository.bulkCreate > conflict target is (contactInboxId, sourceId, createdAt) — TimescaleDB 3-column constraint"
#[tokio::test]
async fn sharded_message_repository_bulk_create_conflict_target_is_contact_inbox_id_source_id_created_at_timescale_db_3_column_constraint()
 {
    let db = fresh_db().await;
    let rows = vec![
        make_message(),
        with_id(2),
        CreateMessageInput {
            id: Some(3),
            created_at: Some(ms("2026-01-01T00:00:01Z")),
            ..make_message()
        },
        CreateMessageInput {
            id: Some(4),
            contact_inbox_id: CI + 1,
            ..make_message()
        },
    ];
    let result = ok(repo::write(&db, move |tx| bulk_create(tx, &rows)).await);
    assert_eq!(
        result.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![1, 3, 4]
    );
    assert_eq!(message_rows(&db).await.len(), 3);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:100 "ShardedMessageRepository.bulkCreate > bulkCreate stamps a missing createdAt itself instead of leaving it to the database"
#[tokio::test]
async fn sharded_message_repository_bulk_create_bulk_create_stamps_a_missing_created_at_itself_instead_of_leaving_it_to_the_database()
 {
    let db = fresh_db().await;
    let before = now_ms();
    let rows = vec![CreateMessageInput {
        created_at: None,
        ..make_message()
    }];
    ok(repo::write(&db, move |tx| bulk_create(tx, &rows)).await);
    let after = now_ms();
    let row = message_row(&db, 1).await.expect("row");
    assert!(row.created_at >= before && row.created_at <= after);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:111 "ShardedMessageRepository.bulkCreate > create stamps a missing createdAt itself, and keeps it on a primary-key retry"
#[tokio::test]
async fn sharded_message_repository_bulk_create_create_stamps_a_missing_created_at_itself_and_keeps_it_on_a_primary_key_retry()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("other"),
                ..MsgRow::new(1, ms("2025-12-01T00:00:00Z"))
            },
        )
    })
    .await;
    let before = now_ms();
    let input = CreateMessageInput {
        created_at: None,
        ..make_message()
    };
    let created = ok(repo::write(&db, move |tx| create(tx, &input)).await);
    let after = now_ms();
    assert_ne!(created.id, 1);
    assert!(created.created_at >= before && created.created_at <= after);
    assert_eq!(message_row(&db, created.id).await, Some(created));
    assert_eq!(message_rows(&db).await.len(), 2);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:131 "ShardedMessageRepository.bulkCreate > returns empty array when no messages provided"
#[tokio::test]
async fn sharded_message_repository_bulk_create_returns_empty_array_when_no_messages_provided() {
    let db = fresh_db().await;
    let result = ok(repo::write(&db, |tx| bulk_create(tx, &[])).await);
    assert!(result.is_empty());
    assert!(message_rows(&db).await.is_empty());
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:138 "ShardedMessageRepository.bulkCreate > inserts messages and returns { id, sourceId }[]"
#[tokio::test]
async fn sharded_message_repository_bulk_create_inserts_messages_and_returns_id_source_id() {
    let db = fresh_db().await;
    let result = ok(repo::write(&db, |tx| bulk_create(tx, &[make_message()])).await);
    assert_eq!(
        result,
        vec![CreatedMessageRef {
            id: 1,
            source_id: Some("src-1".to_owned())
        }]
    );
    assert_eq!(message_rows(&db).await.len(), 1);
}

fn many(n: i64) -> Vec<CreateMessageInput> {
    (0..n)
        .map(|i| CreateMessageInput {
            id: Some(i + 1),
            source_id: Some(format!("src-{i}")),
            ..make_message()
        })
        .collect()
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:147 "ShardedMessageRepository.bulkCreate > chunks correctly — 2001 messages triggers 3 db.insert calls with correct slice sizes"
#[tokio::test]
async fn sharded_message_repository_bulk_create_chunks_correctly_2001_messages_triggers_3_db_insert_calls_with_correct_slice_sizes()
 {
    let db = fresh_db().await;
    let rows = many(2001);
    ok(repo::write(&db, move |tx| bulk_create(tx, &rows)).await);
    let stored = message_rows(&db).await;
    assert_eq!(stored.len(), 2001);
    assert_eq!(stored[0].source_id.as_deref(), Some("src-0"));
    assert_eq!(stored[1000].source_id.as_deref(), Some("src-1000"));
    assert_eq!(stored[2000].source_id.as_deref(), Some("src-2000"));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:164 "ShardedMessageRepository.bulkCreate > aggregates results from multiple chunks"
#[tokio::test]
async fn sharded_message_repository_bulk_create_aggregates_results_from_multiple_chunks() {
    let db = fresh_db().await;
    let rows = many(2001);
    let result = ok(repo::write(&db, move |tx| bulk_create(tx, &rows)).await);
    assert_eq!(result.len(), 2001);
    let r = |id: i64, src: &str| CreatedMessageRef {
        id,
        source_id: Some(src.to_owned()),
    };
    assert_eq!(result[0], r(1, "src-0"));
    assert_eq!(result[1000], r(1001, "src-1000"));
    assert_eq!(result[2000], r(2001, "src-2000"));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:222 "ShardedMessageRepository.bulkCreate > throws when messages span multiple workspaceIds"
#[tokio::test]
async fn sharded_message_repository_bulk_create_throws_when_messages_span_multiple_workspace_ids() {
    let db = fresh_db().await;
    let rows = vec![
        make_message(),
        CreateMessageInput {
            id: Some(2),
            workspace_id: WS_OTHER,
            ..make_message()
        },
    ];
    let message = err_message(repo::write(&db, move |tx| bulk_create(tx, &rows)).await);
    assert_eq!(
        message,
        "bulkCreate: all messages must belong to the same workspace"
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:233 "ShardedMessageRepository.bulkCreate > bulkCreateAttachments routes to shard db and passes messageCreatedAt"
#[tokio::test]
async fn sharded_message_repository_bulk_create_bulk_create_attachments_routes_to_shard_db_and_passes_message_created_at()
 {
    let db = fresh_db().await;
    let msg_created_at = ms("2026-01-01T00:00:00Z");
    let input = make_attachment(Some(11), 1, msg_created_at);
    let result = ok(repo::write(&db, move |tx| bulk_create_attachments(tx, &[input])).await);
    assert_eq!(result, vec![11]);
    let row = attachment_row(&db, 11).await.expect("row");
    assert_eq!(row.message_created_at, msg_created_at);
    assert_eq!(row.workspace_id, WS);
    assert_eq!(row.message_id, 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:258 "ShardedMessageRepository.bulkCreate > bulkCreateAttachments stamps createdAt itself, at the precision it will later match on"
#[tokio::test]
async fn sharded_message_repository_bulk_create_bulk_create_attachments_stamps_created_at_itself_at_the_precision_it_will_later_match_on()
 {
    let db = fresh_db().await;
    let given = ms("2026-01-02T03:04:05.678Z");
    let base = make_attachment(Some(11), 1, ms("2026-01-01T00:00:00Z"));
    let second = CreateAttachmentInput {
        id: Some(12),
        created_at: Some(given),
        ..base.clone()
    };
    let before = now_ms();
    ok(repo::write(&db, move |tx| bulk_create_attachments(tx, &[base, second])).await);
    let after = now_ms();
    let first = attachment_row(&db, 11).await.expect("row 11");
    assert!(first.created_at >= before && first.created_at <= after);
    assert_eq!(
        attachment_row(&db, 12).await.expect("row 12").created_at,
        given
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:284 "ShardedMessageRepository.bulkCreate > bulkCreateAttachments returns empty array for empty input without calling insert"
#[tokio::test]
async fn sharded_message_repository_bulk_create_bulk_create_attachments_returns_empty_array_for_empty_input_without_calling_insert()
 {
    let db = fresh_db().await;
    let result = ok(repo::write(&db, |tx| bulk_create_attachments(tx, &[])).await);
    assert!(result.is_empty());
    assert!(attachment_rows(&db).await.is_empty());
}

// ---------------------------------------------------------------------------
// direct message/attachment lookup helpers
// ---------------------------------------------------------------------------

const SINCE: &str = "2026-01-01T00:00:00Z";

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:343 "ShardedMessageRepository direct message/attachment lookup helpers > findManyBySourceIds requires sinceTime"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_many_by_source_ids_requires_since_time()
 {
    let db = fresh_db().await;
    let params = FindManyBySourceIdsParams {
        contact_inbox_ids: vec![CI],
        source_ids: vec!["src-1".to_owned()],
        workspace_id: WS,
        since_time: None,
    };
    let message = err_message(repo::read(&db, move |c| find_many_by_source_ids(c, &params)).await);
    assert!(message.contains("sinceTime is required"), "{message}");
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:355 "ShardedMessageRepository direct message/attachment lookup helpers > findManyBySourceIds unions the write shard with time-range shards"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_many_by_source_ids_unions_the_write_shard_with_time_range_shards()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                ..MsgRow::new(11, ms("2026-01-02T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                ..MsgRow::new(12, ms("2026-02-01T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                workspace_id: WS_OTHER,
                ..MsgRow::new(13, ms("2026-01-03T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-2"),
                ..MsgRow::new(14, ms("2026-01-02T00:00:00Z"))
            },
        )
    })
    .await;
    let params = FindManyBySourceIdsParams {
        contact_inbox_ids: vec![CI],
        source_ids: vec!["src-1".to_owned()],
        workspace_id: WS,
        since_time: Some(ms(SINCE)),
    };
    let result = ok(repo::read(&db, move |c| find_many_by_source_ids(c, &params)).await);
    assert_eq!(
        result.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![11, 12]
    );
    assert_eq!(result[0].conversation_id, CONV);
    assert_eq!(result[0].contact_inbox_id, CI);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:403 "ShardedMessageRepository direct message/attachment lookup helpers > bulkPatchContentAttributes requires sinceTime and fans updates across read shards"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_bulk_patch_content_attributes_requires_since_time_and_fans_updates_across_read_shards()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                content_attributes: Some(r#"{"kept":1}"#),
                ..MsgRow::new(11, ms("2026-01-02T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                contact_inbox_id: CI + 1,
                ..MsgRow::new(12, ms("2026-01-02T00:00:00Z"))
            },
        )
    })
    .await;
    let params = BulkPatchContentAttributesParams {
        workspace_id: WS,
        since_time: Some(ms(SINCE)),
        patches: vec![ContentAttributesPatch {
            contact_inbox_id: CI,
            source_id: "src-1".to_owned(),
            overlay: obj(json!({ "edited": true })),
            text: Some(Some("edited".to_owned())),
        }],
    };
    let without_since = BulkPatchContentAttributesParams {
        since_time: None,
        ..params.clone()
    };
    ok(repo::write(&db, move |tx| bulk_patch_content_attributes(tx, &params)).await);
    let patched = message_row(&db, 11).await.expect("row 11");
    assert_eq!(patched.text.as_deref(), Some("edited"));
    assert_eq!(
        patched.content_attributes.map(|j| j.0),
        Some(json!({ "kept": 1, "edited": true }))
    );
    let other = message_row(&db, 12).await.expect("row 12");
    assert_eq!(other.text, None);
    assert_eq!(other.content_attributes, None);
    let message = err_message(
        repo::write(&db, move |tx| {
            bulk_patch_content_attributes(tx, &without_since)
        })
        .await,
    );
    assert!(message.contains("sinceTime is required"), "{message}");
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:453 "ShardedMessageRepository direct message/attachment lookup helpers > findAttachmentById returns attachment and parent-message lookup fields"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_attachment_by_id_returns_attachment_and_parent_message_lookup_fields()
 {
    let db = fresh_db().await;
    let at = ms("2026-01-03T00:00:00Z");
    setup(&db, move |tx| {
        put_attachment(tx, 21, 1, at, "wa-media:123", None)
    })
    .await;
    let params = FindAttachmentByIdParams {
        id: 21,
        workspace_id: WS,
        message_created_at: None,
    };
    let other_ws = FindAttachmentByIdParams {
        workspace_id: WS_OTHER,
        ..params.clone()
    };
    let result = ok(repo::read(&db, move |c| find_attachment_by_id(c, &params)).await);
    assert_eq!(
        result,
        Some(AttachmentLookupRow {
            id: 21,
            message_id: 1,
            message_created_at: at,
            source_id: Some("source-attachment-21".to_owned()),
            origin_path: "wa-media:123".to_owned(),
            mime_type: "image/png".to_owned(),
            size: 0,
            created_at: at,
            keep_original: None,
        })
    );
    assert_eq!(
        ok(repo::read(&db, move |c| find_attachment_by_id(c, &other_ws)).await),
        None
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:545 "ShardedMessageRepository direct message/attachment lookup helpers > findAttachmentById falls back to the recent-history scan when the hint window misses"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_attachment_by_id_falls_back_to_the_recent_history_scan_when_the_hint_window_misses()
 {
    let db = fresh_db().await;
    let message_created_at = ms("2026-01-03T12:00:00Z");
    setup(&db, move |tx| {
        put_attachment(tx, 21, 1, message_created_at, "wa-media:123", None)
    })
    .await;
    let params = FindAttachmentByIdParams {
        id: 21,
        workspace_id: WS,
        message_created_at: Some(message_created_at - 30 * 24 * 60 * 60 * 1000),
    };
    let result =
        ok(repo::read(&db, move |c| find_attachment_by_id(c, &params)).await).expect("found");
    assert_eq!(result.id, 21);
    assert_eq!(result.message_created_at, message_created_at);
    assert_eq!(result.origin_path, "wa-media:123");
}

fn attachment_update(created_at: i64, id: i64) -> UpdateAttachmentParams {
    UpdateAttachmentParams {
        id,
        workspace_id: WS,
        created_at,
        fields: UpdateAttachmentFields {
            origin_path: "workspace/a/att-1.png".to_owned(),
            mime_type: "image/png".to_owned(),
            size: 0,
            height: None,
            width: None,
        },
    }
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:722 "ShardedMessageRepository direct message/attachment lookup helpers > updateAttachment fans across read shards and includes createdAt for pruning"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_update_attachment_fans_across_read_shards_and_includes_created_at_for_pruning()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-01-03T00:00:00Z");
    setup(&db, move |tx| {
        put_attachment(tx, 21, 1, created_at, "wa-media:123", None)
    })
    .await;
    let params = attachment_update(created_at, 21);
    ok(repo::write(&db, move |tx| update_attachment(tx, &params)).await);
    let row = attachment_row(&db, 21).await.expect("row");
    assert_eq!(row.origin_path, "workspace/a/att-1.png");
    assert_eq!(row.mime_type, "image/png");
    assert_eq!(row.size, 0);
    assert_eq!(row.created_at, created_at);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:765 "ShardedMessageRepository direct message/attachment lookup helpers > updateAttachment reports how many rows it updated, 0 when none matched"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_update_attachment_reports_how_many_rows_it_updated_0_when_none_matched()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-01-03T00:00:00Z");
    setup(&db, move |tx| {
        put_attachment(tx, 21, 1, created_at, "wa-media:123", None)
    })
    .await;
    let mut params = attachment_update(created_at, 21);
    params.fields = UpdateAttachmentFields {
        origin_path: "public/a.jpg".to_owned(),
        mime_type: "image/jpeg".to_owned(),
        size: 1,
        height: None,
        width: None,
    };
    let missing = UpdateAttachmentParams {
        id: 999,
        ..params.clone()
    };
    assert_eq!(
        ok(repo::write(&db, move |tx| update_attachment(tx, &params)).await),
        1
    );
    assert_eq!(
        ok(repo::write(&db, move |tx| update_attachment(tx, &missing)).await),
        0
    );
}

const RICH_BUTTON_1: &str = r#"{"richResponse":{"executionId":"exec-1","buttonPayloads":{"button-1":{"executionId":"exec-1","buttonId":"button-1","payload":{"type":"text","text":"size_s"}}}}}"#;
const RICH_OTHER: &str = r#"{"richResponse":{"executionId":"exec-1","buttonPayloads":{"other-button":{"executionId":"exec-1","buttonId":"other-button","payload":{"type":"text","text":"other"}}}}}"#;

fn rich_params(message_id: Option<i64>) -> FindRichResponseByButtonParams {
    FindRichResponseByButtonParams {
        button_id: "button-1".to_owned(),
        contact_inbox_id: CI,
        conversation_id: CONV,
        message_id,
        workspace_id: WS,
        since_time: Some(ms(SINCE)),
    }
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:829 "ShardedMessageRepository direct message/attachment lookup helpers > findRichResponseByButton falls back to button lookup when inbound message id has no rich response"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_rich_response_by_button_falls_back_to_button_lookup_when_inbound_message_id_has_no_rich_response()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                content_attributes: Some("{}"),
                ..MsgRow::new(11, ms("2026-06-28T15:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                message_type: "outgoing",
                content_attributes: Some(RICH_BUTTON_1),
                ..MsgRow::new(12, ms("2026-06-28T14:59:00Z"))
            },
        )
    })
    .await;
    let params = rich_params(Some(11));
    let result = ok(repo::read(&db, move |c| find_rich_response_by_button(c, &params)).await);
    let expected: Value = serde_json::from_str(RICH_BUTTON_1).expect("json");
    assert_eq!(result, Some(expected["richResponse"].clone()));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:886 "ShardedMessageRepository direct message/attachment lookup helpers > findRichResponseByButton returns null when message rich response lacks the requested button"
#[tokio::test]
async fn sharded_message_repository_direct_message_attachment_lookup_helpers_find_rich_response_by_button_returns_null_when_message_rich_response_lacks_the_requested_button()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                content_attributes: Some(RICH_OTHER),
                ..MsgRow::new(11, ms("2026-06-28T15:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                message_type: "outgoing",
                content_attributes: Some(RICH_BUTTON_1),
                ..MsgRow::new(12, ms("2026-06-29T00:00:00Z"))
            },
        )
    })
    .await;
    let params = rich_params(Some(11));
    assert_eq!(
        ok(repo::read(&db, move |c| find_rich_response_by_button(c, &params)).await),
        None
    );
}

// ---------------------------------------------------------------------------
// updateSourceId / mergeContentAttributesBySourceId
// ---------------------------------------------------------------------------

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1036 "ShardedMessageRepository.updateSourceId > routes by the message's createdAt shard, not the current write shard"
#[tokio::test]
async fn sharded_message_repository_update_source_id_routes_by_the_message_s_created_at_shard_not_the_current_write_shard()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-01-01T00:00:00Z");
    setup(&db, move |tx| put_message(tx, &MsgRow::new(1, created_at))).await;
    let result = ok(repo::write(&db, move |tx| {
        update_source_id(tx, 1, "prov-abc", WS, created_at)
    })
    .await);
    assert_eq!(result, Some(1));
    assert_eq!(
        message_row(&db, 1).await.expect("row").source_id.as_deref(),
        Some("prov-abc")
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1128 "ShardedMessageRepository.mergeContentAttributesBySourceId > merges the overlay via a single jsonb UPDATE — never overwrites the whole column"
#[tokio::test]
async fn sharded_message_repository_merge_content_attributes_by_source_id_merges_the_overlay_via_a_single_jsonb_update_never_overwrites_the_whole_column()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("wacall-call-1"),
                content_attributes: Some(r#"{"hasTranscript":true}"#),
                ..MsgRow::new(1, ms("2026-06-01T00:00:00Z"))
            },
        )
    })
    .await;
    let overlay = obj(json!({ "hasRecording": true }));
    let result = ok(repo::write(&db, move |tx| {
        merge_content_attributes_by_source_id(tx, "wacall-call-1", WS, &overlay)
    })
    .await);
    assert_eq!(
        result,
        Some(MergedContentAttributes {
            id: 1,
            content_attributes: Some(json!({ "hasRecording": true, "hasTranscript": true })),
        })
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1157 "ShardedMessageRepository.mergeContentAttributesBySourceId > swallows a shard-level failure and returns null when no shard has the row"
#[tokio::test]
async fn sharded_message_repository_merge_content_attributes_by_source_id_swallows_a_shard_level_failure_and_returns_null_when_no_shard_has_the_row()
 {
    let db = fresh_db().await;
    let overlay = obj(json!({ "hasRecording": true }));
    let result = ok(repo::write(&db, move |tx| {
        merge_content_attributes_by_source_id(tx, "wacall-call-1", WS, &overlay)
    })
    .await);
    assert_eq!(result, None);
}

// ---------------------------------------------------------------------------
// listByConversation — write-shard union
// ---------------------------------------------------------------------------

fn list_query(since: &str, cursor_at: &str) -> ListMessagesQuery {
    ListMessagesQuery {
        workspace_id: WS,
        conversation_id: Some(CONV),
        contact_inbox_id: None,
        since_time: Some(ms(since)),
        pagination: Pagination {
            limit: 20,
            cursor: Some(PaginationCursor {
                created_at: ms(cursor_at),
                id: None,
                shard_id: None,
            }),
        },
    }
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1178 "ShardedMessageRepository.listByConversation — write-shard union > returns back-dated message from the write shard even when time-range selection is empty"
#[tokio::test]
async fn sharded_message_repository_list_by_conversation_write_shard_union_returns_back_dated_message_from_the_write_shard_even_when_time_range_selection_is_empty()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                text: Some("old imported message"),
                ..MsgRow::new(11, ms("2026-03-10T00:00:00Z"))
            },
        )
    })
    .await;
    let query = list_query("2026-03-01T00:00:00Z", "2026-03-10T01:00:00Z");
    let result = ok(repo::read(&db, move |c| list_by_conversation(c, &query)).await);
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.data[0].message.id, 11);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1217 "ShardedMessageRepository.listByConversation — write-shard union > does not double-query when the write shard is already in the time-range set"
#[tokio::test]
async fn sharded_message_repository_list_by_conversation_write_shard_union_does_not_double_query_when_the_write_shard_is_already_in_the_time_range_set()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                text: Some("recent"),
                ..MsgRow::new(11, ms("2026-06-05T00:00:00Z"))
            },
        )
    })
    .await;
    let query = list_query("2026-06-01T00:00:00Z", "2026-06-05T01:00:00Z");
    let result = ok(repo::read(&db, move |c| list_by_conversation(c, &query)).await);
    assert_eq!(result.data.len(), 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1255 "ShardedMessageRepository.listByConversation — write-shard union > returns empty when neither time-range nor write shard yields a shard"
#[tokio::test]
async fn sharded_message_repository_list_by_conversation_write_shard_union_returns_empty_when_neither_time_range_nor_write_shard_yields_a_shard()
 {
    let db = fresh_db().await;
    let query = list_query("2026-03-01T00:00:00Z", "2026-03-10T01:00:00Z");
    let result = ok(repo::read(&db, move |c| list_by_conversation(c, &query)).await);
    assert_eq!(
        result,
        PaginatedMessages {
            data: vec![],
            has_partial_results: None,
            next_cursor: None,
        }
    );
}

// ---------------------------------------------------------------------------
// listIncomingTextsByContactInbox
// ---------------------------------------------------------------------------

fn incoming(limit: Option<i64>) -> ListIncomingTextsByContactInboxParams {
    ListIncomingTextsByContactInboxParams {
        contact_inbox_id: CI,
        limit,
        workspace_id: WS,
        since_time: ms("2026-06-01T00:00:00Z"),
    }
}

fn seed_me_loop(tx: &chatbotx_db::rusqlite::Connection) -> Result<usize, chatbotx_db::DbError> {
    put_message(
        tx,
        &MsgRow {
            text: Some("me"),
            ..MsgRow::new(12, ms("2026-06-05T10:00:00Z"))
        },
    )?;
    put_message(
        tx,
        &MsgRow {
            text: Some("loop"),
            ..MsgRow::new(11, ms("2026-06-05T09:00:00Z"))
        },
    )
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1300 "ShardedMessageRepository.listIncomingTextsByContactInbox > does not duplicate messages when the registry reports the same physical shard twice"
#[tokio::test]
async fn sharded_message_repository_list_incoming_texts_by_contact_inbox_does_not_duplicate_messages_when_the_registry_reports_the_same_physical_shard_twice()
 {
    let db = fresh_db().await;
    setup(&db, seed_me_loop).await;
    let params = incoming(Some(200));
    let result = ok(repo::read(&db, move |c| {
        list_incoming_texts_by_contact_inbox(c, &params)
    })
    .await);
    assert_eq!(result, vec!["me".to_owned(), "loop".to_owned()]);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1338 "ShardedMessageRepository.listIncomingTextsByContactInbox > stops after enough rows without querying older shards"
#[tokio::test]
async fn sharded_message_repository_list_incoming_texts_by_contact_inbox_stops_after_enough_rows_without_querying_older_shards()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        seed_me_loop(tx)?;
        put_message(
            tx,
            &MsgRow {
                text: Some("out"),
                message_type: "outgoing",
                ..MsgRow::new(13, ms("2026-06-05T11:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                text: Some("other inbox"),
                contact_inbox_id: CI + 1,
                ..MsgRow::new(14, ms("2026-06-05T11:00:00Z"))
            },
        )
    })
    .await;
    let one = incoming(Some(1));
    let all = incoming(None);
    assert_eq!(
        ok(repo::read(&db, move |c| list_incoming_texts_by_contact_inbox(c, &one)).await),
        vec!["me".to_owned()]
    );
    assert_eq!(
        ok(repo::read(&db, move |c| list_incoming_texts_by_contact_inbox(c, &all)).await),
        vec!["me".to_owned(), "loop".to_owned()]
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1389 "ShardedMessageRepository.listIncomingTextsByContactInbox > reads unbounded exports in sequential chunks of at most 1000 rows per shard"
#[tokio::test]
async fn sharded_message_repository_list_incoming_texts_by_contact_inbox_reads_unbounded_exports_in_sequential_chunks_of_at_most_1000_rows_per_shard()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        let newest = ms("2026-06-05T10:00:00Z");
        for i in 0..1001_i64 {
            let text: &'static str = Box::leak(format!("new-{i}").into_boxed_str());
            put_message(
                tx,
                &MsgRow {
                    text: Some(text),
                    ..MsgRow::new(5000 + i, newest - i * 1000)
                },
            )?;
        }
        put_message(
            tx,
            &MsgRow {
                text: Some("old"),
                ..MsgRow::new(10, ms("2026-06-04T09:00:00Z"))
            },
        )
    })
    .await;
    let params = incoming(None);
    let result = ok(repo::read(&db, move |c| {
        list_incoming_texts_by_contact_inbox(c, &params)
    })
    .await);
    assert_eq!(result.len(), 1002);
    assert_eq!(result[0], "new-0");
    assert_eq!(result[1000], "new-1000");
    assert_eq!(result.last().map(String::as_str), Some("old"));
}

// ---------------------------------------------------------------------------
// hardDeleteAllByContactInbox / findById / complete conversation reads
// ---------------------------------------------------------------------------

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1506 "ShardedMessageRepository.hardDeleteAllByContactInbox > scopes hard delete by contact inbox"
#[tokio::test]
async fn sharded_message_repository_hard_delete_all_by_contact_inbox_scopes_hard_delete_by_contact_inbox()
 {
    let db = fresh_db().await;
    let recent = ms("2026-06-02T00:00:00Z");
    let old = ms("2026-05-01T00:00:00Z");
    setup(&db, move |tx| {
        put_message(tx, &MsgRow::new(11, recent))?;
        put_attachment(tx, 21, 11, recent, "a/one.png", Some("a/one-thumb.png"))?;
        put_message(
            tx,
            &MsgRow {
                contact_inbox_id: CI + 1,
                ..MsgRow::new(12, recent)
            },
        )?;
        put_attachment(tx, 22, 12, recent, "b/two.png", None)?;
        put_message(tx, &MsgRow::new(13, old))?;
        put_attachment(tx, 23, 13, old, "c/old.png", None)
    })
    .await;
    let params = HardDeleteAllByContactInboxParams {
        contact_inbox_id: CI,
        workspace_id: WS,
        since_time: ms("2026-06-01T00:00:00Z"),
    };
    let result =
        ok(repo::write(&db, move |tx| hard_delete_all_by_contact_inbox(tx, &params)).await);
    assert_eq!(
        result.attachment_paths,
        vec!["a/one.png".to_owned(), "a/one-thumb.png".to_owned()]
    );
    assert_eq!(
        message_rows(&db)
            .await
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![12, 13]
    );
    assert_eq!(
        attachment_rows(&db)
            .await
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>(),
        vec![22, 23]
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1555 "ShardedMessageRepository.findById > scopes message lookup by workspaceId"
#[tokio::test]
async fn sharded_message_repository_find_by_id_scopes_message_lookup_by_workspace_id() {
    let db = fresh_db().await;
    let created_at = ms("2026-06-01T00:00:00Z");
    setup(&db, move |tx| {
        put_message(
            tx,
            &MsgRow {
                text: Some("scoped message"),
                ..MsgRow::new(1, created_at)
            },
        )
    })
    .await;
    let params = FindMessageByIdParams {
        id: 1,
        created_at,
        workspace_id: WS,
        conversation_id: None,
    };
    let other = FindMessageByIdParams {
        workspace_id: WS_OTHER,
        ..params.clone()
    };
    let found = ok(repo::read(&db, move |c| find_by_id(c, &params)).await).expect("found");
    assert_eq!(found.message.text.as_deref(), Some("scoped message"));
    assert_eq!(
        ok(repo::read(&db, move |c| find_by_id(c, &other)).await),
        None
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1604 "ShardedMessageRepository complete conversation reads > merges equal timestamps by numeric bigint id"
#[tokio::test]
async fn sharded_message_repository_complete_conversation_reads_merges_equal_timestamps_by_numeric_bigint_id()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-06-01T00:00:00Z");
    setup(&db, move |tx| {
        put_message(tx, &MsgRow::new(9, created_at))?;
        put_message(tx, &MsgRow::new(10, created_at))
    })
    .await;
    let options = FindManyByConversationOptions {
        limit: 10,
        since_time: Some(ms(SINCE)),
        workspace_id: Some(WS),
        ..FindManyByConversationOptions::default()
    };
    let result = ok(repo::read(&db, move |c| find_many_by_conversation(c, CONV, &options)).await);
    assert_eq!(result.iter().map(|m| m.id).collect::<Vec<_>>(), vec![10, 9]);
}

// ---------------------------------------------------------------------------
// findAIContextMessages
// ---------------------------------------------------------------------------

fn ai_options() -> FindAiContextMessagesOptions {
    FindAiContextMessagesOptions {
        conversation_id: CONV,
        workspace_id: WS,
        marker_message_id: Some(8),
        limit: 100,
        since_time: Some(ms(SINCE)),
        ..FindAiContextMessagesOptions::default()
    }
}

fn put_marker(
    tx: &chatbotx_db::rusqlite::Connection,
    conversation_id: i64,
) -> Result<usize, chatbotx_db::DbError> {
    put_message(
        tx,
        &MsgRow {
            text: Some("marker"),
            conversation_id,
            ..MsgRow::new(8, ms("2026-05-30T00:00:00Z"))
        },
    )
}

fn put_newer(tx: &chatbotx_db::rusqlite::Connection) -> Result<usize, chatbotx_db::DbError> {
    put_message(
        tx,
        &MsgRow {
            text: Some("newest"),
            ..MsgRow::new(10, ms("2026-06-01T00:00:00Z"))
        },
    )?;
    put_message(
        tx,
        &MsgRow {
            text: Some("new"),
            ..MsgRow::new(9, ms("2026-05-31T00:00:00Z"))
        },
    )
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1737 "ShardedMessageRepository.findAIContextMessages > finds the marker before limiting messages and returns chronological history"
#[tokio::test]
async fn sharded_message_repository_find_ai_context_messages_finds_the_marker_before_limiting_messages_and_returns_chronological_history()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                text: Some("older"),
                ..MsgRow::new(7, ms("2026-05-29T00:00:00Z"))
            },
        )?;
        put_marker(tx, CONV)?;
        put_newer(tx)
    })
    .await;
    let options = ai_options();
    let result = ok(repo::read(&db, move |c| find_ai_context_messages(c, &options)).await);
    assert_eq!(result.iter().map(|m| m.id).collect::<Vec<_>>(), vec![9, 10]);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1758 "ShardedMessageRepository.findAIContextMessages > returns empty history when the marker is latest"
#[tokio::test]
async fn sharded_message_repository_find_ai_context_messages_returns_empty_history_when_the_marker_is_latest()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                text: Some("older"),
                ..MsgRow::new(7, ms("2026-05-29T00:00:00Z"))
            },
        )?;
        put_marker(tx, CONV)
    })
    .await;
    let options = ai_options();
    assert!(ok(repo::read(&db, move |c| find_ai_context_messages(c, &options)).await).is_empty());
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1764 "ShardedMessageRepository.findAIContextMessages > falls back to latest history when the marker is outside the shard window"
#[tokio::test]
async fn sharded_message_repository_find_ai_context_messages_falls_back_to_latest_history_when_the_marker_is_outside_the_shard_window()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_marker(tx, CONV + 1)?;
        put_newer(tx)
    })
    .await;
    let options = ai_options();
    let result = ok(repo::read(&db, move |c| find_ai_context_messages(c, &options)).await);
    assert_eq!(result.iter().map(|m| m.id).collect::<Vec<_>>(), vec![9, 10]);
}

// ---------------------------------------------------------------------------
// createOrUpdate / createOrUpdateWithAttachments — idempotent echo save
// ---------------------------------------------------------------------------

const SENT_AT: &str = "2026-08-04T08:19:29.777Z";
const ECHO_AT: &str = "2026-08-04T08:19:31.191Z";

fn put_prior(tx: &chatbotx_db::rusqlite::Connection) -> Result<usize, chatbotx_db::DbError> {
    put_message(
        tx,
        &MsgRow {
            source_id: Some("mid-2"),
            message_type: "outgoing",
            ..MsgRow::new(5, ms(SENT_AT))
        },
    )
}

fn echo() -> CreateMessageInput {
    CreateMessageInput {
        id: Some(6),
        source_id: Some("mid-2".to_owned()),
        message_type: "outgoing".to_owned(),
        created_at: Some(ms(ECHO_AT)),
        ..make_message()
    }
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1834 "ShardedMessageRepository.createOrUpdate — idempotent echo save > inserts a new message and returns isNew=true"
#[tokio::test]
async fn sharded_message_repository_create_or_update_idempotent_echo_save_inserts_a_new_message_and_returns_is_new_true()
 {
    let db = fresh_db().await;
    let result = ok(repo::write(&db, |tx| create_or_update(tx, &make_message())).await);
    assert!(result.is_new);
    assert_eq!(message_row(&db, 1).await, Some(result.message));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1853 "ShardedMessageRepository.createOrUpdate — idempotent echo save > duplicate echo (insert conflict) returns the existing row, isNew=false, without throwing"
#[tokio::test]
async fn sharded_message_repository_create_or_update_idempotent_echo_save_duplicate_echo_insert_conflict_returns_the_existing_row_is_new_false_without_throwing()
 {
    let db = fresh_db().await;
    let first = ok(repo::write(&db, |tx| create_or_update(tx, &make_message())).await);
    let second = ok(repo::write(&db, |tx| create_or_update(tx, &with_id(2))).await);
    assert!(!second.is_new);
    assert_eq!(second.message, first.message);
    assert_eq!(message_rows(&db).await.len(), 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1875 "ShardedMessageRepository.createOrUpdate — idempotent echo save > conflict where the replica lags is resolved from the write shard (primary)"
#[tokio::test]
async fn sharded_message_repository_create_or_update_idempotent_echo_save_conflict_where_the_replica_lags_is_resolved_from_the_write_shard_primary()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                conversation_id: CONV + 1,
                ..MsgRow::new(5, ms("2026-01-01T00:00:00Z"))
            },
        )
    })
    .await;
    let result = ok(repo::write(&db, |tx| create_or_update(tx, &make_message())).await);
    assert!(!result.is_new);
    assert_eq!(Some(result.message), message_row(&db, 5).await);
    assert_eq!(message_rows(&db).await.len(), 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1918 "ShardedMessageRepository.createOrUpdate — idempotent echo save > dedup guard read looks back past the message's own createdAt so an echo of a just-sent message is caught"
#[tokio::test]
async fn sharded_message_repository_create_or_update_idempotent_echo_save_dedup_guard_read_looks_back_past_the_message_s_own_created_at_so_an_echo_of_a_just_sent_message_is_caught()
 {
    let db = fresh_db().await;
    setup(&db, put_prior).await;
    let result = ok(repo::write(&db, |tx| create_or_update(tx, &echo())).await);
    assert!(!result.is_new);
    assert_eq!(Some(result.message), message_row(&db, 5).await);
    assert_eq!(message_rows(&db).await.len(), 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:1958 "ShardedMessageRepository.createWithAttachments — attachment timestamps > stamps each attachment's createdAt itself instead of leaving it to the database"
#[tokio::test]
async fn sharded_message_repository_create_with_attachments_attachment_timestamps_stamps_each_attachment_s_created_at_itself_instead_of_leaving_it_to_the_database()
 {
    let db = fresh_db().await;
    let mut attachment = make_attachment(None, 0, 0);
    attachment.mime_type = "image/jpeg".to_owned();
    attachment.origin_path = "https://scontent.xx.fbcdn.net/a.jpg".to_owned();
    let before = now_ms();
    let result = ok(repo::write(&db, move |tx| {
        create_with_attachments(tx, &make_message(), &[attachment])
    })
    .await);
    let after = now_ms();
    let stored = &result.attachments[0];
    assert!(stored.created_at >= before && stored.created_at <= after);
    assert_eq!(stored.message_created_at, result.message.created_at);
    assert_eq!(stored.message_id, result.message.id);
    assert_eq!(attachment_row(&db, stored.id).await.as_ref(), Some(stored));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2009 "ShardedMessageRepository.createOrUpdateWithAttachments — idempotent echo save > duplicate echo with attachments returns existing row, isNew=false, without throwing"
#[tokio::test]
async fn sharded_message_repository_create_or_update_with_attachments_idempotent_echo_save_duplicate_echo_with_attachments_returns_existing_row_is_new_false_without_throwing()
 {
    let db = fresh_db().await;
    let attachment = make_attachment(Some(21), 0, 0);
    let first = ok(repo::write(&db, move |tx| {
        create_or_update_with_attachments(tx, &make_message(), &[attachment])
    })
    .await);
    assert!(first.is_new);
    let second = ok(repo::write(&db, |tx| {
        create_or_update_with_attachments(tx, &with_id(2), &[])
    })
    .await);
    assert!(!second.is_new);
    assert_eq!(second.result, first.result);
    assert_eq!(message_rows(&db).await.len(), 1);
    assert_eq!(attachment_rows(&db).await.len(), 1);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2047 "ShardedMessageRepository.createOrUpdateWithAttachments — idempotent echo save > dedup guard read looks back past the message's own createdAt so an echo of a just-sent message is caught"
#[tokio::test]
async fn sharded_message_repository_create_or_update_with_attachments_idempotent_echo_save_dedup_guard_read_looks_back_past_the_message_s_own_created_at_so_an_echo_of_a_just_sent_message_is_caught()
 {
    let db = fresh_db().await;
    setup(&db, put_prior).await;
    let result = ok(repo::write(&db, |tx| {
        create_or_update_with_attachments(tx, &echo(), &[])
    })
    .await);
    assert!(!result.is_new);
    assert_eq!(result.result.message.id, 5);
    assert!(result.result.attachments.is_empty());
    assert_eq!(message_rows(&db).await.len(), 1);
}

// ---------------------------------------------------------------------------
// deleteBySourceId
// ---------------------------------------------------------------------------

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2100 "ShardedMessageRepository.deleteBySourceId > falls back to a full-history shard scan when the createdAt hint misses, and re-searches children using the row's real createdAt"
#[tokio::test]
async fn sharded_message_repository_delete_by_source_id_falls_back_to_a_full_history_shard_scan_when_the_created_at_hint_misses_and_re_searches_children_using_the_row_s_real_created_at()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-old"),
                ..MsgRow::new(11, ms("2026-01-01T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                parent_id: Some("11".to_owned()),
                ..MsgRow::new(12, ms("2026-01-02T00:00:00Z"))
            },
        )?;
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-other"),
                ..MsgRow::new(13, ms("2026-01-01T00:00:00Z"))
            },
        )
    })
    .await;
    let stale_hint = ms("2026-06-15T00:00:00Z");
    let mut result = ok(repo::write(&db, move |tx| {
        delete_by_source_id(tx, "src-old", WS, stale_hint)
    })
    .await);
    result.sort_by_key(|r| r.id);
    let r = |id| MessageRef {
        conversation_id: CONV,
        id,
    };
    assert_eq!(result, vec![r(11), r(12)]);
    let rows = message_rows(&db).await;
    assert!(rows[0].deleted_at.is_some() && rows[1].deleted_at.is_some());
    assert_eq!(rows[2].deleted_at, None);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2169 "ShardedMessageRepository.deleteBySourceId > does not fall back when the narrow createdAt-hinted search already finds the row"
#[tokio::test]
async fn sharded_message_repository_delete_by_source_id_does_not_fall_back_when_the_narrow_created_at_hinted_search_already_finds_the_row()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-06-01T00:00:00Z");
    setup(&db, move |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                ..MsgRow::new(1, created_at)
            },
        )
    })
    .await;
    let result = ok(repo::write(&db, move |tx| {
        delete_by_source_id(tx, "src-1", WS, created_at)
    })
    .await);
    assert_eq!(result.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
    assert!(message_row(&db, 1).await.expect("row").deleted_at.is_some());
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2204 "ShardedMessageRepository.deleteBySourceId > returns conversation IDs and deduplicates a row reached through multiple shards"
#[tokio::test]
async fn sharded_message_repository_delete_by_source_id_returns_conversation_i_ds_and_deduplicates_a_row_reached_through_multiple_shards()
 {
    let db = fresh_db().await;
    let created_at = ms("2026-06-01T00:00:00Z");
    setup(&db, move |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                ..MsgRow::new(1, created_at)
            },
        )
    })
    .await;
    let result = ok(repo::write(&db, move |tx| {
        delete_by_source_id(tx, "src-1", WS, created_at)
    })
    .await);
    assert_eq!(
        result,
        vec![MessageRef {
            conversation_id: CONV,
            id: 1
        }]
    );
}

// ---------------------------------------------------------------------------
// Primary-key collision safety net
// ---------------------------------------------------------------------------

const COLLIDING: i64 = 11_708_385_501_429_760;

fn put_colliding(tx: &chatbotx_db::rusqlite::Connection) -> Result<usize, chatbotx_db::DbError> {
    put_message(
        tx,
        &MsgRow {
            source_id: Some("other"),
            ..MsgRow::new(COLLIDING, ms("2025-12-01T00:00:00Z"))
        },
    )
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2272 "ShardedMessageRepository primary-key collision safety net > createOrUpdate regenerates the id and retries once when the insert hits the primary key"
#[tokio::test]
async fn sharded_message_repository_primary_key_collision_safety_net_create_or_update_regenerates_the_id_and_retries_once_when_the_insert_hits_the_primary_key()
 {
    let db = fresh_db().await;
    setup(&db, put_colliding).await;
    let message = with_id(COLLIDING);
    let input = message.clone();
    let result = ok(repo::write(&db, move |tx| create_or_update(tx, &input)).await);
    assert!(result.is_new);
    assert_ne!(result.message.id, COLLIDING);
    assert!(result.message.id > 0);
    assert_eq!(Some(result.message.created_at), message.created_at);
    assert_eq!(result.message.source_id, message.source_id);
    assert_eq!(
        message_row(&db, result.message.id).await,
        Some(result.message)
    );
    assert_eq!(message_rows(&db).await.len(), 2);
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2346 "ShardedMessageRepository primary-key collision safety net > create regenerates the id and retries once on a primary-key collision"
#[tokio::test]
async fn sharded_message_repository_primary_key_collision_safety_net_create_regenerates_the_id_and_retries_once_on_a_primary_key_collision()
 {
    let db = fresh_db().await;
    setup(&db, put_colliding).await;
    let result = ok(repo::write(&db, |tx| create(tx, &with_id(COLLIDING))).await);
    assert_ne!(result.id, COLLIDING);
    assert_eq!(message_row(&db, result.id).await, Some(result));
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2370 "ShardedMessageRepository primary-key collision safety net > createOrUpdateWithAttachments regenerates the id and re-runs the transaction on a primary-key collision"
#[tokio::test]
async fn sharded_message_repository_primary_key_collision_safety_net_create_or_update_with_attachments_regenerates_the_id_and_re_runs_the_transaction_on_a_primary_key_collision()
 {
    let db = fresh_db().await;
    setup(&db, put_colliding).await;
    let attachment = make_attachment(Some(21), 0, 0);
    let result = ok(repo::write(&db, move |tx| {
        create_or_update_with_attachments(tx, &with_id(COLLIDING), &[attachment])
    })
    .await);
    assert!(result.is_new);
    let id = result.result.message.id;
    assert_ne!(id, COLLIDING);
    assert_eq!(
        Some(result.result.message.created_at),
        make_message().created_at
    );
    assert_eq!(result.result.attachments[0].message_id, id);
    assert_eq!(
        attachment_row(&db, 21)
            .await
            .expect("attachment")
            .message_id,
        id
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2425 "ShardedMessageRepository.createWithAttachments primary-key collision safety net > regenerates the id and re-runs the transaction once on a primary-key collision"
#[tokio::test]
async fn sharded_message_repository_create_with_attachments_primary_key_collision_safety_net_regenerates_the_id_and_re_runs_the_transaction_once_on_a_primary_key_collision()
 {
    let db = fresh_db().await;
    setup(&db, put_colliding).await;
    let message = CreateMessageInput {
        source_id: None,
        ..with_id(COLLIDING)
    };
    let attachment = make_attachment(Some(21), 0, 0);
    let result = ok(repo::write(&db, move |tx| {
        create_with_attachments(tx, &message, &[attachment])
    })
    .await);
    let id = result.message.id;
    assert_ne!(id, COLLIDING);
    assert_eq!(Some(result.message.created_at), make_message().created_at);
    assert_eq!(result.attachments[0].message_id, id);
    assert_eq!(
        attachment_row(&db, 21)
            .await
            .expect("attachment")
            .message_id,
        id
    );
}

/// Port of: packages/database/__tests__/sharded-message-repository.test.ts:2473 "ShardedMessageRepository.createWithAttachments primary-key collision safety net > surfaces any other insert error unchanged without re-running the transaction"
#[tokio::test]
async fn sharded_message_repository_create_with_attachments_primary_key_collision_safety_net_surfaces_any_other_insert_error_unchanged_without_re_running_the_transaction()
 {
    let db = fresh_db().await;
    setup(&db, |tx| {
        put_message(
            tx,
            &MsgRow {
                source_id: Some("src-1"),
                ..MsgRow::new(5, ms("2026-01-01T00:00:00Z"))
            },
        )
    })
    .await;
    let attachment = make_attachment(Some(21), 0, 0);
    let result = repo::write(&db, move |tx| {
        create_with_attachments(tx, &with_id(6), &[attachment])
    })
    .await;
    let error = result.expect_err("dedup violation");
    assert!(
        error.as_db().is_some_and(|e| e.is_unique_violation()),
        "{error}"
    );
    assert!(!is_message_primary_key_violation(&error));
    assert_eq!(
        message_rows(&db)
            .await
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![5]
    );
    assert!(attachment_rows(&db).await.is_empty());
}
