//! Port of `packages/database/__tests__/purge-fk-indexes.test.ts`, the part-2 cases
//! (P2.4 spec section 3.1, rows 1.1-1.8): the `FlowRun` / `FlowVersion` cases of the
//! `normalTargets` `test.each` (`:141`) and the 3 `partitionedTargets` cases (`:164`).
//! The `ContactToTag` case is in `purge_fk_indexes.rs` (P2.3); the `AIConversationEmbedding`,
//! `AIEmbedding`, `LeaderboardParticipant` cases belong to P2.5. The migration-text cases
//! (`:174`, `:186`) are not-applicable (see `conversion/specs/P2.4-test-map.tsv`).
//! Class C: runs against a migrated SQLite file.

mod common;

use common::{fresh_db, index, index_columns};

/// The `:141` body: the index exists, its key starts with the FK columns and is exactly them.
async fn assert_normal_target(table: &str, name: &str, columns: &[&str]) {
    let db = fresh_db().await;
    let columns: Vec<Option<String>> = columns.iter().map(|c| Some((*c).to_owned())).collect();
    index(&db, table, name).await;
    let actual = index_columns(&db, name).await;
    assert_eq!(actual.get(..columns.len()), Some(&columns[..]), "FK prefix");
    assert_eq!(actual, columns);
}

/// The `:164` body: the index exists and its key is exactly the FK columns.
async fn assert_partitioned_target(table: &str, name: &str, columns: &[&str]) {
    let db = fresh_db().await;
    let columns: Vec<Option<String>> = columns.iter().map(|c| Some((*c).to_owned())).collect();
    index(&db, table, name).await;
    assert_eq!(index_columns(&db, name).await, columns);
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > FlowRun declares FlowRun_workspaceId_idx with the exact FK prefix"
#[tokio::test]
async fn flow_run_declares_flow_run_workspace_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target("FlowRun", "FlowRun_workspaceId_idx", &["workspaceId"]).await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > FlowRun declares FlowRun_flowId_idx with the exact FK prefix"
#[tokio::test]
async fn flow_run_declares_flow_run_flow_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target("FlowRun", "FlowRun_flowId_idx", &["flowId"]).await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > FlowRun declares FlowRun_flowVersionId_idx with the exact FK prefix"
#[tokio::test]
async fn flow_run_declares_flow_run_flow_version_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target("FlowRun", "FlowRun_flowVersionId_idx", &["flowVersionId"]).await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > FlowVersion declares FlowVersion_workspaceId_idx with the exact FK prefix"
#[tokio::test]
async fn flow_version_declares_flow_version_workspace_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target(
        "FlowVersion",
        "FlowVersion_workspaceId_idx",
        &["workspaceId"],
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > FlowVersion declares FlowVersion_flowId_idx with the exact FK prefix"
#[tokio::test]
async fn flow_version_declares_flow_version_flow_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target("FlowVersion", "FlowVersion_flowId_idx", &["flowId"]).await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:164 "workspace purge FK indexes > ContactOnBroadcast declares ContactOnBroadcast_conversationId_idx with the exact FK prefix"
///
/// The `partitioned_` prefix keeps the Rust name unique (the TS title pattern is the same as `:141`).
#[tokio::test]
async fn partitioned_contact_on_broadcast_declares_contact_on_broadcast_conversation_id_idx_with_the_exact_fk_prefix()
 {
    assert_partitioned_target(
        "ContactOnBroadcast",
        "ContactOnBroadcast_conversationId_idx",
        &["conversationId"],
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:164 "workspace purge FK indexes > SequenceDispatch declares SequenceDispatch_sequenceId_idx with the exact FK prefix"
#[tokio::test]
async fn partitioned_sequence_dispatch_declares_sequence_dispatch_sequence_id_idx_with_the_exact_fk_prefix()
 {
    assert_partitioned_target(
        "SequenceDispatch",
        "SequenceDispatch_sequenceId_idx",
        &["sequenceId"],
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:164 "workspace purge FK indexes > SequenceDispatch declares SequenceDispatch_stepId_idx with the exact FK prefix"
#[tokio::test]
async fn partitioned_sequence_dispatch_declares_sequence_dispatch_step_id_idx_with_the_exact_fk_prefix()
 {
    assert_partitioned_target(
        "SequenceDispatch",
        "SequenceDispatch_stepId_idx",
        &["stepId"],
    )
    .await;
}
