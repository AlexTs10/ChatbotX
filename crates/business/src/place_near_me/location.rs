//! `place-near-me/location.service.ts` (`placeLocationService`).
//!
//! The original invalidates the `place-locations:` / `place-categories:` cache
//! tags after each write; the Rust port has no read cache.

use chatbotx_core::js;
use chatbotx_db::Db;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::place::{self, PlaceLocationWithCategory};
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use super::nearest::{Coordinates, NearestPlace, PlaceCoordinates, pick_nearest_place};
use super::preview_url::build_google_maps_preview_url;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `PlaceLocationItem`: a location with its category name and preview link.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLocationItem {
    pub id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub preview_url: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input of [`list`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPlaceLocations {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Result of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLocationList {
    pub data: Vec<PlaceLocationItem>,
    pub page_count: i64,
}

/// Input of [`create`].
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePlaceLocation {
    pub workspace_id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub category_id: Option<i64>,
}

/// Input of [`update`] (`None` = not sent; `category_id: Some(None)` clears it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdatePlaceLocation {
    pub workspace_id: i64,
    pub location_id: i64,
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub category_id: Option<Option<i64>>,
}

/// Input of [`find_nearest`] (`category_id` `None` = every category).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FindNearestPlace {
    pub workspace_id: i64,
    pub category_id: Option<i64>,
    pub latitude: f64,
    pub longitude: f64,
}

fn to_item(row: PlaceLocationWithCategory) -> PlaceLocationItem {
    let location = row.location;
    PlaceLocationItem {
        preview_url: build_google_maps_preview_url(location.latitude, location.longitude),
        id: location.id,
        name: location.name,
        latitude: location.latitude,
        longitude: location.longitude,
        category_id: location.category_id,
        category_name: row.category_name,
        created_at: location.created_at,
        updated_at: location.updated_at,
    }
}

fn name_required() -> BusinessError {
    BusinessError::exception(
        "Place location name is required",
        "placeLocationNameRequired",
        400,
    )
}

fn normalize_name(name: &str) -> Result<String> {
    let trimmed = js::trim(name);
    if trimmed.is_empty() {
        return Err(name_required());
    }
    Ok(trimmed.to_string())
}

fn location_not_found() -> BusinessError {
    BusinessError::not_found("Place location not found")
}

fn find_in(conn: &Connection, workspace_id: i64, location_id: i64) -> Result<PlaceLocationItem> {
    place::find_location(conn, workspace_id, location_id)?
        .map(to_item)
        .ok_or_else(location_not_found)
}

/// `assertCategory`: a category id must name a category of this workspace.
fn assert_category(conn: &Connection, workspace_id: i64, category_id: Option<i64>) -> Result<()> {
    let Some(category_id) = category_id else {
        return Ok(());
    };
    match place::find_category(conn, workspace_id, category_id)? {
        Some(_) => Ok(()),
        None => Err(BusinessError::not_found("Place category does not exist.")),
    }
}

/// `placeLocationService.list`: newest first; without paging every row (`pageCount` 1).
pub async fn list(db: &Db, input: ListPlaceLocations) -> Result<PlaceLocationList> {
    let page = tx::read(db, move |conn| {
        Ok(place::list_locations(
            conn,
            input.workspace_id,
            input.name.as_deref(),
            PaginationInput {
                page: input.page,
                per_page: input.per_page,
            },
        )?)
    })
    .await?;
    Ok(PlaceLocationList {
        data: page.data.into_iter().map(to_item).collect(),
        page_count: page.page_count,
    })
}

/// `placeLocationService.find`.
pub async fn find(db: &Db, workspace_id: i64, location_id: i64) -> Result<PlaceLocationItem> {
    tx::read(db, move |conn| find_in(conn, workspace_id, location_id)).await
}

/// `placeLocationService.create`.
pub async fn create(db: &Db, input: CreatePlaceLocation) -> Result<PlaceLocationItem> {
    let name = normalize_name(&input.name)?;
    tx::write(db, move |conn| {
        assert_category(conn, input.workspace_id, input.category_id)?;
        let values: Patch = vec![
            ("name", Value::Text(name)),
            ("latitude", Value::Real(input.latitude)),
            ("longitude", Value::Real(input.longitude)),
            (
                "categoryId",
                input.category_id.map_or(Value::Null, Value::Integer),
            ),
        ];
        let created =
            place::create_location(conn, input.workspace_id, &values)?.ok_or_else(|| {
                BusinessError::exception("Failed to create place location", "systemError", 400)
            })?;
        find_in(conn, input.workspace_id, created.id)
    })
    .await
}

/// `placeLocationService.update`: only the sent fields change; nothing sent = the row as is.
pub async fn update(db: &Db, input: UpdatePlaceLocation) -> Result<PlaceLocationItem> {
    let mut values: Patch = Vec::new();
    if let Some(name) = &input.name {
        values.push(("name", Value::Text(normalize_name(name)?)));
    }
    if let Some(latitude) = input.latitude {
        values.push(("latitude", Value::Real(latitude)));
    }
    if let Some(longitude) = input.longitude {
        values.push(("longitude", Value::Real(longitude)));
    }
    if let Some(category_id) = input.category_id {
        values.push((
            "categoryId",
            category_id.map_or(Value::Null, Value::Integer),
        ));
    }
    let (workspace_id, location_id) = (input.workspace_id, input.location_id);
    if values.is_empty() {
        return find(db, workspace_id, location_id).await;
    }
    tx::write(db, move |conn| {
        assert_category(conn, workspace_id, input.category_id.flatten())?;
        place::update_location(conn, workspace_id, location_id, &values)?
            .ok_or_else(location_not_found)?;
        find_in(conn, workspace_id, location_id)
    })
    .await
}

/// `placeLocationService.delete`.
pub async fn delete(db: &Db, workspace_id: i64, location_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        place::delete_location(conn, workspace_id, location_id)?
            .map(|_| ())
            .ok_or_else(location_not_found)
    })
    .await
}

/// `placeLocationService.findNearest`: the location closest to a point, in one
/// category or all; `None` if there is none.
pub async fn find_nearest(db: &Db, input: FindNearestPlace) -> Result<Option<NearestPlace>> {
    let rows = tx::read(db, move |conn| {
        Ok(place::list_location_coordinates(
            conn,
            input.workspace_id,
            input.category_id,
        )?)
    })
    .await?;
    let places: Vec<PlaceCoordinates> = rows
        .into_iter()
        .map(|(id, name, latitude, longitude)| PlaceCoordinates {
            id,
            name,
            latitude,
            longitude,
        })
        .collect();
    Ok(pick_nearest_place(
        Coordinates {
            latitude: input.latitude,
            longitude: input.longitude,
        },
        &places,
    ))
}
