//! Port of `packages/database/__tests__/workspace-member-repository.test.ts` (P3.3). The
//! mocked select/update chains become real `WorkspaceMember` rows: "scoped to workspaceId
//! AND userIds" = rows of another workspace or user are not returned / not touched;
//! "does not query" = empty result for empty input.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::workspace_member::{
    WorkspaceMemberPermissionsRow, list_permissions_by_user_ids, mark_online_bulk,
};
use chatbotx_db::repo::{self, Json};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{NOW, ok, rows, setup};
use serde_json::json;

/// Workspace 2001 with members 1001 (owner), 1002, 1003; workspace 2002 with member 1002.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, 2002).await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => 1002_i64, "email" => "u2@example.com" })?;
        ins!(tx, "User", { "id" => 1003_i64, "email" => "u3@example.com" })?;
        ins!(tx, "WorkspaceMember", { "id" => 1_i64, "workspaceId" => 2001_i64, "userId" => 1001_i64, "role" => "owner", "permissions" => r#"{"contacts":true}"# })?;
        ins!(tx, "WorkspaceMember", { "id" => 2_i64, "workspaceId" => 2001_i64, "userId" => 1002_i64, "role" => "agent", "permissions" => r#"{"inbox":true}"# })?;
        ins!(tx, "WorkspaceMember", { "id" => 3_i64, "workspaceId" => 2001_i64, "userId" => 1003_i64, "role" => "agent" })?;
        ins!(tx, "WorkspaceMember", { "id" => 4_i64, "workspaceId" => 2002_i64, "userId" => 1002_i64, "role" => "agent", "permissions" => r#"{"other":true}"# })?;
        Ok(())
    })
    .await;
    db
}

async fn online_since(db: &common::TestDb) -> Vec<(i64, Option<i64>)> {
    rows(
        db,
        "SELECT \"id\", \"onlineSince\" FROM \"WorkspaceMember\" ORDER BY \"id\"",
        vec![],
    )
    .await
    .into_iter()
    .map(|r| {
        let id = match &r[0] {
            chatbotx_db::rusqlite::types::Value::Integer(i) => *i,
            other => panic!("{other:?}"),
        };
        let at = match &r[1] {
            chatbotx_db::rusqlite::types::Value::Integer(i) => Some(*i),
            _ => None,
        };
        (id, at)
    })
    .collect()
}

/// Port of: packages/database/__tests__/workspace-member-repository.test.ts:46 "workspaceMemberRepository.listPermissionsByUserIds > returns [] without querying when userIds is empty"
#[tokio::test]
async fn workspace_member_repository_list_permissions_by_user_ids_returns_without_querying_when_user_ids_is_empty()
 {
    let db = seeded().await;
    let result = ok(repo::read(&db, |c| list_permissions_by_user_ids(c, 2001, &[])).await);
    assert_eq!(result, vec![]);
}

/// Port of: packages/database/__tests__/workspace-member-repository.test.ts:56 "workspaceMemberRepository.listPermissionsByUserIds > scopes the query to workspaceId AND the requested userIds"
#[tokio::test]
async fn workspace_member_repository_list_permissions_by_user_ids_scopes_the_query_to_workspace_id_and_the_requested_user_ids()
 {
    let db = seeded().await;
    // 1003 is a member of 2001 but not requested; 1002's row in 2002 is another workspace;
    // 4242 has no row.
    let mut result = ok(repo::read(&db, |c| {
        list_permissions_by_user_ids(c, 2001, &[1001, 1002, 4242])
    })
    .await);
    result.sort_by_key(|r| r.user_id);
    assert_eq!(
        result,
        vec![
            WorkspaceMemberPermissionsRow {
                user_id: 1001,
                permissions: Json(json!({"contacts": true})),
            },
            WorkspaceMemberPermissionsRow {
                user_id: 1002,
                permissions: Json(json!({"inbox": true})),
            },
        ]
    );
}

/// Port of: packages/database/__tests__/workspace-member-repository.test.ts:90 "workspaceMemberRepository.markOnlineBulk > sets onlineSince=now() for every userId in ONE bulk UPDATE, scoped to workspaceId"
#[tokio::test]
async fn workspace_member_repository_mark_online_bulk_sets_online_since_now_for_every_user_id_in_one_bulk_update_scoped_to_workspace_id()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| mark_online_bulk(tx, 2001, &[1001, 1002], NOW)).await);
    assert_eq!(
        online_since(&db).await,
        vec![(1, Some(NOW)), (2, Some(NOW)), (3, None), (4, None)]
    );
}

/// Port of: packages/database/__tests__/workspace-member-repository.test.ts:109 "workspaceMemberRepository.markOnlineBulk > is a no-op with no query when userIds is empty"
#[tokio::test]
async fn workspace_member_repository_mark_online_bulk_is_a_no_op_with_no_query_when_user_ids_is_empty()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| mark_online_bulk(tx, 2001, &[], NOW)).await);
    assert_eq!(
        online_since(&db).await,
        vec![(1, None), (2, None), (3, None), (4, None)]
    );
}

/// Port of: packages/database/__tests__/workspace-member-repository.test.ts:118 "workspaceMemberRepository.markOnlineBulk > a support-session userId with no matching row is included in the same IN clause, not special-cased (the UPDATE...WHERE itself is what silently skips it)"
#[tokio::test]
async fn workspace_member_repository_mark_online_bulk_a_support_session_user_id_with_no_matching_row_is_included_in_the_same_in_clause_not_special_cased_the_update_where_itself_is_what_silently_skips_it()
 {
    let db = seeded().await;
    // 9_999_999 stands for the synthetic support-session user (no WorkspaceMember row).
    ok(repo::write(&db, |tx| {
        mark_online_bulk(tx, 2001, &[1001, 9_999_999], NOW)
    })
    .await);
    assert_eq!(
        online_since(&db).await,
        vec![(1, Some(NOW)), (2, None), (3, None), (4, None)]
    );
}
