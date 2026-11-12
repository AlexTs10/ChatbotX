//! Port of `packages/database/__tests__/integration/product-import-identifiers.test.ts` (P3.3).
//! A fresh migrated SQLite file per test replaces the rolled-back Postgres transaction.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::product::find_by_import_identifiers;
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const PRODUCT: i64 = 812_345_678_901;

async fn seed_product(sku: &'static str) -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, move |tx| {
        repo4::product(tx, PRODUCT, WS, "Jean", "active", Some(sku))?;
        Ok(())
    })
    .await;
    db
}

async fn matches(db: &common::TestDb, ids: Vec<String>) -> Vec<i64> {
    ok(repo::read(db, move |c| {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        find_by_import_identifiers(c, WS, &refs)
    })
    .await)
    .into_iter()
    .map(|p| p.id)
    .collect()
}

/// Port of: packages/database/__tests__/integration/product-import-identifiers.test.ts:69 "productRepository.findByImportIdentifiers (real database) > matches a non-numeric retailer id by SKU without comparing it to the bigint id"
#[tokio::test]
async fn product_repository_find_by_import_identifiers_real_database_matches_a_non_numeric_retailer_id_by_sku_without_comparing_it_to_the_bigint_id()
 {
    let db = seed_product("SKU-RED-01").await;
    let got = matches(&db, vec!["SKU-RED-01".to_owned(), "abc123xyz".to_owned()]).await;
    assert_eq!(got, vec![PRODUCT]);
}

/// Port of: packages/database/__tests__/integration/product-import-identifiers.test.ts:82 "productRepository.findByImportIdentifiers (real database) > still matches a retailer id that is our own product id"
#[tokio::test]
async fn product_repository_find_by_import_identifiers_real_database_still_matches_a_retailer_id_that_is_our_own_product_id()
 {
    let db = seed_product("SKU-1").await;
    let got = matches(&db, vec!["not-an-id".to_owned(), PRODUCT.to_string()]).await;
    assert_eq!(got, vec![PRODUCT]);
}

/// Port of: packages/database/__tests__/integration/product-import-identifiers.test.ts:95 "productRepository.findByImportIdentifiers (real database) > does not fail on a number too large for a bigint"
#[tokio::test]
async fn product_repository_find_by_import_identifiers_real_database_does_not_fail_on_a_number_too_large_for_a_bigint()
 {
    let db = seed_product("SKU-2").await;
    let got = matches(&db, vec!["99999999999999999999999".to_owned()]).await;
    assert!(got.is_empty());
}

/// Port of: packages/database/__tests__/integration/product-import-identifiers.test.ts:108 "productRepository.findByImportIdentifiers (real database) > returns nothing for an empty list without querying"
#[tokio::test]
async fn product_repository_find_by_import_identifiers_real_database_returns_nothing_for_an_empty_list_without_querying()
 {
    let db = seed_product("SKU-3").await;
    assert!(matches(&db, vec![]).await.is_empty());
}
