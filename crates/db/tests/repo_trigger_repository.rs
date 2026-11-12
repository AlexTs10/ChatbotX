//! Port of `packages/database/__tests__/trigger-repository.test.ts` (P3.3). The mocked
//! `findMany` / `findFirst` / `$count` become real `Trigger` + `Condition` rows: the
//! asserted `where` objects are checked as the rows returned and counted.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::trigger::{
    ListTriggersInput, find_with_conditions, list_paginated_with_conditions,
};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const WS: i64 = 2001;

/// Triggers: 1 "Welcome" in folder 51, 2 "Welcome" at root, 3 "Other" in folder 51.
/// Trigger 2 has two conditions.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        ins!(tx, "Folder", { "id" => 51_i64, "name" => "f", "folderType" => "trigger", "workspaceId" => WS, "paths" => "[]" })?;
        ins!(tx, "Trigger", { "id" => 1_i64, "createdAt" => 1_000_i64, "name" => "Welcome", "folderId" => 51_i64, "actions" => "[]", "workspaceId" => WS })?;
        ins!(tx, "Trigger", { "id" => 2_i64, "createdAt" => 2_000_i64, "name" => "Welcome", "actions" => "[]", "workspaceId" => WS })?;
        ins!(tx, "Trigger", { "id" => 3_i64, "createdAt" => 3_000_i64, "name" => "Other", "folderId" => 51_i64, "actions" => "[]", "workspaceId" => WS })?;
        ins!(tx, "Condition", { "id" => 91_i64, "triggerId" => 2_i64, "type" => "tag" })?;
        ins!(tx, "Condition", { "id" => 92_i64, "triggerId" => 2_i64, "type" => "keyword" })?;
        Ok(())
    })
    .await;
    db
}

fn input(folder_id: Option<Option<i64>>, name: Option<&str>) -> ListTriggersInput {
    ListTriggersInput {
        workspace_id: WS,
        folder_id,
        name: name.map(str::to_owned),
        limit: 10,
        offset: 0,
    }
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:36 "triggerRepository.listPaginatedWithConditions > resolves an empty-string folderId to isNull (trigger sentinel, not rootFolderId)"
///
/// The TS `""` and `null` folder ids are `Some(None)` here.
#[tokio::test]
async fn trigger_repository_list_paginated_with_conditions_resolves_an_empty_string_folder_id_to_is_null_trigger_sentinel_not_root_folder_id()
 {
    let db = seeded().await;
    let page = ok(repo::read(&db, |c| {
        list_paginated_with_conditions(c, &input(Some(None), None))
    })
    .await);
    assert_eq!(page.total, 1);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].trigger.id, 2);
    let mut cond: Vec<i64> = page.rows[0].conditions.iter().map(|c| c.id).collect();
    cond.sort_unstable();
    assert_eq!(cond, vec![91, 92]);
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:61 "triggerRepository.listPaginatedWithConditions > filters by folderId and name when provided"
#[tokio::test]
async fn trigger_repository_list_paginated_with_conditions_filters_by_folder_id_and_name_when_provided()
 {
    let db = seeded().await;
    let page = ok(repo::read(&db, |c| {
        list_paginated_with_conditions(c, &input(Some(Some(51)), Some("Welcome")))
    })
    .await);
    assert_eq!(
        page.rows.iter().map(|r| r.trigger.id).collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(page.total, 1);
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:84 "triggerRepository.listPaginatedWithConditions > omits folderId/name filters entirely when not provided"
#[tokio::test]
async fn trigger_repository_list_paginated_with_conditions_omits_folder_id_name_filters_entirely_when_not_provided()
 {
    let db = seeded().await;
    let page = ok(repo::read(&db, |c| {
        list_paginated_with_conditions(c, &input(None, None))
    })
    .await);
    // Newest first (createdAt desc).
    assert_eq!(
        page.rows.iter().map(|r| r.trigger.id).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(page.total, 3);
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:105 "triggerRepository.findWithConditions > returns null when neither id nor workspaceId is provided"
#[tokio::test]
async fn trigger_repository_find_with_conditions_returns_null_when_neither_id_nor_workspace_id_is_provided()
 {
    let db = seeded().await;
    assert_eq!(
        ok(repo::read(&db, |c| find_with_conditions(c, None, None)).await),
        None
    );
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:111 "triggerRepository.findWithConditions > queries with conditions included when id is provided"
#[tokio::test]
async fn trigger_repository_find_with_conditions_queries_with_conditions_included_when_id_is_provided()
 {
    let db = seeded().await;
    let found =
        ok(repo::read(&db, |c| find_with_conditions(c, Some(2), None)).await).expect("trigger");
    assert_eq!(found.trigger.id, 2);
    assert_eq!(found.conditions.len(), 2);
    let empty =
        ok(repo::read(&db, |c| find_with_conditions(c, Some(1), None)).await).expect("trigger");
    assert!(empty.conditions.is_empty());
}

/// Port of: packages/database/__tests__/trigger-repository.test.ts:127 "triggerRepository.findWithConditions > returns null when no row matches"
#[tokio::test]
async fn trigger_repository_find_with_conditions_returns_null_when_no_row_matches() {
    let db = seeded().await;
    assert_eq!(
        ok(repo::read(&db, |c| find_with_conditions(c, None, Some(9999))).await),
        None
    );
}
