//! `features/places-near-me/api/public.ts`: `placeCategories.*` and `placeLocations.*`.

use chatbotx_business::place_near_me::category::{self, CreatePlaceCategory, UpdatePlaceCategory};
use chatbotx_business::place_near_me::location::{
    self, CreatePlaceLocation, ListPlaceLocations, UpdatePlaceLocation,
};
use chatbotx_business::place_near_me::{PlaceCategoryListItem, PlaceLocationItem};
use chatbotx_db::repo::models::PlaceCategory;
use serde_json::{Value, json};

use super::input::{Input, Num, Str};
use super::{
    ApiState, Call, ErrorMap, Reply, business_error, date, id, json, no_content, number, opt_id,
};

/// `placeCategoryErrors` on top of the create / mutate maps.
const CREATING_PLACE_CATEGORY: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("placeCategoryDuplicated", 409),
    ("placeCategoryNameRequired", 400),
];
const MUTATING_PLACE_CATEGORY: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("placeCategoryDuplicated", 409),
    ("placeCategoryNameRequired", 400),
];

/// `MAX_RANK`.
const MAX_RANK: f64 = 2_147_483_647.0;
const NAME: Str = Str::trimmed(1, 255);
const RANK: Num = Num::int(1.0, MAX_RANK);
const LATITUDE: Num = Num::range(-90.0, 90.0);
const LONGITUDE: Num = Num::range(-180.0, 180.0);

/// `placeCategoryWriteResource`.
fn category_json(row: &PlaceCategory) -> Value {
    json!({
        "id": id(row.id),
        "name": row.name,
        "rank": row.rank,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    })
}

/// `placeCategoryResource`.
fn category_list_json(row: &PlaceCategoryListItem) -> Value {
    json!({
        "id": id(row.id),
        "name": row.name,
        "rank": row.rank,
        "locationCount": row.location_count,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    })
}

/// `placeLocationResource`.
fn location_json(item: &PlaceLocationItem) -> Value {
    json!({
        "id": id(item.id),
        "name": item.name,
        "latitude": number(item.latitude),
        "longitude": number(item.longitude),
        "categoryId": opt_id(item.category_id),
        "categoryName": item.category_name,
        "previewUrl": item.preview_url,
        "createdAt": date(item.created_at),
        "updatedAt": date(item.updated_at),
    })
}

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "placeCategories.list",
    "placeCategories.get",
    "placeCategories.create",
    "placeCategories.update",
    "placeCategories.delete",
    "placeLocations.list",
    "placeLocations.get",
    "placeLocations.create",
    "placeLocations.update",
    "placeLocations.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    todo!("P3.6-D1")
}
