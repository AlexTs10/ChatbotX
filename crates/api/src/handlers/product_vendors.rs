//! `features/product-vendors/api/public.ts`: `productVendors.*`.

use chatbotx_business::vendor::{self as service, ListVendors, VendorData, money};
use chatbotx_db::repo::models::Vendor;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Str, is_bigint_id};
use super::{
    ApiState, Call, ErrorMap, Reply, business_error, id, json, no_content, number, opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "productVendors.list",
    "productVendors.create",
    "productVendors.update",
    "productVendors.delete",
];

/// `possibleErrorsOnCreatingProductVendor`.
const CREATING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("productVendorDuplicated", 400),
    ("productVendorNameRequired", 400),
];
/// `possibleErrorsOnMutatingProductVendor`.
const MUTATING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("productVendorDuplicated", 400),
    ("productVendorNameRequired", 400),
];
/// `possibleErrorsOnDeletingResource` + `externalStoreManagedErrors`.
const DELETING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("externalStoreManaged", 409),
];

/// `publicProductVendorWriteResource`.
fn vendor_json(row: &Vendor) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": id(row.id),
        "name": row.name,
        "shortDescription": row.short_description,
        "phone": row.phone,
        "storeAddress": row.store_address,
        "imageUrl": row.image_url,
        "isActive": row.is_active,
        "discountPercent": number(money(row.discount_percent)),
        "minimumOrderValue": number(money(row.minimum_order_value)),
        "openHoursId": opt_id(row.open_hours_id),
        "categoryId": opt_id(row.category_id),
        "subcategoryId": opt_id(row.subcategory_id),
        "rank": row.rank,
    }) else {
        unreachable!("json! object")
    };
    out
}

/// `idSchema`: `z.string().regex(/^\d+$/).nullish()`.
fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    let out = input.nullish_string(name, Str::default())?;
    match out {
        None => Some(None),
        Some(text) if text.bytes().all(|b| b.is_ascii_digit()) && !text.is_empty() => {
            // A value past i64 is an unknown id.
            Some(Some(if is_bigint_id(&text) {
                text.parse().unwrap_or(-1)
            } else {
                -1
            }))
        }
        Some(_) => {
            input.issue(
                &super::input::key(name),
                "invalid_format",
                "Invalid string: must match pattern /^\\d+$/",
            );
            None
        }
    }
}

/// `productVendorFormRequest` (`required_name`: create vs. the partial update).
fn form(input: &mut Input, required_name: bool) -> VendorData {
    let name_spec = Str::trimmed(1, 255);
    VendorData {
        name: if required_name {
            input.string("name", name_spec)
        } else {
            input.opt_string("name", name_spec)
        },
        short_description: input.nullish_string("shortDescription", Str::trimmed(0, 500)),
        phone: input.nullish_string("phone", Str::trimmed(0, 64)),
        store_address: input.nullish_string("storeAddress", Str::trimmed(0, 1000)),
        image_url: input.nullish_url("imageUrl", 2048),
        is_active: input.opt_bool("isActive"),
        discount_percent: input.opt_number("discountPercent", Num::range(0.0, 100.0)),
        minimum_order_value: input.opt_number(
            "minimumOrderValue",
            Num {
                min: Some(0.0),
                ..Num::default()
            },
        ),
        open_hours_id: nullish_digits(input, "openHoursId"),
        category_id: nullish_digits(input, "categoryId"),
        subcategory_id: nullish_digits(input, "subcategoryId"),
        rank: input
            .opt_number(
                "rank",
                Num {
                    int: true,
                    ..Num::default()
                },
            )
            .map(|n| n as i64),
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "productVendors.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let name = input.opt_string("name", Str::trimmed(0, 255));
            let page = input.opt_coerce_int(
                "page",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 100.0));
            input.finish()?;
            let page = service::list(
                db,
                ListVendors {
                    workspace_id: ws,
                    name,
                    page,
                    per_page,
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    let mut out = vendor_json(&row.vendor);
                    out.insert("productCount".into(), json!(row.product_count));
                    Value::Object(out)
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "productVendors.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = form(&mut input, true);
            input.finish()?;
            service::create(db, ws, data)
                .await
                .map(|row| json(200, Value::Object(vendor_json(&row))))
                .map_err(|e| business_error(e, CREATING))
        }
        "productVendors.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let vendor_id = input.id("id");
            let data = form(&mut input, false);
            input.finish()?;
            service::update(db, ws, vendor_id.unwrap_or_default(), data)
                .await
                .map(|row| json(200, Value::Object(vendor_json(&row))))
                .map_err(|e| business_error(e, MUTATING))
        }
        "productVendors.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let vendor_id = input.id("id");
            input.finish()?;
            service::delete(db, ws, vendor_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, DELETING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
