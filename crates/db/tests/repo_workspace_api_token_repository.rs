//! Port of `packages/database/__tests__/workspace-api-token-repository.test.ts` (P3.3).
//! The mocked `findFirst` / `findMany` / `$count` / `delete` / `insert` / `update` chains
//! become real `WorkspaceApiToken` rows in a migrated SQLite file: the asserted `where`
//! and `values` arguments are checked as the rows found, written or left alone. The
//! `logger.warn` spy is the process-wide log recorder of `tests/common/log.rs`.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/log.rs"]
mod log_common;

use chatbotx_db::repo::models::WorkspaceApiToken;
use chatbotx_db::repo::workspace_api_token::{
    InsertDefaultWorkspaceApiTokenInput, InsertWorkspaceApiTokenInput, count_by_workspace_id,
    delete_by_id_for_workspace, find_by_token_hash, find_default_by_workspace_id, insert,
    insert_default, list_by_workspace_id, set_encrypted_token,
};
use chatbotx_db::repo::{self, Json};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{int, ok, scalar_i64, scalar_text, setup};
use serde_json::json;

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;

/// Workspaces 2001 and 2002, no tokens.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER_WS).await;
    db
}

/// Inserts one token row (`scopes` = JSON text or NULL).
async fn token(
    db: &common::TestDb,
    id: i64,
    workspace_id: i64,
    hash: &str,
    created_at: i64,
    scopes: Option<&str>,
    is_default: bool,
) {
    let hash = hash.to_owned();
    let scopes = scopes.map(str::to_owned);
    setup(db, move |tx| {
        ins!(tx, "WorkspaceApiToken", {
            "id" => id,
            "createdAt" => created_at,
            "workspaceId" => workspace_id,
            "name" => format!("token {id}"),
            "permission" => "full",
            "tokenHash" => hash,
            "isDefault" => is_default,
            "scopes" => scopes,
        })?;
        Ok(())
    })
    .await;
}

fn scopes_of(row: &WorkspaceApiToken) -> Option<serde_json::Value> {
    row.scopes.as_ref().map(|j| j.0.clone())
}

