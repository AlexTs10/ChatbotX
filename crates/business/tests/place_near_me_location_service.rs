//! Port of `packages/business/src/place-near-me/__tests__/location-service.test.ts`.
//! Adapted: the mocked repositories become real rows of a migrated SQLite file;
//! "the repository was (not) called" becomes the observable row state. The
//! cache invalidation assertion is dropped (no read cache in the Rust port).

mod common;

use chatbotx_business::place_near_me::PlaceLocationItem;
use chatbotx_business::place_near_me::category::{self, CreatePlaceCategory};
use chatbotx_business::place_near_me::location::{
    self, CreatePlaceLocation, FindNearestPlace, ListPlaceLocations, UpdatePlaceLocation,
};
use common::{OTHER_WS, WS, scalar_i64, two_workspaces};

const COUNT_WS: &str = r#"SELECT count(*) FROM "PlaceLocation" WHERE "workspaceId" = ?1"#;

async fn new_category(db: &chatbotx_db::Db, workspace_id: i64, name: &str) -> i64 {
    category::create(
        db,
        CreatePlaceCategory {
            workspace_id,
            name: name.to_string(),
            rank: None,
        },
    )
    .await
    .expect("category")
    .id
}

async fn new_location(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    name: &str,
    latitude: f64,
    longitude: f64,
    category_id: Option<i64>,
) -> PlaceLocationItem {
    location::create(
        db,
        CreatePlaceLocation {
            workspace_id,
            name: name.to_string(),
            latitude,
            longitude,
            category_id,
        },
    )
    .await
    .expect("location")
}

#[tokio::test]
async fn place_location_service_create_rejects_a_category_from_another_workspace() {
    let db = two_workspaces().await;
    let foreign = new_category(&db, OTHER_WS, "Other").await;
    let error = location::create(
        &db,
        CreatePlaceLocation {
            workspace_id: WS,
            name: "HQ".to_string(),
            latitude: 1.0,
            longitude: 2.0,
            category_id: Some(foreign),
        },
    )
    .await
    .expect_err("foreign category");
    assert_eq!(error.message(), "Place category does not exist.");
    assert_eq!(scalar_i64(&db, COUNT_WS, WS).await, Some(0));
}

#[tokio::test]
async fn place_location_service_create_with_category_id_null_skips_the_category_check() {
    let db = two_workspaces().await;
    let item = new_location(&db, WS, "HQ", 10.5, 20.25, None).await;
    assert_eq!(item.category_id, None);
    assert_eq!(item.category_name, None);
    assert_eq!(
        item.preview_url,
        "https://www.google.com/maps/search/?api=1&query=10.5%2C20.25"
    );
}

#[tokio::test]
async fn place_location_service_list_adds_preview_url_to_every_row_and_passes_name_page_per_page_through()
 {
    let db = two_workspaces().await;
    for index in 0..21 {
        new_location(&db, WS, &format!("HQ {index}"), 10.5, 20.25, None).await;
    }
    new_location(&db, WS, "Depot", 1.0, 1.0, None).await;
    let result = location::list(
        &db,
        ListPlaceLocations {
            workspace_id: WS,
            name: Some("hq".to_string()),
            page: Some(2),
            per_page: Some(10),
        },
    )
    .await
    .expect("list");
    assert_eq!(result.page_count, 3);
    assert_eq!(result.data.len(), 10);
    assert!(result.data[0].preview_url.contains("query=10.5%2C20.25"));
    assert!(result.data.iter().all(|item| item.name.starts_with("HQ ")));
}

