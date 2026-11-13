//! Port of `apps/builder/__tests__/places-near-me-public-api.test.ts`.
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over a
//! real migrated database; "the service was called with" becomes the rows and
//! JSON the call reads or writes.

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

fn id_of(value: &Value) -> String {
    value["id"].as_str().expect("id").to_string()
}

async fn new_category(f: &handlers_common::Fixture, name: &str, rank: i64) -> Value {
    let (status, body) = call(
        f,
        "placeCategories.create",
        WS,
        &[],
        None,
        Some(json!({ "name": name, "rank": rank })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn new_location(f: &handlers_common::Fixture, body: Value) -> Value {
    let (status, out) = call(f, "placeLocations.create", WS, &[], None, Some(body)).await;
    assert_eq!(status, 200, "{out}");
    out
}

#[tokio::test]
async fn registers_under_the_places_scope() {
    assert_eq!(
        scopes_of_module("features/places-near-me/api/public"),
        vec!["places"]
    );
}

#[tokio::test]
async fn place_categories_public_api_get_list_wraps_the_service_result_in_data() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    new_location(
        &f,
        json!({ "name": "A", "latitude": 1, "longitude": 2, "categoryId": id_of(&cafe) }),
    )
    .await;
    new_location(
        &f,
        json!({ "name": "B", "latitude": 1, "longitude": 2, "categoryId": id_of(&cafe) }),
    )
    .await;
    let (status, body) = call(&f, "placeCategories.list", WS, &[], None, None).await;
    assert_eq!(status, 200);
    let data = body["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], cafe["id"]);
    assert_eq!(data[0]["locationCount"], json!(2));
    let (_, other) = call(&f, "placeCategories.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [] }));
}

#[tokio::test]
async fn place_categories_public_api_get_one() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    let id = id_of(&cafe);
    let (status, body) = call(&f, "placeCategories.get", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, body), (200, cafe));
    let (status, _) = call(
        &f,
        "placeCategories.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn place_categories_public_api_post_create() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    assert_eq!(
        (cafe["name"].clone(), cafe["rank"].clone()),
        (json!("Cafe"), json!(3))
    );
}

#[tokio::test]
async fn place_categories_public_api_patch_update() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    let id = id_of(&cafe);
    let (status, body) = call(
        &f,
        "placeCategories.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "rank": 2 })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["name"].clone(), body["rank"].clone()),
        (json!("Cafe"), json!(2))
    );
}

#[tokio::test]
async fn place_categories_public_api_delete_returns_the_uncategorized_count() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    let id = id_of(&cafe);
    for name in ["A", "B"] {
        new_location(
            &f,
            json!({ "name": name, "latitude": 1, "longitude": 2, "categoryId": id }),
        )
        .await;
    }
    let (status, body) = call(&f, "placeCategories.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(
        (status, body),
        (200, json!({ "uncategorizedLocationCount": 2 }))
    );
}

#[tokio::test]
async fn place_locations_public_api_get_list_passes_filter_and_paging() {
    let f = fixture().await;
    for index in 0..11 {
        new_location(
            &f,
            json!({ "name": format!("HQ {index}"), "latitude": 1, "longitude": 2 }),
        )
        .await;
    }
    new_location(
        &f,
        json!({ "name": "Depot", "latitude": 1, "longitude": 2 }),
    )
    .await;
    let (status, body) = call(
        &f,
        "placeLocations.list",
        WS,
        &[],
        Some("name=hq&page=2&perPage=5"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(3));
    assert_eq!(body["data"].as_array().map(Vec::len), Some(5));
}

#[tokio::test]
async fn place_locations_public_api_get_one() {
    let f = fixture().await;
    let hq = new_location(
        &f,
        json!({ "name": "HQ", "latitude": 1.5, "longitude": 2.5 }),
    )
    .await;
    let id = id_of(&hq);
    let (status, body) = call(&f, "placeLocations.get", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, body), (200, hq));
    let (status, _) = call(
        &f,
        "placeLocations.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn place_locations_public_api_post_create() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    let hq = new_location(
        &f,
        json!({ "name": "HQ", "latitude": 1.5, "longitude": 2.5, "categoryId": id_of(&cafe) }),
    )
    .await;
    assert_eq!(hq["name"], json!("HQ"));
    assert_eq!(hq["latitude"], json!(1.5));
    assert_eq!(hq["longitude"], json!(2.5));
    assert_eq!(hq["categoryId"], cafe["id"]);
    assert_eq!(hq["categoryName"], json!("Cafe"));
}

#[tokio::test]
async fn place_locations_public_api_patch_update() {
    let f = fixture().await;
    let cafe = new_category(&f, "Cafe", 3).await;
    let hq = new_location(
        &f,
        json!({ "name": "HQ", "latitude": 1.5, "longitude": 2.5, "categoryId": id_of(&cafe) }),
    )
    .await;
    let id = id_of(&hq);
    let (status, body) = call(
        &f,
        "placeLocations.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "categoryId": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["categoryId"].clone(), body["categoryName"].clone()),
        (Value::Null, Value::Null)
    );
    assert_eq!(body["name"], json!("HQ"));
}

#[tokio::test]
async fn place_locations_public_api_delete() {
    let f = fixture().await;
    let hq = new_location(
        &f,
        json!({ "name": "HQ", "latitude": 1.5, "longitude": 2.5 }),
    )
    .await;
    let id = id_of(&hq);
    let (status, body) = call(&f, "placeLocations.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, body), (204, Value::Null));
    let (status, _) = call(&f, "placeLocations.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}
