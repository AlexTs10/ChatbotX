//! `features/product-categories/api/public.ts`: `productCategories.*`.

use chatbotx_business::product_category::{
    self as service, CategoryWithCount, CreateCategory, ListFilter, UpdateCategory,
};
use chatbotx_db::repo::models::ProductCategory;
use serde_json::{Value, json};

use super::input::{Input, Num, Str};
use super::{ApiState, Call, ErrorMap, Reply, business_error, id, json, no_content, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "productCategories.list",
    "productCategories.create",
    "productCategories.setVisible",
    "productCategories.update",
    "productCategories.delete",
];

/// `externalStoreManagedErrors`.
const MANAGED: (&str, u16) = ("externalStoreManaged", 409);
const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    MANAGED,
];
const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    MANAGED,
];

/// `publicProductCategoryWriteResource`.
fn write_json(row: &ProductCategory) -> Value {
    json!({
        "id": id(row.id),
        "parentId": opt_id(row.parent_id),
        "name": row.name,
        "imageUrl": row.image_url,
        "isVisible": row.is_visible,
        "rank": row.rank,
    })
}

/// `publicProductCategoryResource`.
fn list_json(row: &CategoryWithCount) -> Value {
    json!({
        "id": id(row.id),
        "parentId": opt_id(row.parent_id),
        "name": row.name,
        "imageUrl": row.image_url,
        "isVisible": row.is_visible,
        "rank": row.rank,
        "productCount": row.product_count,
    })
}

/// The fields of `createProductCategoryPublicRequest`.
struct Form {
    name: Option<String>,
    parent_id: Option<Option<i64>>,
    rank: Option<i64>,
    image_url: Option<Option<String>>,
    is_visible: Option<bool>,
}

fn form(input: &mut Input) -> Form {
    Form {
        name: input.string("name", Str::trimmed(1, 255)),
        parent_id: input.nullish_id("parentId"),
        rank: input
            .opt_number(
                "rank",
                Num {
                    int: true,
                    ..Num::default()
                },
            )
            .map(|n| n as i64),
        image_url: input.nullish_url("imageUrl", 2048),
        is_visible: input.opt_bool("isVisible"),
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "productCategories.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let parent_id = if input.has("parentId") {
                input.id("parentId")
            } else {
                None
            };
            let name = input.opt_string("name", Str::trimmed(0, 255));
            input.finish()?;
            let rows = service::list(db, ws, ListFilter { parent_id, name })
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            Ok(json(
                200,
                json!({ "data": rows.iter().map(list_json).collect::<Vec<_>>() }),
            ))
        }
        "productCategories.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let f = form(&mut input);
            input.finish()?;
            service::create(
                db,
                CreateCategory {
                    workspace_id: ws,
                    name: f.name.unwrap_or_default(),
                    rank: f.rank,
                    parent_id: f.parent_id.flatten(),
                    image_url: f.image_url.flatten(),
                    is_visible: f.is_visible,
                },
            )
            .await
            .map(|row| json(200, write_json(&row)))
            .map_err(|e| business_error(e, CREATING))
        }
        "productCategories.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let category_id = input.id("id");
            let f = form(&mut input);
            input.finish()?;
            service::update(
                db,
                UpdateCategory {
                    workspace_id: ws,
                    category_id: category_id.unwrap_or_default(),
                    name: f.name,
                    parent_id: f.parent_id,
                    image_url: f.image_url,
                    is_visible: f.is_visible,
                    rank: f.rank,
                },
            )
            .await
            .map(|row| json(200, write_json(&row)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "productCategories.setVisible" => {
            let mut input = Input::from_body(call.params, call.body);
            let category_id = input.id("id");
            let is_visible = input.bool("isVisible");
            input.finish()?;
            service::update(
                db,
                UpdateCategory {
                    workspace_id: ws,
                    category_id: category_id.unwrap_or_default(),
                    is_visible,
                    ..UpdateCategory::default()
                },
            )
            .await
            .map(|row| json(200, write_json(&row)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "productCategories.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let category_id = input.id("id");
            input.finish()?;
            service::delete(db, ws, category_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
