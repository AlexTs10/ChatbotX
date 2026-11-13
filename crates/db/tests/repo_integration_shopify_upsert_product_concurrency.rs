//! Port of `packages/database/__tests__/integration/shopify-upsert-product-concurrency.test.ts`
//! (P3.3). The TS runs two writes on a Postgres pool; here two write jobs run at once and
//! the one SQLite writer serialises them.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use std::sync::Arc;

use chatbotx_db::repo;
use chatbotx_db::repo::external_store::{
    ExternalProductOption, ExternalVariantValues, UpsertProductInput, upsert_product,
};
use chatbotx_db::rusqlite::params;
use chatbotx_db::rusqlite::types::Value;
use repo_common::{ok, scalar_i64, setup};

const OWNER: i64 = 1001;
const WS: i64 = 2001;

async fn shop_db() -> common::TestDb {
    let db = common::fresh_db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'shopify-upsert@example.test')"#,
            params![OWNER],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'shopify-upsert', ?2)"#,
            params![WS, OWNER],
        )?;
        Ok(())
    })
    .await;
    db
}

fn product_input(status: &str) -> UpsertProductInput {
    UpsertProductInput {
        workspace_id: WS,
        source: "shopify".to_owned(),
        external_id: "gid://shopify/Product/1".to_owned(),
        values: vec![
            ("name", Value::Text("Tote".to_owned())),
            ("price", Value::Real(10.0)),
            ("currency", Value::Text("USD".to_owned())),
            ("status", Value::Text(status.to_owned())),
        ],
        options: vec![
            ExternalProductOption {
                name: "Size".to_owned(),
                values: vec!["S".to_owned(), "M".to_owned()],
                position: 1,
            },
            ExternalProductOption {
                name: "Color".to_owned(),
                values: vec!["Black".to_owned()],
                position: 2,
            },
        ],
        variants: ["S", "M"]
            .iter()
            .map(|size| ExternalVariantValues {
                external_id: format!("gid://shopify/ProductVariant/{size}"),
                external_inventory_item_id: None,
                combination: serde_json::json!({ "Size": size, "Color": "Black" }),
                price: 10.0,
                sku: None,
                inventory_quantity: Some(1),
                is_enabled: true,
            })
            .collect(),
    }
}

async fn upsert(
    db: &chatbotx_db::Db,
    input: UpsertProductInput,
) -> chatbotx_db::repo::models::Product {
    ok(repo::write(db, move |tx| upsert_product(tx, &input)).await)
}

/// Port of: packages/database/__tests__/integration/shopify-upsert-product-concurrency.test.ts:87 "externalStoreRepository.upsertProduct under concurrency (real database) > two writes of one product leave one set of options and variants"
#[tokio::test]
async fn external_store_repository_upsert_product_under_concurrency_real_database_two_writes_of_one_product_leave_one_set_of_options_and_variants()
 {
    let db = Arc::new(shop_db().await);
    let input = product_input("active");
    let (a, b) = (Arc::clone(&db), Arc::clone(&db));
    let (ia, ib) = (input.clone(), input.clone());
    let first = tokio::spawn(async move { upsert(&a, ia).await });
    let second = tokio::spawn(async move { upsert(&b, ib).await });
    let (first, second) = (first.await.expect("join"), second.await.expect("join"));
    assert_eq!(first.id, second.id);

    let options = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ProductVariantOption" WHERE "productId" = ?1"#,
        vec![Value::Integer(first.id)],
    )
    .await;
    let variants = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ProductVariant" WHERE "productId" = ?1"#,
        vec![Value::Integer(first.id)],
    )
    .await;
    assert_eq!(options, Some(input.options.len() as i64));
    assert_eq!(variants, Some(input.variants.len() as i64));
}

/// Port of: packages/database/__tests__/integration/shopify-upsert-product-concurrency.test.ts:107 "externalStoreRepository.upsertProduct under concurrency (real database) > a product follows Shopify's status through draft, unlisted and back to active"
#[tokio::test]
async fn external_store_repository_upsert_product_under_concurrency_real_database_a_product_follows_shopify_s_status_through_draft_unlisted_and_back_to_active()
 {
    let db = shop_db().await;
    upsert(&db, product_input("active")).await;
    let drafted = upsert(&db, product_input("draft")).await;
    assert_eq!(drafted.status, "draft");
    let unlisted = upsert(&db, product_input("unlisted")).await;
    assert_eq!(unlisted.status, "unlisted");
    let republished = upsert(&db, product_input("active")).await;
    assert_eq!(republished.status, "active");
}
