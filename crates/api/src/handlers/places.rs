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
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "placeCategories.list" => match category::list(db, ws).await {
            Ok(rows) => Ok(json(
                200,
                json!({ "data": rows.iter().map(category_list_json).collect::<Vec<_>>() }),
            )),
            Err(error) => Err(business_error(error, super::LISTING)),
        },
        "placeCategories.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let category_id = input.id("id");
            input.finish()?;
            category::find(db, ws, category_id.unwrap_or_default())
                .await
                .map(|row| json(200, category_json(&row)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "placeCategories.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", NAME);
            let rank = input.opt_number("rank", RANK);
            input.finish()?;
            category::create(
                db,
                CreatePlaceCategory {
                    workspace_id: ws,
                    name: name.unwrap_or_default(),
                    rank: rank.map(|r| r as i64),
                },
            )
            .await
            .map(|row| json(200, category_json(&row)))
            .map_err(|e| business_error(e, CREATING_PLACE_CATEGORY))
        }
        "placeCategories.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.opt_string("name", NAME);
            let rank = input.opt_number("rank", RANK);
            let category_id = input.id("id");
            input.require_one_of(&["name", "rank"]);
            input.finish()?;
            category::update(
                db,
                UpdatePlaceCategory {
                    workspace_id: ws,
                    category_id: category_id.unwrap_or_default(),
                    name,
                    rank: rank.map(|r| r as i64),
                },
            )
            .await
            .map(|row| json(200, category_json(&row)))
            .map_err(|e| business_error(e, MUTATING_PLACE_CATEGORY))
        }
        "placeCategories.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let category_id = input.id("id");
            input.finish()?;
            category::delete(db, ws, category_id.unwrap_or_default())
                .await
                .map(|out| {
                    json(
                        200,
                        json!({ "uncategorizedLocationCount": out.uncategorized_location_count }),
                    )
                })
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "placeLocations.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let name = input.opt_string(
                "name",
                Str {
                    trim: true,
                    ..Str::max(255)
                },
            );
            let page = input.opt_coerce_int(
                "page",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
            input.finish()?;
            location::list(
                db,
                ListPlaceLocations {
                    workspace_id: ws,
                    name,
                    page,
                    per_page,
                },
            )
            .await
            .map(|list| {
                json(
                    200,
                    json!({
                        "data": list.data.iter().map(location_json).collect::<Vec<_>>(),
                        "pageCount": list.page_count,
                    }),
                )
            })
            .map_err(|e| business_error(e, super::LISTING))
        }
        "placeLocations.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let location_id = input.id("id");
            input.finish()?;
            location::find(db, ws, location_id.unwrap_or_default())
                .await
                .map(|item| json(200, location_json(&item)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "placeLocations.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", NAME);
            let latitude = input.number("latitude", LATITUDE);
            let longitude = input.number("longitude", LONGITUDE);
            let category_id = input.nullish_id("categoryId");
            input.finish()?;
            location::create(
                db,
                CreatePlaceLocation {
                    workspace_id: ws,
                    name: name.unwrap_or_default(),
                    latitude: latitude.unwrap_or_default(),
                    longitude: longitude.unwrap_or_default(),
                    category_id: category_id.flatten(),
                },
            )
            .await
            .map(|item| json(200, location_json(&item)))
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "placeLocations.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.opt_string("name", NAME);
            let latitude = input.opt_number("latitude", LATITUDE);
            let longitude = input.opt_number("longitude", LONGITUDE);
            let category_id = input.nullish_id("categoryId");
            let location_id = input.id("id");
            input.require_one_of(&["name", "latitude", "longitude", "categoryId"]);
            input.finish()?;
            location::update(
                db,
                UpdatePlaceLocation {
                    workspace_id: ws,
                    location_id: location_id.unwrap_or_default(),
                    name,
                    latitude,
                    longitude,
                    category_id,
                },
            )
            .await
            .map(|item| json(200, location_json(&item)))
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "placeLocations.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let location_id = input.id("id");
            input.finish()?;
            location::delete(db, ws, location_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