fn encrypted() -> Json {
    Json(json!({"v": 1, "iv": "iv", "text": "ct", "tag": "tag"}))
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:48 "workspaceApiTokenRepository.findByTokenHash > returns the row when a token hash matches"
#[tokio::test]
async fn workspace_api_token_repository_find_by_token_hash_returns_the_row_when_a_token_hash_matches()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    token(&db, 2, WS, "hash-2", 2_000, None, false).await;
    let row = ok(repo::read(&db, |c| find_by_token_hash(c, "hash-1")).await).expect("row");
    assert_eq!(row.id, 1);
    assert_eq!(row.workspace_id, WS);
    assert_eq!(row.scopes, None);
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:59 "workspaceApiTokenRepository.findByTokenHash > preserves distinct channel and integration scopes"
#[tokio::test]
async fn workspace_api_token_repository_find_by_token_hash_preserves_distinct_channel_and_integration_scopes()
 {
    let db = seeded().await;
    token(
        &db,
        1,
        WS,
        "hash-1",
        1_000,
        Some(r#"["contacts","channels","integrations"]"#),
        false,
    )
    .await;
    let row = ok(repo::read(&db, |c| find_by_token_hash(c, "hash-1")).await).expect("row");
    assert_eq!(
        scopes_of(&row),
        Some(json!(["contacts", "channels", "integrations"]))
    );
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:74 "workspaceApiTokenRepository.findByTokenHash > drops an unknown stored scope instead of rejecting bearer authentication"
#[tokio::test]
async fn workspace_api_token_repository_find_by_token_hash_drops_an_unknown_stored_scope_instead_of_rejecting_bearer_authentication()
 {
    log_common::install();
    let db = seeded().await;
    token(
        &db,
        1,
        WS,
        "hash-1",
        1_000,
        Some(r#"["contacts","future_scope"]"#),
        false,
    )
    .await;
    let row = ok(repo::read(&db, |c| find_by_token_hash(c, "hash-1")).await).expect("row");
    assert_eq!(scopes_of(&row), Some(json!(["contacts"])));
    let warned = log_common::find(
        tracing::Level::WARN,
        "Dropping unrecognized workspace API token scopes",
    );
    assert!(!warned.is_empty(), "{:?}", log_common::entries());
    assert!(warned.iter().all(|e| e.fields.contains_key("err")));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:91 "workspaceApiTokenRepository.findByTokenHash > returns null when no row matches"
#[tokio::test]
async fn workspace_api_token_repository_find_by_token_hash_returns_null_when_no_row_matches() {
    let db = seeded().await;
    token(&db, 1, WS, "hash-other", 1_000, None, false).await;
    let row = ok(repo::read(&db, |c| find_by_token_hash(c, "hash-1")).await);
    assert_eq!(row, None);
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:102 "workspaceApiTokenRepository.listByWorkspaceId > lists tokens ordered by createdAt desc"
#[tokio::test]
async fn workspace_api_token_repository_list_by_workspace_id_lists_tokens_ordered_by_created_at_desc()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    token(&db, 2, WS, "hash-2", 2_000, None, false).await;
    token(&db, 3, OTHER_WS, "hash-3", 3_000, None, false).await;
    let rows = ok(repo::read(&db, |c| list_by_workspace_id(c, WS)).await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 1]);
    assert!(rows.iter().all(|r| r.scopes.is_none()));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:119 "workspaceApiTokenRepository.listByWorkspaceId > drops unknown scopes without preventing a workspace token list"
#[tokio::test]
async fn workspace_api_token_repository_list_by_workspace_id_drops_unknown_scopes_without_preventing_a_workspace_token_list()
 {
    let db = seeded().await;
    token(
        &db,
        1,
        WS,
        "hash-1",
        1_000,
        Some(r#"["channels","future_scope"]"#),
        false,
    )
    .await;
    let rows = ok(repo::read(&db, |c| list_by_workspace_id(c, WS)).await);
    assert_eq!(rows.len(), 1);
    assert_eq!(scopes_of(&rows[0]), Some(json!(["channels"])));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:130 "workspaceApiTokenRepository.listByWorkspaceId > preserves separately scoped stored rows"
#[tokio::test]
async fn workspace_api_token_repository_list_by_workspace_id_preserves_separately_scoped_stored_rows()
 {
    let db = seeded().await;
    token(
        &db,
        1,
        WS,
        "hash-1",
        1_000,
        Some(r#"["channels","integrations"]"#),
        false,
    )
    .await;
    let rows = ok(repo::read(&db, |c| list_by_workspace_id(c, WS)).await);
    assert_eq!(
        scopes_of(&rows[0]),
        Some(json!(["channels", "integrations"]))
    );
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:143 "workspaceApiTokenRepository.countByWorkspaceId > counts rows scoped to the workspace"
#[tokio::test]
async fn workspace_api_token_repository_count_by_workspace_id_counts_rows_scoped_to_the_workspace()
{
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    token(&db, 2, WS, "hash-2", 2_000, None, false).await;
    token(&db, 3, WS, "hash-3", 3_000, None, true).await;
    token(&db, 4, OTHER_WS, "hash-4", 4_000, None, false).await;
    let count = ok(repo::read(&db, |c| count_by_workspace_id(c, WS)).await);
    assert_eq!(count, 3);
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:167 "workspaceApiTokenRepository.deleteByIdForWorkspace > returns true and deletes when id and workspaceId both match"
#[tokio::test]
async fn workspace_api_token_repository_delete_by_id_for_workspace_returns_true_and_deletes_when_id_and_workspace_id_both_match()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    token(&db, 2, WS, "hash-2", 2_000, None, false).await;
    let deleted = ok(repo::write(&db, |tx| delete_by_id_for_workspace(tx, 1, WS)).await);
    assert!(deleted);
    let count = scalar_i64(&db, "SELECT count(*) FROM \"WorkspaceApiToken\"", vec![]).await;
    assert_eq!(count, Some(1));
    let left = scalar_i64(&db, "SELECT \"id\" FROM \"WorkspaceApiToken\"", vec![]).await;
    assert_eq!(left, Some(2));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:186 "workspaceApiTokenRepository.deleteByIdForWorkspace > returns false when the id belongs to a different workspace"
#[tokio::test]
async fn workspace_api_token_repository_delete_by_id_for_workspace_returns_false_when_the_id_belongs_to_a_different_workspace()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    let deleted = ok(repo::write(&db, |tx| delete_by_id_for_workspace(tx, 1, OTHER_WS)).await);
    assert!(!deleted);
    let count = scalar_i64(&db, "SELECT count(*) FROM \"WorkspaceApiToken\"", vec![]).await;
    assert_eq!(count, Some(1));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:202 "workspaceApiTokenRepository.insert > inserts and returns the new row with name/permission/tokenPrefix"
#[tokio::test]
async fn workspace_api_token_repository_insert_inserts_and_returns_the_new_row_with_name_permission_token_prefix()
 {
    let db = seeded().await;
    let row = ok(repo::write(&db, |tx| {
        insert(
            tx,
            &InsertWorkspaceApiTokenInput {
                workspace_id: WS,
                token_hash: "hash-1".to_owned(),
                name: "My token".to_owned(),
                permission: "full".to_owned(),
                token_prefix: "cbx_ws_abcd".to_owned(),
                scopes: None,
            },
        )
    })
    .await);
    assert_eq!(row.workspace_id, WS);
    assert_eq!(row.token_hash, "hash-1");
    assert_eq!(row.name, "My token");
    assert_eq!(row.permission, "full");
    assert_eq!(row.token_prefix.as_deref(), Some("cbx_ws_abcd"));
    assert_eq!(row.scopes, None);
    assert!(!row.is_default);
    let stored = scalar_text(
        &db,
        "SELECT coalesce(\"scopes\", 'NULL') FROM \"WorkspaceApiToken\" WHERE \"id\" = ?1",
        vec![int(row.id)],
    )
    .await;
    assert_eq!(stored.as_deref(), Some("NULL"));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:239 "workspaceApiTokenRepository.insert > passes an explicit scopes array through to the insert values"
#[tokio::test]
async fn workspace_api_token_repository_insert_passes_an_explicit_scopes_array_through_to_the_insert_values()
 {
    let db = seeded().await;
    let row = ok(repo::write(&db, |tx| {
        insert(
            tx,
            &InsertWorkspaceApiTokenInput {
                workspace_id: WS,
                token_hash: "hash-2".to_owned(),
                name: "Scoped token".to_owned(),
                permission: "full".to_owned(),
                token_prefix: "cbx_ws_efgh".to_owned(),
                scopes: Some(vec!["contacts".to_owned(), "inbox".to_owned()]),
            },
        )
    })
    .await);
    assert_eq!(row.name, "Scoped token");
    assert_eq!(scopes_of(&row), Some(json!(["contacts", "inbox"])));
    let stored = scalar_text(
        &db,
        "SELECT \"scopes\" FROM \"WorkspaceApiToken\" WHERE \"id\" = ?1",
        vec![int(row.id)],
    )
    .await;
    assert_eq!(stored.as_deref(), Some(r#"["contacts","inbox"]"#));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:280 "workspaceApiTokenRepository.findDefaultByWorkspaceId > returns the default row when one exists"
#[tokio::test]
async fn workspace_api_token_repository_find_default_by_workspace_id_returns_the_default_row_when_one_exists()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    token(&db, 2, WS, "hash-2", 2_000, None, true).await;
    token(&db, 3, OTHER_WS, "hash-3", 3_000, None, true).await;
    let row = ok(repo::read(&db, |c| find_default_by_workspace_id(c, WS)).await).expect("row");
    assert_eq!(row.id, 2);
    assert!(row.is_default);
    assert_eq!(row.scopes, None);
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:298 "workspaceApiTokenRepository.findDefaultByWorkspaceId > preserves a channel-scoped default token"
#[tokio::test]
async fn workspace_api_token_repository_find_default_by_workspace_id_preserves_a_channel_scoped_default_token()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, Some(r#"["channels"]"#), true).await;
    let row = ok(repo::read(&db, |c| find_default_by_workspace_id(c, WS)).await).expect("row");
    assert_eq!(scopes_of(&row), Some(json!(["channels"])));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:312 "workspaceApiTokenRepository.findDefaultByWorkspaceId > returns null when no default row exists"
#[tokio::test]
async fn workspace_api_token_repository_find_default_by_workspace_id_returns_null_when_no_default_row_exists()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, false).await;
    let row = ok(repo::read(&db, |c| find_default_by_workspace_id(c, WS)).await);
    assert_eq!(row, None);
}

fn default_input() -> InsertDefaultWorkspaceApiTokenInput {
    InsertDefaultWorkspaceApiTokenInput {
        workspace_id: WS,
        token_hash: "hash-1".to_owned(),
        name: "Default token".to_owned(),
        token_prefix: "cbx_ws_abcd".to_owned(),
        encrypted_token: encrypted(),
    }
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:325 "workspaceApiTokenRepository.insertDefault > inserts the default row and returns it"
#[tokio::test]
async fn workspace_api_token_repository_insert_default_inserts_the_default_row_and_returns_it() {
    let db = seeded().await;
    // A non-default token of the workspace does not take the default slot.
    token(&db, 9, WS, "hash-9", 1_000, None, false).await;
    let row = ok(repo::write(&db, |tx| insert_default(tx, &default_input())).await)
        .expect("inserted default");
    assert_eq!(row.workspace_id, WS);
    assert_eq!(row.token_hash, "hash-1");
    assert_eq!(row.name, "Default token");
    assert_eq!(row.permission, "full");
    assert_eq!(row.token_prefix.as_deref(), Some("cbx_ws_abcd"));
    assert!(row.is_default);
    assert_eq!(row.encrypted_token, Some(encrypted()));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:366 "workspaceApiTokenRepository.insertDefault > returns null when a concurrent insert already won the default slot"
#[tokio::test]
async fn workspace_api_token_repository_insert_default_returns_null_when_a_concurrent_insert_already_won_the_default_slot()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-winner", 1_000, None, true).await;
    let row = ok(repo::write(&db, |tx| insert_default(tx, &default_input())).await);
    assert_eq!(row, None);
    let count = scalar_i64(&db, "SELECT count(*) FROM \"WorkspaceApiToken\"", vec![]).await;
    assert_eq!(count, Some(1));
}

/// Port of: packages/database/__tests__/workspace-api-token-repository.test.ts:389 "workspaceApiTokenRepository.setEncryptedToken > updates encryptedToken guarded on it currently being null"
#[tokio::test]
async fn workspace_api_token_repository_set_encrypted_token_updates_encrypted_token_guarded_on_it_currently_being_null()
 {
    let db = seeded().await;
    token(&db, 1, WS, "hash-1", 1_000, None, true).await;
    token(&db, 2, OTHER_WS, "hash-2", 2_000, None, true).await;
    ok(repo::write(&db, |tx| set_encrypted_token(tx, 1, WS, &encrypted())).await);
    let blob = |id: i64| {
        let db = &db;
        async move {
            scalar_text(
                db,
                "SELECT \"encryptedToken\" FROM \"WorkspaceApiToken\" WHERE \"id\" = ?1",
                vec![int(id)],
            )
            .await
        }
    };
    let first = blob(1).await.expect("set");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first).unwrap(),
        encrypted().0
    );

    // Guard: a second upgrade does not overwrite the blob.
    let other = Json(json!({"v": 1, "iv": "iv2", "text": "ct2", "tag": "tag2"}));
    ok(repo::write(&db, move |tx| set_encrypted_token(tx, 1, WS, &other)).await);
    assert_eq!(blob(1).await, Some(first));
    // Guard: the id of another workspace is not touched.
    ok(repo::write(&db, |tx| set_encrypted_token(tx, 2, WS, &encrypted())).await);
    assert_eq!(blob(2).await, None);
}