#[tokio::test]
async fn place_location_service_update_of_a_missing_location_throws_not_found() {
    let db = two_workspaces().await;
    let error = location::update(
        &db,
        UpdatePlaceLocation {
            workspace_id: WS,
            location_id: 999,
            name: Some("X".to_string()),
            ..UpdatePlaceLocation::default()
        },
    )
    .await
    .expect_err("missing");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn place_location_service_update_with_empty_data_does_not_reach_the_repository() {
    let db = two_workspaces().await;
    let row = new_location(&db, WS, "HQ", 10.5, 20.25, None).await;
    let item = location::update(
        &db,
        UpdatePlaceLocation {
            workspace_id: WS,
            location_id: row.id,
            ..UpdatePlaceLocation::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(item.id, row.id);
    assert_eq!(item, row);
}

#[tokio::test]
async fn place_location_service_delete_of_a_missing_location_throws_not_found() {
    let db = two_workspaces().await;
    let error = location::delete(&db, WS, 999).await.expect_err("missing");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn place_location_service_create_rejects_a_blank_name() {
    let db = two_workspaces().await;
    let error = location::create(
        &db,
        CreatePlaceLocation {
            workspace_id: WS,
            name: "   ".to_string(),
            latitude: 1.0,
            longitude: 2.0,
            category_id: None,
        },
    )
    .await
    .expect_err("blank");
    assert_eq!(error.code(), Some("placeLocationNameRequired"));
    assert_eq!(error.status(), Some(400));
    assert_eq!(scalar_i64(&db, COUNT_WS, WS).await, Some(0));
}

#[tokio::test]
async fn place_location_service_update_rejects_a_blank_name() {
    let db = two_workspaces().await;
    let row = new_location(&db, WS, "HQ", 10.5, 20.25, None).await;
    let error = location::update(
        &db,
        UpdatePlaceLocation {
            workspace_id: WS,
            location_id: row.id,
            name: Some(" ".to_string()),
            ..UpdatePlaceLocation::default()
        },
    )
    .await
    .expect_err("blank");
    assert_eq!(error.code(), Some("placeLocationNameRequired"));
    assert_eq!(location::find(&db, WS, row.id).await.expect("find"), row);
}

#[tokio::test]
async fn place_location_service_update_rejects_a_category_from_another_workspace_without_updating()
{
    let db = two_workspaces().await;
    let row = new_location(&db, WS, "HQ", 10.5, 20.25, None).await;
    let foreign = new_category(&db, OTHER_WS, "Other").await;
    let error = location::update(
        &db,
        UpdatePlaceLocation {
            workspace_id: WS,
            location_id: row.id,
            category_id: Some(Some(foreign)),
            ..UpdatePlaceLocation::default()
        },
    )
    .await
    .expect_err("foreign category");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(location::find(&db, WS, row.id).await.expect("find"), row);
}

#[tokio::test]
async fn place_location_service_scopes_by_workspace_and_category() {
    let db = two_workspaces().await;
    let seven = new_category(&db, WS, "Seven").await;
    let other_category = new_category(&db, WS, "Elsewhere").await;
    new_location(&db, WS, "Far", 21.0285, 105.8542, Some(seven)).await;
    let near = new_location(&db, WS, "Near", 10.8231, 106.6297, Some(seven)).await;
    let closer = new_location(&db, WS, "Closer", 10.8, 106.7, Some(other_category)).await;
    new_location(&db, OTHER_WS, "Foreign", 10.8, 106.7, None).await;
    let nearest = location::find_nearest(
        &db,
        FindNearestPlace {
            workspace_id: WS,
            category_id: Some(seven),
            latitude: 10.8,
            longitude: 106.7,
        },
    )
    .await
    .expect("nearest")
    .expect("a place");
    assert_eq!((nearest.id, nearest.name.as_str()), (near.id, "Near"));
    let any = location::find_nearest(
        &db,
        FindNearestPlace {
            workspace_id: WS,
            category_id: None,
            latitude: 10.8,
            longitude: 106.7,
        },
    )
    .await
    .expect("nearest")
    .expect("a place");
    assert_eq!(any.id, closer.id);
}

#[tokio::test]
async fn place_location_service_find_nearest_returns_null_when_the_repository_returns_empty() {
    let db = two_workspaces().await;
    let nearest = location::find_nearest(
        &db,
        FindNearestPlace {
            workspace_id: WS,
            category_id: None,
            latitude: 1.0,
            longitude: 2.0,
        },
    )
    .await
    .expect("nearest");
    assert_eq!(nearest, None);
}
