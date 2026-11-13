//! Port of `packages/business/__tests__/product-service.test.ts`.
//! Adapted: the mocked repositories become real rows; "update / create was
//! called with" becomes the stored product. The template-lock tests are n-a
//! (no template edit locks in the Rust port; DECISIONS, P3.6-D1).

mod common;

use chatbotx_business::product::{self as svc, AddonInput, FullData, ImageInput, ProductData};
use common::{
    OTHER_WS, WS, category, discount_code, product, scalar_i64, scalar_text, setup, two_workspaces,
};

async fn filing(db: &chatbotx_db::Db, id: i64) -> (Option<i64>, Option<i64>) {
    (
        scalar_i64(
            db,
            r#"SELECT "categoryId" FROM "Product" WHERE "id" = ?1"#,
            id,
        )
        .await,
        scalar_i64(
            db,
            r#"SELECT "subcategoryId" FROM "Product" WHERE "id" = ?1"#,
            id,
        )
        .await,
    )
}

fn patch_category(category_id: Option<i64>) -> ProductData {
    ProductData {
        category_id: Some(category_id),
        ..ProductData::default()
    }
}

fn patch_both(category_id: i64, subcategory_id: i64) -> ProductData {
    ProductData {
        category_id: Some(Some(category_id)),
        subcategory_id: Some(Some(subcategory_id)),
        ..ProductData::default()
    }
}

fn full(name: &str) -> FullData {
    FullData {
        product: ProductData {
            name: Some(name.to_string()),
            ..ProductData::default()
        },
        ..FullData::default()
    }
}

/// One product (id 10) in `WS`, with the given filing.
async fn one_product(category: Option<i64>, subcategory: Option<i64>) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        common::category(tx, 1, WS, None, "Category 1")?;
        common::category(tx, 2, WS, None, "Category 2")?;
        common::category(tx, 3, WS, Some(1), "Sub 1")?;
        common::category(tx, 4, WS, Some(2), "Sub 2")?;
        product(tx, 10, WS, category, subcategory)
    })
    .await;
    db
}

// --- workspace boundaries ---------------------------------------------------

#[tokio::test]
async fn product_service_workspace_boundaries_rejects_a_category_that_does_not_belong_to_the_product_workspace()
 {
    let db = one_product(None, None).await;
    setup(&db, |tx| category(tx, 9, OTHER_WS, None, "Foreign")).await;
    let error = svc::update(&db, WS, 10, patch_category(Some(9)))
        .await
        .expect_err("foreign");
    assert!(error.message().contains("Product category does not exist"));
    assert_eq!(filing(&db, 10).await, (None, None));
}

