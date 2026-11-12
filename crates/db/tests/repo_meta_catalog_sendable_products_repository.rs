//! Port of `packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts` (P3.3).
//! The mocked select chain becomes real `Product` / `MetaCatalogItem` / `IntegrationMetaCatalog`
//! rows; each asserted `where` condition is checked by a decoy row that breaks only it.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::meta_catalog_item::{
    CatalogScope, SendableListInput, find_linked_products_by_retailer_ids,
    find_sendable_by_product_ids, list_sendable_products,
};
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;
const CONN: i64 = 61;

fn scope() -> CatalogScope {
    CatalogScope {
        integration_meta_catalog_id: CONN,
        catalog_id: "cat-1".to_owned(),
    }
}

/// One connection per workspace (`IntegrationMetaCatalog_workspaceId_key`): 61 = WS (cat-1),
/// 62 = workspace 2003 (cat-1, deleted), 63 = workspace 2004 (cat-2), 64 = OTHER_WS (cat-1).
/// Products of WS: 1 active (sku-1 on 61), 2 draft (sku-2 on 61), 3 active (sku-3 on 62),
/// 4 active (sku-4 on 63), 6 active (sku-6 on 64); 5 active of OTHER_WS (sku-5 on 61).
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::workspace(tx, 1009, OTHER_WS)?;
        repo4::meta_catalog(tx, CONN, WS, "cat-1", false)?;
        repo4::workspace(tx, 1010, 2003)?;
        repo4::workspace(tx, 1011, 2004)?;
        repo4::meta_catalog(tx, 62, 2003, "cat-1", true)?;
        repo4::meta_catalog(tx, 63, 2004, "cat-2", false)?;
        repo4::meta_catalog(tx, 64, OTHER_WS, "cat-1", false)?;
        repo4::product(tx, 1, WS, "Alpha", "active", None)?;
        repo4::product(tx, 2, WS, "Bravo", "draft", None)?;
        repo4::product(tx, 3, WS, "Charlie", "active", None)?;
        repo4::product(tx, 4, WS, "Delta", "active", None)?;
        repo4::product(tx, 5, OTHER_WS, "Echo", "active", None)?;
        repo4::product(tx, 6, WS, "Foxtrot", "active", None)?;
        repo4::catalog_item(tx, CONN, "cat-1", 1, "sku-1")?;
        repo4::catalog_item(tx, CONN, "cat-1", 2, "sku-2")?;
        repo4::catalog_item(tx, 62, "cat-1", 3, "sku-3")?;
        repo4::catalog_item(tx, 63, "cat-2", 4, "sku-4")?;
        repo4::catalog_item(tx, CONN, "cat-1", 5, "sku-5")?;
        repo4::catalog_item(tx, 64, "cat-1", 6, "sku-6")?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:105 "metaCatalogItemRepository.findSendableByProductIds > scopes to the workspace, the current catalog and active products"
