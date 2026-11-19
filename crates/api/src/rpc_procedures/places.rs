//! `app/space/[workspaceId]/places-near-me` and
//! `features/places-near-me/actions`: the categories / locations tabs of the
//! SPA (`placeCategoryService`, `placeLocationService`). Membership is the
//! `/rpc` check; the inputs are `placeCategoryFormSchema` /
//! `placeLocationFormSchema` (the public API fields).

use chatbotx_business::BusinessError;
use chatbotx_business::place_near_me::category::{self, CreatePlaceCategory, UpdatePlaceCategory};
use chatbotx_business::place_near_me::location::{
    self, CreatePlaceLocation, ListPlaceLocations, UpdatePlaceLocation,
};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg};
use crate::handlers::input::Input;
use crate::handlers::places::{
    LATITUDE, LONGITUDE, NAME, RANK, category_json, category_list_json, location_json,
};
use crate::orpc::OrpcError;

const ACTIONS: &[&str] = &[
    "listPlaceCategories",
    "listPlaceLocations",
    "createPlaceCategoryAction",
    "updatePlaceCategoryAction",
    "deletePlaceCategoryAction",
    "createPlaceLocationAction",
    "updatePlaceLocationAction",
    "deletePlaceLocationAction",
];

/// `pickLocationSearch`: the same 1-50 cap as the public API.
const MAX_PER_PAGE: i64 = 50;

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The form coercions: a rank from the select is text, an emptied category
/// box is `""` (stored as null).
fn form_input(value: &Value) -> Input {
    let mut body = value.clone();
    if let Some(rank) = body["rank"].as_str().map(str::trim) {
        body["rank"] = rank.parse::<f64>().map_or(json!(rank), |n| json!(n));
    }
    if body["categoryId"]
        .as_str()
        .is_some_and(|c| c.trim().is_empty())
    {
        body["categoryId"] = Value::Null;
    }
    Input::from_body(&[], body.to_string().as_bytes())
}

/// `throwCategoryFormError`: a duplicate name is a field error on `name`.
fn category_fail(error: BusinessError) -> OrpcError {
    if error.code() != Some("placeCategoryDuplicated") {
        return action_fail(error);
    }
    let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    out.data = Some(json!({
        "validationErrors": { "name": { "_errors": ["Category name already exists"] } }
    }));
    out
}

fn bound_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    match name {
        "listPlaceCategories" => {
            let rows = category::list(db, ws).await.map_err(action_fail)?;
            Ok(json!(
                rows.iter().map(category_list_json).collect::<Vec<_>>()
            ))
        }
        "listPlaceLocations" => {
            let search = In(arg(args, 0));
            let list = location::list(
                db,
                ListPlaceLocations {
                    workspace_id: ws,
                    name: search.opt_str("name")?.filter(|n| !n.trim().is_empty()),
                    page: Some(search.opt_positive("page")?.unwrap_or(1)),
                    per_page: Some(
                        search
                            .opt_positive("perPage")?
                            .unwrap_or(10)
                            .min(MAX_PER_PAGE),
                    ),
                },
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({
                "data": list.data.iter().map(location_json).collect::<Vec<_>>(),
                "pageCount": list.page_count,
            }))
        }
        "createPlaceCategoryAction" | "updatePlaceCategoryAction" => {
            let create = name == "createPlaceCategoryAction";
            let mut input = form_input(arg(args, if create { 1 } else { 2 }));
            let category_name = input.string("name", NAME);
            let rank = input.opt_number("rank", RANK).map(|r| r as i64);
            input.finish().map_err(super::schema_errors)?;
            let category_name = category_name.unwrap_or_default();
            let row = if create {
                category::create(
                    db,
                    CreatePlaceCategory {
                        workspace_id: ws,
                        name: category_name,
                        rank,
                    },
                )
                .await
            } else {
                category::update(
                    db,
                    UpdatePlaceCategory {
                        workspace_id: ws,
                        category_id: bound_id(args, 1)?,
                        name: Some(category_name),
                        rank,
                    },
                )
                .await
            }
            .map_err(category_fail)?;
            Ok(category_json(&row))
        }
        "deletePlaceCategoryAction" => {
            let out = category::delete(db, ws, bound_id(args, 1)?)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "uncategorizedLocationCount": out.uncategorized_location_count }))
        }
        "createPlaceLocationAction" | "updatePlaceLocationAction" => {
            let create = name == "createPlaceLocationAction";
            let mut input = form_input(arg(args, if create { 1 } else { 2 }));
            let location_name = input.string("name", NAME);
            let latitude = input.number("latitude", LATITUDE);
            let longitude = input.number("longitude", LONGITUDE);
            let category_id = input.nullish_id("categoryId");
            input.finish().map_err(super::schema_errors)?;
            let item = if create {
                location::create(
                    db,
                    CreatePlaceLocation {
                        workspace_id: ws,
                        name: location_name.unwrap_or_default(),
                        latitude: latitude.unwrap_or_default(),
                        longitude: longitude.unwrap_or_default(),
                        category_id: category_id.flatten(),
                    },
                )
                .await
            } else {
                location::update(
                    db,
                    UpdatePlaceLocation {
                        workspace_id: ws,
                        location_id: bound_id(args, 1)?,
                        name: location_name,
                        latitude,
                        longitude,
                        category_id: Some(category_id.flatten()),
                    },
                )
                .await
            }
            .map_err(action_fail)?;
            Ok(location_json(&item))
        }
        "deletePlaceLocationAction" => {
            location::delete(db, ws, bound_id(args, 1)?)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "ok": true }))
        }
        _ => Err(super::invalid()),
    }
}