#[tokio::test]
async fn product_service_workspace_boundaries_stops_before_replacing_child_rows_when_the_product_is_outside_the_workspace()
 {
    let db = one_product(None, None).await;
    setup(&db, |tx| {
        product(tx, 20, OTHER_WS, None, None)?;
        tx.execute(
            r#"INSERT INTO "ProductAddon" ("id", "productId", "name", "addonProductIds") VALUES (21, 20, 'Kept', '[]')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let error = svc::update_full(&db, WS, 20, full("Product"))
        .await
        .expect_err("foreign");
    assert!(error.message().contains("Product does not exist"));
    let addons = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ProductAddon" WHERE "productId" = ?1"#,
        20,
    )
    .await;
    assert_eq!(addons, Some(1));
}

#[tokio::test]
async fn product_service_workspace_boundaries_updates_a_product_after_workspace_scoped_references_are_validated()
 {
    let db = one_product(None, None).await;
    svc::update(&db, WS, 10, patch_category(Some(1)))
        .await
        .expect("update");
    assert_eq!(filing(&db, 10).await, (Some(1), None));
}

#[tokio::test]
async fn product_service_workspace_boundaries_scopes_a_delete_to_the_workspace_that_asked_for_it() {
    let db = one_product(None, None).await;
    setup(&db, |tx| product(tx, 20, OTHER_WS, None, None)).await;
    svc::delete(&db, WS, vec![10, 20]).await.expect("delete");
    let count = r#"SELECT count(*) FROM "Product" WHERE "workspaceId" = ?1"#;
    assert_eq!(
        (
            scalar_i64(&db, count, WS).await,
            scalar_i64(&db, count, OTHER_WS).await
        ),
        (Some(0), Some(1))
    );
}

#[tokio::test]
async fn product_service_workspace_boundaries_removes_deleted_products_from_discount_codes_in_the_same_transaction()
 {
    let db = one_product(None, None).await;
    setup(&db, |tx| {
        product(tx, 11, WS, None, None)?;
        discount_code(tx, 30, WS, "products", None, r#"["10","11"]"#)?;
        discount_code(tx, 31, WS, "products", None, r#"["10"]"#)?;
        Ok(())
    })
    .await;
    svc::delete(&db, WS, vec![10]).await.expect("delete");
    let ids = r#"SELECT "productIds" FROM "DiscountCode" WHERE "id" = ?1"#;
    let active = r#"SELECT "isActive" FROM "DiscountCode" WHERE "id" = ?1"#;
    assert_eq!(
        scalar_text(&db, ids, 30).await.as_deref(),
        Some(r#"["11"]"#)
    );
    assert_eq!(scalar_i64(&db, active, 30).await, Some(1));
    assert_eq!(scalar_text(&db, ids, 31).await.as_deref(), Some("[]"));
    assert_eq!(scalar_i64(&db, active, 31).await, Some(0));
}

// --- status -----------------------------------------------------------------

fn draft() -> ProductData {
    ProductData {
        status: Some("draft".into()),
        ..ProductData::default()
    }
}

#[tokio::test]
async fn product_service_status_refuses_to_change_a_shopify_product_s_status_it_follows_shopify() {
    let db = one_product(None, None).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Product" SET "source" = 'shopify' WHERE "id" = 10"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let error = svc::update(&db, WS, 10, draft())
        .await
        .expect_err("mirrored");
    assert_eq!(
        (error.code(), error.status()),
        (Some("externalStoreManaged"), Some(409))
    );
    let status = scalar_text(&db, r#"SELECT "status" FROM "Product" WHERE "id" = ?1"#, 10).await;
    assert_eq!(status.as_deref(), Some("active"));
}

#[tokio::test]
async fn product_service_status_switches_our_own_product_to_draft() {
    let db = one_product(None, None).await;
    svc::update(&db, WS, 10, draft()).await.expect("draft");
    let status = scalar_text(&db, r#"SELECT "status" FROM "Product" WHERE "id" = ?1"#, 10).await;
    assert_eq!(status.as_deref(), Some("draft"));
}

// --- category change ------------------------------------------------------

#[tokio::test]
async fn product_service_category_change_clears_a_stored_sub_category_that_belongs_to_the_old_category()
 {
    let db = one_product(Some(1), Some(3)).await;
    svc::update(&db, WS, 10, patch_category(Some(2)))
        .await
        .expect("update");
    assert_eq!(filing(&db, 10).await, (Some(2), None));
}

#[tokio::test]
async fn product_service_category_change_clears_the_sub_category_when_the_category_is_set_to_null()
{
    let db = one_product(Some(1), Some(3)).await;
    svc::update(&db, WS, 10, patch_category(None))
        .await
        .expect("update");
    assert_eq!(filing(&db, 10).await, (None, None));
}

#[tokio::test]
async fn product_service_category_change_keeps_the_sub_category_when_it_already_sits_under_the_new_category()
 {
    // Stored inconsistently (sub-category 4 sits under category 2).
    let db = one_product(Some(1), Some(4)).await;
    svc::update(&db, WS, 10, patch_category(Some(2)))
        .await
        .expect("update");
    assert_eq!(filing(&db, 10).await, (Some(2), Some(4)));
}

// --- sub-category parentage -------------------------------------------------

#[tokio::test]
async fn product_service_sub_category_parentage_rejects_a_sub_category_that_does_not_exist_in_the_workspace()
 {
    let db = one_product(None, None).await;
    let error = svc::update(&db, WS, 10, patch_both(1, 404))
        .await
        .expect_err("ghost");
    assert!(
        error
            .message()
            .contains("Product sub-category does not exist")
    );
    assert_eq!(filing(&db, 10).await, (None, None));
}

#[tokio::test]
async fn product_service_sub_category_parentage_rejects_a_sub_category_filed_under_a_different_category()
 {
    let db = one_product(None, None).await;
    let error = svc::update(&db, WS, 10, patch_both(1, 4))
        .await
        .expect_err("other parent");
    assert!(
        error
            .message()
            .contains("does not belong to the selected category")
    );
    assert_eq!(filing(&db, 10).await, (None, None));
}

#[tokio::test]
async fn product_service_sub_category_parentage_accepts_a_sub_category_filed_under_the_selected_category()
 {
    let db = one_product(None, None).await;
    svc::update(&db, WS, 10, patch_both(1, 3))
        .await
        .expect("update");
    assert_eq!(filing(&db, 10).await, (Some(1), Some(3)));
}

#[tokio::test]
async fn product_service_sub_category_parentage_rejects_a_top_level_category_used_as_a_sub_category()
 {
    let db = one_product(None, None).await;
    let error = svc::update(&db, WS, 10, patch_both(1, 2))
        .await
        .expect_err("top level");
    assert!(
        error
            .message()
            .contains("does not belong to the selected category")
    );
}

fn sub_only(subcategory_id: i64) -> ProductData {
    ProductData {
        subcategory_id: Some(Some(subcategory_id)),
        ..ProductData::default()
    }
}

#[tokio::test]
async fn product_service_sub_category_parentage_falls_back_to_the_stored_category_when_a_patch_omits_it()
 {
    let db = one_product(Some(1), None).await;
    svc::update(&db, WS, 10, sub_only(3)).await.expect("update");
    assert_eq!(filing(&db, 10).await, (Some(1), Some(3)));
}

#[tokio::test]
async fn product_service_sub_category_parentage_still_rejects_when_the_stored_category_is_not_the_sub_category_s_parent()
 {
    let db = one_product(Some(2), None).await;
    let error = svc::update(&db, WS, 10, sub_only(3))
        .await
        .expect_err("stored parent differs");
    assert!(
        error
            .message()
            .contains("does not belong to the selected category")
    );
}

#[tokio::test]
async fn product_service_sub_category_parentage_does_not_read_the_product_when_the_patch_touches_no_sub_category_or_locked_content()
 {
    let db = one_product(Some(1), Some(3)).await;
    svc::update(
        &db,
        WS,
        10,
        ProductData {
            rank: Some(3),
            ..ProductData::default()
        },
    )
    .await
    .expect("update");
    let rank = scalar_i64(&db, r#"SELECT "rank" FROM "Product" WHERE "id" = ?1"#, 10).await;
    assert_eq!((rank, filing(&db, 10).await), (Some(3), (Some(1), Some(3))));
}

// --- write shaping ----------------------------------------------------------

#[tokio::test]
async fn product_service_write_shaping_stores_the_form_s_image_mode_as_the_column_s_type() {
    let db = two_workspaces().await;
    let row = svc::create(
        &db,
        WS,
        ProductData {
            name: Some("Product".into()),
            images: Some(vec![ImageInput {
                mode: "link".into(),
                url: "https://example.com/a.png".into(),
            }]),
            ..ProductData::default()
        },
    )
    .await
    .expect("create");
    let images = scalar_text(
        &db,
        r#"SELECT "images" FROM "Product" WHERE "id" = ?1"#,
        row.id,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&images.expect("images")).expect("json"),
        serde_json::json!([{ "type": "link", "url": "https://example.com/a.png" }])
    );
}

#[tokio::test]
async fn product_service_write_shaping_leaves_images_alone_when_a_patch_does_not_mention_them() {
    let db = one_product(None, None).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Product" SET "images" = '[{"type":"link","url":"https://example.com/a.png"}]' WHERE "id" = 10"#,
            [],
        )?;
        Ok(())
    })
    .await;
    svc::update(
        &db,
        WS,
        10,
        ProductData {
            name: Some("Renamed".into()),
            ..ProductData::default()
        },
    )
    .await
    .expect("rename");
    let name = scalar_text(&db, r#"SELECT "name" FROM "Product" WHERE "id" = ?1"#, 10).await;
    let images = scalar_text(&db, r#"SELECT "images" FROM "Product" WHERE "id" = ?1"#, 10).await;
    assert_eq!(name.as_deref(), Some("Renamed"));
    assert!(
        images
            .expect("images")
            .contains("https://example.com/a.png")
    );
}

#[tokio::test]
async fn product_service_write_shaping_validates_references_once_before_inserting_the_full_product()
{
    let db = one_product(None, None).await;
    let mut data = full("Product");
    data.product.category_id = Some(Some(1));
    let row = svc::create_full(&db, WS, data).await.expect("create");
    assert_eq!(row.category_id, Some(1));
}

#[tokio::test]
async fn product_service_write_shaping_rejects_an_addon_product_from_another_workspace_before_inserting()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| product(tx, 20, OTHER_WS, None, None)).await;
    let mut data = full("Product");
    data.addons = vec![AddonInput {
        name: "Extras".into(),
        max_selections: 1,
        addon_product_ids: vec!["20".into()],
    }];
    let error = svc::create_full(&db, WS, data)
        .await
        .expect_err("foreign addon");
    assert!(error.message().contains("addon products do not exist"));
    let count = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "Product" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(count, Some(0));
}
