//! Port of `packages/business/src/vendor/__tests__/vendor-service.test.ts`.
//! Adapted: the mocked repositories become real rows; "update was called
//! with" becomes the stored vendor.

mod common;

use chatbotx_business::tx;
use chatbotx_business::vendor::{self as svc, VendorData};
use common::{OTHER_WS, WS, category, open_hours, scalar_i64, setup, two_workspaces, vendor};

fn named(name: &str) -> VendorData {
    VendorData {
        name: Some(name.to_string()),
        ..VendorData::default()
    }
}

async fn vendors_in(db: &chatbotx_db::Db) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "Vendor" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or(0)
}

#[tokio::test]
async fn vendor_service_resolve_by_names_creates_the_missing_names_once_and_keys_ids_by_lower_cased_name()
 {
    let db = two_workspaces().await;
    let names: Vec<String> = [" Acme ", "acme", "Globex", ""]
        .iter()
        .map(|n| n.to_string())
        .collect();
    let ids = tx::write(&db, move |tx| svc::resolve_by_names(tx, WS, &names, true))
        .await
        .expect("resolve");
    let mut keys: Vec<&String> = ids.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["acme", "globex"]);
    assert_eq!(vendors_in(&db).await, 2);
}

#[tokio::test]
async fn vendor_service_resolve_by_names_only_looks_up_when_it_may_not_create() {
    let db = two_workspaces().await;
    let names = vec!["Acme".to_string()];
    let ids = tx::write(&db, move |tx| svc::resolve_by_names(tx, WS, &names, false))
        .await
        .expect("resolve");
    assert!(ids.is_empty());
    assert_eq!(vendors_in(&db).await, 0);
}

#[tokio::test]
async fn vendor_service_resolve_by_names_resolves_an_empty_name_to_no_vendor() {
    let db = two_workspaces().await;
    let id = tx::write(&db, |tx| svc::resolve_name(tx, WS, Some("  ")))
        .await
        .expect("resolve");
    assert_eq!((id, vendors_in(&db).await), (None, 0));
}

#[tokio::test]
async fn vendor_service_create_turns_a_duplicate_name_into_product_vendor_duplicated() {
    let db = two_workspaces().await;
    svc::create(&db, WS, named("Acme")).await.expect("first");
    let error = svc::create(&db, WS, named("ACME"))
        .await
        .expect_err("duplicate");
    assert_eq!(
        (error.code(), error.message(), error.status()),
        (
            Some("productVendorDuplicated"),
            "Vendor already exists".to_string(),
            Some(400)
        )
    );
}

#[tokio::test]
async fn vendor_service_create_refuses_a_blank_name() {
    let db = two_workspaces().await;
    let error = svc::create(&db, WS, named("   ")).await.expect_err("blank");
    assert_eq!(error.code(), Some("productVendorNameRequired"));
    assert_eq!(vendors_in(&db).await, 0);
}

#[tokio::test]
async fn vendor_service_references_refuses_a_category_of_another_workspace() {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 9, OTHER_WS, None, "Foreign")).await;
    let error = svc::create(
        &db,
        WS,
        VendorData {
            category_id: Some(Some(9)),
            ..named("Acme")
        },
    )
    .await
    .expect_err("foreign category");
    assert_eq!(
        (error.code(), error.message()),
        (
            Some("notFound"),
            "Product category does not exist.".to_string()
        )
    );
    assert_eq!(vendors_in(&db).await, 0);
}

#[tokio::test]
async fn vendor_service_references_refuses_a_sub_category_under_another_category() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "One")?;
        category(tx, 99, WS, None, "Other")?;
        category(tx, 2, WS, Some(99), "Child of other")
    })
    .await;
    let error = svc::create(
        &db,
        WS,
        VendorData {
            category_id: Some(Some(1)),
            subcategory_id: Some(Some(2)),
            ..named("Acme")
        },
    )
    .await
    .expect_err("wrong parent");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn vendor_service_references_checks_the_open_hours_schedule_belongs_to_the_workspace() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        open_hours(tx, 5, WS)?;
        open_hours(tx, 6, OTHER_WS)
    })
    .await;
    let own = svc::create(
        &db,
        WS,
        VendorData {
            open_hours_id: Some(Some(5)),
            ..named("Acme")
        },
    )
    .await
    .expect("own schedule");
    assert_eq!(own.open_hours_id, Some(5));
    let error = svc::create(
        &db,
        WS,
        VendorData {
            open_hours_id: Some(Some(6)),
            ..named("Globex")
        },
    )
    .await
    .expect_err("foreign schedule");
    assert_eq!(error.message(), "Open-hours schedule does not exist.");
}

async fn vendor_filed(sub_parent: i64) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        category(tx, 1, WS, None, "Old")?;
        category(tx, 2, WS, None, "New")?;
        category(tx, 3, WS, Some(sub_parent), "Child")?;
        vendor(tx, 10, WS, "Acme")?;
        tx.execute(
            r#"UPDATE "Vendor" SET "categoryId" = 1, "subcategoryId" = 3 WHERE "id" = 10"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn vendor_service_update_category_consistency_clears_a_stale_sub_category_when_the_category_changes_alone()
 {
    let db = vendor_filed(1).await;
    let row = svc::update(
        &db,
        WS,
        10,
        VendorData {
            category_id: Some(Some(2)),
            ..VendorData::default()
        },
    )
    .await
    .expect("update");
    assert_eq!((row.category_id, row.subcategory_id), (Some(2), None));
}

#[tokio::test]
async fn vendor_service_update_category_consistency_keeps_the_sub_category_when_it_sits_under_the_new_category()
 {
    let db = vendor_filed(2).await;
    let row = svc::update(
        &db,
        WS,
        10,
        VendorData {
            category_id: Some(Some(2)),
            ..VendorData::default()
        },
    )
    .await
    .expect("update");
    assert_eq!((row.category_id, row.subcategory_id), (Some(2), Some(3)));
}

#[tokio::test]
async fn vendor_service_update_category_consistency_checks_a_sub_category_sent_alone_against_the_stored_category()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Cat")?;
        category(tx, 2, WS, None, "Other")?;
        category(tx, 3, WS, Some(2), "Sub of other")?;
        category(tx, 4, WS, Some(1), "Sub of cat")?;
        vendor(tx, 10, WS, "Acme")?;
        tx.execute(
            r#"UPDATE "Vendor" SET "categoryId" = 1 WHERE "id" = 10"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let sub = |id: i64| VendorData {
        subcategory_id: Some(Some(id)),
        ..VendorData::default()
    };
    let error = svc::update(&db, WS, 10, sub(3))
        .await
        .expect_err("other parent");
    assert!(error.message().contains("does not belong"));
    let row = svc::update(&db, WS, 10, sub(4)).await.expect("own parent");
    assert_eq!(row.subcategory_id, Some(4));
}
