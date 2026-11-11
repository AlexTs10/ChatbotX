//! Port of `packages/database/__tests__/purge-fk-indexes.test.ts`, the part-3 cases
//! (P2.5 spec section 3.1, rows 4.1-4.5): the `AIConversationEmbedding`, `AIEmbedding` and
//! `LeaderboardParticipant` cases of the `normalTargets` `test.each` (`:141`) and the
//! `supersededIndexes` case (`:152`). The other cases are in `purge_fk_indexes.rs` (P2.3)
//! and `purge_fk_indexes_part2.rs` (P2.4). Class C: runs against a migrated SQLite file.

mod common;

use common::{fresh_db, index, index_columns, indexes};

fn owned(columns: &[&str]) -> Vec<Option<String>> {
    columns.iter().map(|c| Some((*c).to_owned())).collect()
}

/// The `:141` body: `replacement` is the `supersededIndexes` entry of `name` (the wider index
/// and its columns), if any. The index that is read exists, its key starts with the FK
/// columns, and it equals the replacement columns (or the FK columns).
async fn assert_normal_target(
    table: &str,
    name: &str,
    columns: &[&str],
    replacement: Option<(&str, &[&str])>,
) {
    let db = fresh_db().await;
    let read = replacement.map_or(name, |(index_name, _)| index_name);
    index(&db, table, read).await;
    let actual = index_columns(&db, read).await;
    let columns = owned(columns);
    assert_eq!(actual.get(..columns.len()), Some(&columns[..]), "FK prefix");
    let expected = replacement.map_or(columns.clone(), |(_, wider)| owned(wider));
    assert_eq!(actual, expected);
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > AIConversationEmbedding declares AIConversationEmbedding_lookup_idx with the exact FK prefix"
#[tokio::test]
async fn ai_conversation_embedding_declares_ai_conversation_embedding_lookup_idx_with_the_exact_fk_prefix()
 {
    assert_normal_target(
        "AIConversationEmbedding",
        "AIConversationEmbedding_lookup_idx",
        &["workspaceId", "conversationId", "sourceId", "status"],
        None,
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > AIEmbedding declares AIEmbedding_aiFileId_idx with the exact FK prefix"
///
/// `AIEmbedding_aiFileId_idx` is in `supersededIndexes`: the test reads the replacement
/// `AIEmbedding_aiFileId_chunkIndex_idx`.
#[tokio::test]
async fn ai_embedding_declares_ai_embedding_ai_file_id_idx_with_the_exact_fk_prefix() {
    assert_normal_target(
        "AIEmbedding",
        "AIEmbedding_aiFileId_idx",
        &["aiFileId"],
        Some((
            "AIEmbedding_aiFileId_chunkIndex_idx",
            &["aiFileId", "chunkIndex"],
        )),
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > LeaderboardParticipant declares LeaderboardParticipant_workspaceId_idx with the exact FK prefix"
#[tokio::test]
async fn leaderboard_participant_declares_leaderboard_participant_workspace_id_idx_with_the_exact_fk_prefix()
 {
    assert_normal_target(
        "LeaderboardParticipant",
        "LeaderboardParticipant_workspaceId_idx",
        &["workspaceId"],
        None,
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:141 "workspace purge FK indexes > LeaderboardParticipant declares LeaderboardParticipant_referrerContactId_idx with the exact FK prefix"
///
/// The index is partial (`WHERE "referrerContactId" IS NOT NULL`); the original does not
/// check the predicate either.
#[tokio::test]
async fn leaderboard_participant_declares_leaderboard_participant_referrer_contact_id_idx_with_the_exact_fk_prefix()
 {
    assert_normal_target(
        "LeaderboardParticipant",
        "LeaderboardParticipant_referrerContactId_idx",
        &["referrerContactId"],
        None,
    )
    .await;
}

/// Port of: packages/database/__tests__/purge-fk-indexes.test.ts:152 "workspace purge FK indexes > AIEmbedding_aiFileId_idx is replaced before it is dropped"
///
/// Replacement (DECISIONS "P2.3 disputed tests"): the original checks the order of
/// `CREATE INDEX CONCURRENTLY` and `DROP INDEX CONCURRENTLY` in migration
/// `20261026060700_add_ai_embedding_chunk_index_idx`, which the consolidated schema does not
/// have (the order half is not-applicable). The port checks the end state: the wider index
/// exists, the old one does not, and the FK `aiFileId` stays covered by a leading index.
#[tokio::test]
async fn ai_embedding_ai_file_id_idx_is_replaced_by_the_wider_index() {
    let db = fresh_db().await;
    let names: Vec<String> = indexes(&db, "AIEmbedding")
        .await
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n == "AIEmbedding_aiFileId_chunkIndex_idx"),
        "the replacement index must exist: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "AIEmbedding_aiFileId_idx"),
        "the replaced index must be gone: {names:?}"
    );
    let mut covered = false;
    for name in &names {
        if index_columns(&db, name).await.first() == Some(&Some("aiFileId".to_owned())) {
            covered = true;
        }
    }
    assert!(covered, "the FK aiFileId must stay covered: {names:?}");
}
