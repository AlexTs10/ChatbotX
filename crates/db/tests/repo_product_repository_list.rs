//! Port of `packages/database/__tests__/product-repository-list.test.ts` (P3.3).
//! The mocked `findMany` where object becomes real `Product` rows: the id filter is
//! observed as the returned ids (inside the workspace).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::product::{ProductListInput, list};
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::workspace(tx, 1009, OTHER_WS)?;
        repo4::product(tx, 1, WS, "One", "active", None)?;
        repo4::product(tx, 2, WS, "Two", "active", None)?;
        repo4::product(tx, 3, WS, "Three", "active", None)?;
        repo4::product(tx, 4, OTHER_WS, "Four", "active", None)?;
        Ok(())
    })
    .await;
    db
}

async fn listed_ids(db: &common::TestDb, ids: Option<Vec<i64>>) -> Vec<i64> {
    let input = ProductListInput {
        workspace_id: WS,
        ids,
        ..ProductListInput::default()
    };
    let page = ok(repo::read(db, move |c| list(c, &input)).await);
    let mut ids: Vec<i64> = page.data.iter().map(|r| r.product.id).collect();
    ids.sort_unstable();
    ids
}

/// Port of: packages/database/__tests__/product-repository-list.test.ts:25 "productRepository.list ids filter > restricts the list to the given ids, within the workspace"
#[tokio::test]
async fn product_repository_list_ids_filter_restricts_the_list_to_the_given_ids_within_the_workspace()
 {
    let db = seeded().await;
    assert_eq!(listed_ids(&db, Some(vec![1, 2, 4])).await, vec![1, 2]);
}

/// Port of: packages/database/__tests__/product-repository-list.test.ts:36 "productRepository.list ids filter > applies no id filter when ids is omitted"
#[tokio::test]
async fn product_repository_list_ids_filter_applies_no_id_filter_when_ids_is_omitted() {
    let db = seeded().await;
    assert_eq!(listed_ids(&db, None).await, vec![1, 2, 3]);
}

/// Port of: packages/database/__tests__/product-repository-list.test.ts:36 "productRepository.list ids filter > applies no id filter when ids is null"
#[tokio::test]
async fn product_repository_list_ids_filter_applies_no_id_filter_when_ids_is_null() {
    let db = seeded().await;
    assert_eq!(listed_ids(&db, None).await, vec![1, 2, 3]);
}

/// Port of: packages/database/__tests__/product-repository-list.test.ts:36 "productRepository.list ids filter > applies no id filter when ids is empty"
#[tokio::test]
async fn product_repository_list_ids_filter_applies_no_id_filter_when_ids_is_empty() {
    let db = seeded().await;
    assert_eq!(listed_ids(&db, Some(vec![])).await, vec![1, 2, 3]);
}
