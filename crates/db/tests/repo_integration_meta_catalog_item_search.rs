//! Port of `packages/database/__tests__/integration/meta-catalog-item-search.test.ts` (P3.3).
//! A fresh migrated SQLite file per test replaces the rolled-back Postgres transaction.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::meta_catalog_item::{CatalogScope, search_with_product_info};
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const CATALOG_ID: &str = "catalog-1";
const WS: i64 = 2001;

async fn seed_catalog() -> (common::TestDb, CatalogScope) {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::meta_catalog(tx, 71, WS, CATALOG_ID, false)?;
        for (i, (name, retailer_id)) in [("Jean", "SKU-1"), ("Shirt", "SKU-2")].iter().enumerate() {
            let id = 10 + i as i64;
            repo4::product(tx, id, WS, name, "active", Some(retailer_id))?;
            repo4::catalog_item(tx, 71, CATALOG_ID, id, retailer_id)?;
        }
        Ok(())
    })
    .await;
    (
        db,
        CatalogScope {
            integration_meta_catalog_id: 71,
            catalog_id: CATALOG_ID.to_owned(),
        },
    )
}

/// Port of: packages/database/__tests__/integration/meta-catalog-item-search.test.ts:93 "metaCatalogItemRepository.searchWithProductInfo (real database) > finds exactly the item whose retailer id is asked for, so a saved pick can be shown by name"
#[tokio::test]
async fn meta_catalog_item_repository_search_with_product_info_real_database_finds_exactly_the_item_whose_retailer_id_is_asked_for_so_a_saved_pick_can_be_shown_by_name()
 {
    let (db, scope) = seed_catalog().await;
    let found = ok(repo::read(&db, move |c| {
        search_with_product_info(c, &scope, None, Some("SKU-2"), None)
    })
    .await);
    assert_eq!(
        found
            .iter()
            .map(|i| (i.retailer_id.as_str(), i.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("SKU-2", "Shirt")]
    );
}

/// Port of: packages/database/__tests__/integration/meta-catalog-item-search.test.ts:108 "metaCatalogItemRepository.searchWithProductInfo (real database) > finds nothing for a retailer id the catalog does not have"
#[tokio::test]
async fn meta_catalog_item_repository_search_with_product_info_real_database_finds_nothing_for_a_retailer_id_the_catalog_does_not_have()
 {
    let (db, scope) = seed_catalog().await;
    let found = ok(repo::read(&db, move |c| {
        search_with_product_info(c, &scope, None, Some("missing"), None)
    })
    .await);
    assert!(found.is_empty());
}

/// Port of: packages/database/__tests__/integration/meta-catalog-item-search.test.ts:121 "metaCatalogItemRepository.searchWithProductInfo (real database) > still lists by name when no retailer id is given"
#[tokio::test]
async fn meta_catalog_item_repository_search_with_product_info_real_database_still_lists_by_name_when_no_retailer_id_is_given()
 {
    let (db, scope) = seed_catalog().await;
    let s2 = scope.clone();
    let all = ok(repo::read(&db, move |c| {
        search_with_product_info(c, &scope, None, None, None)
    })
    .await);
    let by_name = ok(repo::read(&db, move |c| {
        search_with_product_info(c, &s2, Some("jea"), None, None)
    })
    .await);
    assert_eq!(
        all.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        vec!["Jean", "Shirt"]
    );
    assert_eq!(
        by_name.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        vec!["Jean"]
    );
}

/// Port of: packages/database/__tests__/integration/meta-catalog-item-search.test.ts:139 "metaCatalogItemRepository.searchWithProductInfo (real database) > never reaches into another catalog"
#[tokio::test]
async fn meta_catalog_item_repository_search_with_product_info_real_database_never_reaches_into_another_catalog()
 {
    let (db, scope) = seed_catalog().await;
    let other = CatalogScope {
        catalog_id: "other-catalog".to_owned(),
        ..scope
    };
    let found = ok(repo::read(&db, move |c| {
        search_with_product_info(c, &other, None, Some("SKU-1"), None)
    })
    .await);
    assert!(found.is_empty());
}
