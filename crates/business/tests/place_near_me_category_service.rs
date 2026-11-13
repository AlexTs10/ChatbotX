//! Port of `packages/business/src/place-near-me/__tests__/category-service.test.ts`.
//! Adapted: the mocked repository becomes real rows of a migrated SQLite file;
//! the mocked unique violation is a real existing name.

mod common;

use chatbotx_business::place_near_me::category::{self, CreatePlaceCategory, UpdatePlaceCategory};
use chatbotx_business::place_near_me::location::{self, CreatePlaceLocation};
use common::{WS, scalar_i64, two_workspaces};

fn create_input(name: &str) -> CreatePlaceCategory {
    CreatePlaceCategory {
        workspace_id: WS,
        name: name.to_string(),
        rank: None,
    }
}

#[tokio::test]
async fn place_category_service_create_trims_the_name_and_maps_a_unique_violation_to_place_category_duplicated()
 {
    let db = two_workspaces().await;
    let first = category::create(&db, create_input("  Stores "))
        .await
        .expect("create");
    assert_eq!(first.name, "Stores");
    let error = category::create(&db, create_input("  Stores "))
        .await
        .expect_err("duplicate");
    assert_eq!(error.code(), Some("placeCategoryDuplicated"));
    assert_eq!(error.status(), Some(409));
}

#[tokio::test]
async fn place_category_service_create_rejects_a_blank_name() {
    let db = two_workspaces().await;
    let error = category::create(&db, create_input("   "))
        .await
        .expect_err("blank");
    assert_eq!(error.code(), Some("placeCategoryNameRequired"));
    assert_eq!(error.status(), Some(400));
    let count = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "PlaceCategory" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(count, Some(0));
}

#[tokio::test]
async fn place_category_service_update_renaming_to_a_different_case_of_its_own_name_succeeds() {
    let db = two_workspaces().await;
    let row = category::create(&db, create_input("Stores"))
        .await
        .expect("create");
    let updated = category::update(
        &db,
        UpdatePlaceCategory {
            workspace_id: WS,
            category_id: row.id,
            name: Some("STORES".to_string()),
            rank: None,
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.name, "STORES");
}

#[tokio::test]
async fn place_category_service_update_of_a_missing_category_throws_not_found() {
    let db = two_workspaces().await;
    let error = category::update(
        &db,
        UpdatePlaceCategory {
            workspace_id: WS,
            category_id: 999,
            name: None,
            rank: Some(2),
        },
    )
    .await
    .expect_err("missing");
    assert_eq!(error.message(), "Place category not found");
}

#[tokio::test]
async fn place_category_service_delete_returns_how_many_locations_became_uncategorized() {
    let db = two_workspaces().await;
    let row = category::create(&db, create_input("Stores"))
        .await
        .expect("create");
    for index in 0..3 {
        location::create(
            &db,
            CreatePlaceLocation {
                workspace_id: WS,
                name: format!("Shop {index}"),
                latitude: 1.0,
                longitude: 2.0,
                category_id: Some(row.id),
            },
        )
        .await
        .expect("location");
    }
    let deleted = category::delete(&db, WS, row.id).await.expect("delete");
    assert_eq!(deleted.uncategorized_location_count, 3);
}

#[tokio::test]
async fn place_category_service_delete_of_a_missing_category_throws_not_found() {
    let db = two_workspaces().await;
    let error = category::delete(&db, WS, 999).await.expect_err("missing");
    assert_eq!(error.message(), "Place category not found");
}
