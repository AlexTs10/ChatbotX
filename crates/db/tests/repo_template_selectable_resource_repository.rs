//! Port of `packages/database/__tests__/template-selectable-resource-repository.test.ts`
//! (P3.3). The mocked `findMany` / `$count` become real rows; "only the page query ran"
//! is `allIds` absent.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::template_selectable_resource::{CategoryInput, list_flows, list_tags};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{NOW, ok, setup};

const WS: i64 = 2001;

fn input(offset: i64, limit: i64) -> CategoryInput {
    CategoryInput {
        workspace_id: WS,
        offset,
        limit,
        ..Default::default()
    }
}

/// Port of: packages/database/__tests__/template-selectable-resource-repository.test.ts:67 "templateSelectableResourceRepository.listFlows > returns rows, total, and allIds when under the cap"
#[tokio::test]
async fn template_selectable_resource_repository_list_flows_returns_rows_total_and_all_ids_when_under_the_cap()
 {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| repo3::flow(tx, 61, WS)).await;
    let page = ok(repo::read(&db, |c| list_flows(c, &input(0, 100))).await);
    assert_eq!(
        page.rows
            .iter()
            .map(|r| (r.id.as_str(), r.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("61", "Flow 61")]
    );
    assert_eq!(page.total, 1);
    assert_eq!(page.all_ids, Some(vec!["61".to_owned()]));
}

/// Port of: packages/database/__tests__/template-selectable-resource-repository.test.ts:84 "templateSelectableResourceRepository.listFlows > omits allIds when offset is not 0"
#[tokio::test]
async fn template_selectable_resource_repository_list_flows_omits_all_ids_when_offset_is_not_0() {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        for id in 1..=3_i64 {
            repo3::flow(tx, 60 + id, WS)?;
        }
        Ok(())
    })
    .await;
    let page = ok(repo::read(&db, |c| list_flows(c, &input(1, 100))).await);
    assert_eq!(page.all_ids, None);
    assert_eq!(page.total, 3);
    assert_eq!(page.rows.len(), 2);
}

/// Port of: packages/database/__tests__/template-selectable-resource-repository.test.ts:105 "templateSelectableResourceRepository.listTags > filters out soft-deleted tags via deletedAt isNull"
#[tokio::test]
async fn template_selectable_resource_repository_list_tags_filters_out_soft_deleted_tags_via_deleted_at_is_null()
 {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        ins!(tx, "Tag", { "id" => 1_i64, "name" => "Tag 1", "workspaceId" => WS })?;
        ins!(tx, "Tag", { "id" => 2_i64, "name" => "Tag 2", "workspaceId" => WS, "deletedAt" => NOW })?;
        Ok(())
    })
    .await;
    let page = ok(repo::read(&db, |c| list_tags(c, &input(0, 100))).await);
    assert_eq!(
        page.rows
            .iter()
            .map(|r| (r.id.as_str(), r.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("1", "Tag 1")]
    );
    assert_eq!(page.total, 1);
}