#[tokio::test]
async fn meta_catalog_item_repository_find_sendable_by_product_ids_scopes_to_the_workspace_the_current_catalog_and_active_products()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| {
        find_sendable_by_product_ids(c, &scope(), WS, &[1, 2, 3, 4, 5])
    })
    .await);
    assert_eq!(
        rows.iter()
            .map(|r| (r.product_id, r.retailer_id.as_str(), r.catalog_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "sku-1", "cat-1")]
    );
    let narrowed =
        ok(repo::read(&db, |c| find_sendable_by_product_ids(c, &scope(), WS, &[2])).await);
    assert!(narrowed.is_empty());
    // The connection of another workspace does not resolve WS products.
    let foreign = CatalogScope {
        integration_meta_catalog_id: 64,
        catalog_id: "cat-1".to_owned(),
    };
    let rows = ok(repo::read(&db, move |c| {
        find_sendable_by_product_ids(c, &foreign, WS, &[6])
    })
    .await);
    assert!(rows.is_empty());
    // A deleted connection resolves nothing.
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "deletedAt" = 1 WHERE "id" = 61"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| find_sendable_by_product_ids(c, &scope(), WS, &[1])).await);
    assert!(rows.is_empty());
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:129 "metaCatalogItemRepository.findSendableByProductIds > does not query for an empty id list"
#[tokio::test]
async fn meta_catalog_item_repository_find_sendable_by_product_ids_does_not_query_for_an_empty_id_list()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| find_sendable_by_product_ids(c, &scope(), WS, &[])).await);
    assert!(rows.is_empty());
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:142 "metaCatalogItemRepository.listSendableProducts > pages with a stable order and derives the page count from the total"
#[tokio::test]
async fn meta_catalog_item_repository_list_sendable_products_pages_with_a_stable_order_and_derives_the_page_count_from_the_total()
 {
    let db = seeded().await;
    // 44 more active linked products: 45 sendable in total (product 1 + these), names in pairs.
    setup(&db, |tx| {
        for i in 0..44_i64 {
            let id = 100 + i;
            repo4::product(tx, id, WS, &format!("P{:02}", i / 2), "active", None)?;
            repo4::catalog_item(tx, CONN, "cat-1", id, &format!("r-{id}"))?;
        }
        Ok(())
    })
    .await;
    let input = SendableListInput {
        scope: scope(),
        workspace_id: WS,
        keyword: None,
        product_ids: None,
        page: Some(3),
        per_page: Some(10),
    };
    let page = ok(repo::read(&db, move |c| list_sendable_products(c, &input)).await);
    assert_eq!(page.page_count, 5);
    // Sorted by name then id: Alpha, P00(100), P00(101), P01(102), ... -> rows 21..30.
    assert_eq!(
        page.data.iter().map(|r| r.product_id).collect::<Vec<_>>(),
        (119..129).collect::<Vec<_>>()
    );
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:165 "metaCatalogItemRepository.listSendableProducts > filters by name with an escaped LIKE pattern, never raw SQL"
#[tokio::test]
async fn meta_catalog_item_repository_list_sendable_products_filters_by_name_with_an_escaped_like_pattern_never_raw_sql()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        repo4::product(tx, 30, WS, "100% cotton", "active", None)?;
        repo4::product(tx, 31, WS, "1000 shirts", "active", None)?;
        repo4::catalog_item(tx, CONN, "cat-1", 30, "r-30")?;
        repo4::catalog_item(tx, CONN, "cat-1", 31, "r-31")?;
        Ok(())
    })
    .await;
    let input = SendableListInput {
        scope: scope(),
        workspace_id: WS,
        keyword: Some("100%".to_owned()),
        product_ids: None,
        page: None,
        per_page: None,
    };
    let page = ok(repo::read(&db, move |c| list_sendable_products(c, &input)).await);
    assert_eq!(
        page.data
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["100% cotton"]
    );
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:179 "metaCatalogItemRepository.listSendableProducts > narrows to the given product ids, and an empty list matches nothing"
#[tokio::test]
async fn meta_catalog_item_repository_list_sendable_products_narrows_to_the_given_product_ids_and_an_empty_list_matches_nothing()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        repo4::product(tx, 30, WS, "Zulu", "active", None)?;
        repo4::catalog_item(tx, CONN, "cat-1", 30, "r-30")?;
        Ok(())
    })
    .await;
    let mk = |ids: Option<Vec<i64>>| SendableListInput {
        scope: scope(),
        workspace_id: WS,
        keyword: None,
        product_ids: ids,
        page: None,
        per_page: None,
    };
    let narrowed = mk(Some(vec![1, 2]));
    let page = ok(repo::read(&db, move |c| list_sendable_products(c, &narrowed)).await);
    assert_eq!(
        page.data.iter().map(|r| r.product_id).collect::<Vec<_>>(),
        vec![1]
    );
    let empty = mk(Some(vec![]));
    let page = ok(repo::read(&db, move |c| list_sendable_products(c, &empty)).await);
    assert!(page.data.is_empty());
    assert_eq!(page.page_count, 0);
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:202 "metaCatalogItemRepository.listSendableProducts > reports zero pages for an empty catalog"
#[tokio::test]
async fn meta_catalog_item_repository_list_sendable_products_reports_zero_pages_for_an_empty_catalog()
 {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::meta_catalog(tx, CONN, WS, "cat-1", false)?;
        Ok(())
    })
    .await;
    let input = SendableListInput {
        scope: scope(),
        workspace_id: WS,
        keyword: None,
        product_ids: None,
        page: None,
        per_page: None,
    };
    let page = ok(repo::read(&db, move |c| list_sendable_products(c, &input)).await);
    assert!(page.data.is_empty());
    assert_eq!(page.page_count, 0);
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:217 "metaCatalogItemRepository.findLinkedProductsByRetailerIds > scopes to the workspace and current catalog but still resolves inactive products"
#[tokio::test]
async fn meta_catalog_item_repository_find_linked_products_by_retailer_ids_scopes_to_the_workspace_and_current_catalog_but_still_resolves_inactive_products()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| {
        find_linked_products_by_retailer_ids(
            c,
            &scope(),
            WS,
            &["sku-1", "sku-2", "sku-3", "sku-4", "sku-5"],
        )
    })
    .await);
    let mut got: Vec<(i64, String)> = rows
        .iter()
        .map(|r| (r.product_id, r.retailer_id.clone()))
        .collect();
    got.sort();
    assert_eq!(got, vec![(1, "sku-1".to_owned()), (2, "sku-2".to_owned())]);
}

/// Port of: packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts:241 "metaCatalogItemRepository.findLinkedProductsByRetailerIds > skips the query when there are no retailer ids"
#[tokio::test]
async fn meta_catalog_item_repository_find_linked_products_by_retailer_ids_skips_the_query_when_there_are_no_retailer_ids()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| {
        find_linked_products_by_retailer_ids(c, &scope(), WS, &[])
    })
    .await);
    assert!(rows.is_empty());
}
