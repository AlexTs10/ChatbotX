//! Port of `packages/business/src/webshop/__tests__/collection.test.ts`
//! (Q.P6-U4). Adapted: the mocked `listForShop` / settings / language become
//! real rows (`buildCollectionCards` takes the listed products and the
//! settings value); the shop token is the real signed token, so "signs the
//! card links for the contact" checks that the token verifies to the contact.

mod common;

use chatbotx_business::ecommerce_settings;
use chatbotx_business::webshop::collection::{
    build_collection_cards, list_collection_products, resolve_collection_step,
};
use chatbotx_db::repo::product::ShopProduct;
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, category, setup, two_workspaces, vendor};
use serde_json::{Value, json};

const APP: &str = "https://app.example";

/// Category 1 (sub 2), vendor 3 "Acme"; product 10 "Product 10" at 20 in
/// sub 2 with an image and a product URL; product 11 in category 1.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Shirts")?;
        category(tx, 2, WS, Some(1), "Tees")?;
        vendor(tx, 3, WS, "Acme")?;
        for (id, name, sub) in [(10, "Product 10 shirt", Some(2)), (11, "Product 11 shirt", None)] {
            tx.execute(
                r#"INSERT INTO "Product" ("id", "workspaceId", "name", "price", "categoryId", "subcategoryId", "vendorId", "images", "productUrl")
                   VALUES (?1, ?2, ?3, 20, 1, ?4, 3, ?5, ?6)"#,
                params![
                    id,
                    WS,
                    name,
                    sub,
                    json!([{ "url": format!("https://img/{id}.jpg"), "type": "link" }]).to_string(),
                    format!("https://brand.example/{id}"),
                ],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn automated(category: Value, sub: Value) -> Value {
    json!({ "collectionType": "automated", "productIds": [],
            "conditions": { "name": " shirt ", "categoryId": category, "subcategoryId": sub, "vendorId": "3" } })
}

async fn products(db: &TestDb, step: Value) -> Vec<ShopProduct> {
    db.read(move |conn| Ok(list_collection_products(conn, WS, &step).unwrap()))
        .await
        .unwrap()
}

fn settings(buttons: &[&str], checkout_mode: &str) -> Value {
    let mut s = ecommerce_settings::defaults();
    s["general"]["productButtons"] = json!(buttons);
    s["checkout"]["checkoutMode"] = json!(checkout_mode);
    s["payments"]["currency"] = json!("USD");
    s["payments"]["currencySymbol"] = json!("$");
    s["payments"]["currencyPosition"] = json!("left");
    s
}

fn cards(products: &[ShopProduct], settings: &Value, language: &str) -> Vec<Value> {
    build_collection_cards(products, settings, WS, APP, "TOKEN", language)
}

#[tokio::test]
async fn automated_filters_like_the_shop_the_sub_category_narrowing_the_category() {
    let db = db().await;
    let ids: Vec<i64> = products(&db, automated(json!("1"), json!("2")))
        .await
        .iter()
        .map(|p| p.product.id)
        .collect();
    assert_eq!(ids, vec![10]);
    let ids: Vec<i64> = products(&db, automated(json!("1"), Value::Null))
        .await
        .iter()
        .map(|p| p.product.id)
        .collect();
    assert_eq!(ids, vec![10, 11]);
}

#[tokio::test]
async fn manual_only_shown_products_in_the_chosen_order() {
    let db = db().await;
    let step =
        json!({ "collectionType": "manual", "productIds": ["11", "999", "10"], "conditions": {} });
    let ids: Vec<i64> = products(&db, step)
        .await
        .iter()
        .map(|p| p.product.id)
        .collect();
    assert_eq!(ids, vec![11, 10]);
}

#[tokio::test]
async fn the_price_stacks_the_vendor_discount_on_the_product_discount() {
    let db = db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Product" SET "price" = 100, "discount" = 10 WHERE "id" = 10"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "Vendor" SET "discountPercent" = 20 WHERE "id" = 3"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(&listed, &settings(&["addToCart"], "default"), "en")[0];
    assert_eq!(card["subtitle"], "$72.00 · Acme");
}

#[tokio::test]
async fn a_card_per_product_opening_its_shop_page_signed_per_contact() {
    let db = db().await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(&listed, &settings(&["addToCart"], "default"), "en")[0];
    assert_eq!(card["stepType"], "sendCard");
    assert_eq!(card["title"], "Product 10 shirt");
    assert_eq!(card["subtitle"], "$20.00 · Acme");
    assert_eq!(card["image"]["url"], "https://img/10.jpg");
    assert_eq!(card["image"]["mode"], "url");
    let buttons = card["buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["label"], "View");
    assert_eq!(buttons[0]["buttonType"], "openWebsite");
    assert_eq!(
        buttons[0]["beforeStep"]["url"],
        format!("{APP}/shop/{WS}/products/10?token=TOKEN")
    );
}

#[tokio::test]
async fn similar_opens_the_product_category_when_the_store_shows_that_button() {
    let db = db().await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(
        &listed,
        &settings(&["addToCart", "similar"], "default"),
        "vi",
    )[0];
    let labels: Vec<&str> = card["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["Xem", "Tương tự"]);
    assert_eq!(
        card["buttons"][1]["beforeStep"]["url"],
        format!("{APP}/shop/{WS}?token=TOKEN&category=1")
    );
}

#[tokio::test]
async fn with_the_external_product_page_checkout_buy_opens_the_product_page() {
    let db = db().await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(&listed, &settings(&["addToCart"], "productPage"), "en")[0];
    assert_eq!(card["buttons"][0]["label"], "Buy");
    assert_eq!(
        card["buttons"][0]["beforeStep"]["url"],
        "https://brand.example/10"
    );
}

#[tokio::test]
async fn long_names_fit_messengers_80_characters() {
    let db = db().await;
    let name = format!("{} shirt", "x".repeat(120));
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "Product" SET "name" = ?1 WHERE "id" = 10"#,
            params![name],
        )?;
        Ok(())
    })
    .await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(&listed, &settings(&[], "default"), "en")[0];
    assert_eq!(card["title"].as_str().unwrap().chars().count(), 80);
}

#[tokio::test]
async fn a_product_link_that_is_not_a_web_url_falls_back_to_the_shop_page() {
    for url in ["www.brand.example/p1", "javascript:alert(1)"] {
        let db = db().await;
        let url = url.to_owned();
        setup(&db, move |tx| {
            tx.execute(
                r#"UPDATE "Product" SET "productUrl" = ?1 WHERE "id" = 10"#,
                params![url],
            )?;
            Ok(())
        })
        .await;
        let listed = products(&db, automated(json!("1"), json!("2"))).await;
        let card = &cards(&listed, &settings(&[], "productPage"), "en")[0];
        assert_eq!(card["buttons"][0]["label"], "View");
        assert_eq!(
            card["buttons"][0]["beforeStep"]["url"],
            format!("{APP}/shop/{WS}/products/10?token=TOKEN")
        );
    }
}

#[tokio::test]
async fn an_image_that_is_not_a_web_url_is_left_off_the_card() {
    let db = db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Product" SET "images" = '[{"url":"/local/p1.jpg","type":"file"}]' WHERE "id" = 10"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let listed = products(&db, automated(json!("1"), json!("2"))).await;
    let card = &cards(&listed, &settings(&[], "default"), "en")[0];
    assert!(card.get("image").is_none());
}

#[tokio::test]
async fn resolve_is_none_when_no_product_matches() {
    let db = db().await;
    let step = automated(json!("1"), json!("999"));
    assert_eq!(
        resolve_collection_step(&db, WS, 5, None, APP, &step)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn resolve_labels_the_cards_in_the_workspace_language_and_signs_the_links() {
    let db = db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "language" = 'vi' WHERE "id" = ?1"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (5, 'Ada', ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let step = automated(json!("1"), json!("2"));
    let resolved = resolve_collection_step(&db, WS, 5, None, APP, &step)
        .await
        .unwrap()
        .unwrap();
    let button = &resolved["cards"][0]["buttons"][0];
    assert_eq!(button["label"], "Xem");
    let url = button["beforeStep"]["url"].as_str().unwrap();
    let prefix = format!("{APP}/shop/{WS}/products/10?token=");
    assert!(url.starts_with(&prefix), "{url}");
    assert!(url.len() > prefix.len());
}
