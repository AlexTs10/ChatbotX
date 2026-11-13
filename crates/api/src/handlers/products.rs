//! `features/products/api/public.ts`: the catalog routes of `products.*`
//! (the Meta catalog and import routes are not ported yet).

use chatbotx_business::product::{
    self as service, AddonInput, FullData, ImageInput, ProductData, ProductListInput, VariantInput,
    VariantOptionInput,
};
use chatbotx_business::product_message::service::{ListSendable, list_sendable};
use chatbotx_db::repo::models::Product;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::{
    ApiState, Call, ErrorMap, Reply, business_error, date, id, json, no_content, number, opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "products.list",
    "products.listSendable",
    "products.get",
    "products.create",
    "products.update",
    "products.delete",
    "products.setActive",
    "products.deleteMany",
];

const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("externalStoreManaged", 409),
];
const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("externalStoreManaged", 409),
];
const PUBLIC_LIST_MAX_PER_PAGE: f64 = 50.0;
/// `DEFAULT_PRODUCT_CURRENCY`.
const DEFAULT_CURRENCY: &str = "USD";
/// `channelTypes`.
const CHANNELS: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];

/// `publicProductResource` (+ `withLegacyIsActive`); `vendor` is the vendor's name.
fn product_json(row: &Product, vendor: Option<&String>) -> Map<String, Value> {
    let images: Vec<Value> = row
        .images
        .0
        .as_array()
        .map(|list| {
            list.iter()
                .map(|i| json!({ "url": i.get("url").cloned().unwrap_or(json!("")), "type": i.get("type").cloned().unwrap_or(json!("file")) }))
                .collect()
        })
        .unwrap_or_default();
    let Value::Object(out) = json!({
        "id": id(row.id),
        "workspaceId": id(row.workspace_id),
        "name": row.name,
        "shortDescription": row.short_description,
        "longDescription": row.long_description,
        "price": number(row.price),
        "taxes": number(row.taxes),
        "discount": number(row.discount),
        "currency": row.currency,
        "productUrl": row.product_url,
        "sku": row.sku,
        "inventoryPolicy": row.inventory_policy,
        "inventoryQuantity": row.inventory_quantity,
        "allowOutOfStockPurchase": row.allow_out_of_stock_purchase,
        "images": images,
        "tags": row.tags.0,
        "vendor": vendor,
        "vendorId": opt_id(row.vendor_id),
        "rank": row.rank,
        "categoryId": opt_id(row.category_id),
        "subcategoryId": opt_id(row.subcategory_id),
        "status": row.status,
        "isActive": row.status == "active",
        "isSearchable": row.is_searchable,
        "allowSpecialRequest": row.allow_special_request,
        "isAddonOnly": row.is_addon_only,
        "source": row.source,
        "externalId": row.external_id,
        "externalHandle": row.external_handle,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    }) else {
        unreachable!("json! object")
    };
    out
}

/// `publicProductDetailResource`.
fn detail_json(detail: &service::ProductDetail) -> Value {
    let mut out = product_json(&detail.product, detail.vendor.as_ref());
    out.insert(
        "variantOptions".into(),
        detail
            .variant_options
            .iter()
            .map(|o| json!({ "name": o.name, "values": o.values.0, "position": o.position }))
            .collect(),
    );
    out.insert(
        "variants".into(),
        detail
            .variants
            .iter()
            .map(|v| {
                json!({
                    "combination": v.combination.0,
                    "price": number(v.price),
                    "isEnabled": v.is_enabled,
                    "sku": v.sku,
                    "inventoryQuantity": v.inventory_quantity,
                    "externalId": v.external_id,
                })
            })
            .collect(),
    );
    out.insert(
        "addons".into(),
        detail
            .addons
            .iter()
            .map(|a| json!({ "name": a.name, "maxSelections": a.max_selections, "addonProductIds": a.addon_product_ids.0 }))
            .collect(),
    );
    Value::Object(out)
}

/// `z.string().regex(/^\d+$/).nullish()` (a value past i64 is an unknown id).
fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.nullish_string(name, Str::default())? {
        None => Some(None),
        Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Some(Some(text.parse().unwrap_or(-1)))
        }
        Some(_) => {
            input.issue(
                &key(name),
                "invalid_format",
                "Invalid string: must match pattern /^\\d+$/",
            );
            None
        }
    }
}

/// A `.nullish().default("")` string.
fn text_default(input: &mut Input, name: &str, spec: Str) -> Option<Option<String>> {
    Some(
        input
            .nullish_string(name, spec)
            .unwrap_or(Some(String::new())),
    )
}

fn coerce(input: &mut Input, path: &[Seg], value: Option<&Value>, spec: Num, default: f64) -> f64 {
    if value.is_none() {
        return default;
    }
    input
        .coerce_number_at(path, value, spec, true)
        .unwrap_or(default)
}

fn strings_at(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(list) = value.as_array() else {
        input.issue(path, "invalid_type", "Invalid input: expected array");
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, item) in list.iter().enumerate() {
        let mut at: Path = path.to_vec();
        at.push(Seg::Index(index));
        if let Some(text) = input.string_at(&at, Some(item), Str::default(), true) {
            out.push(text);
        }
    }
    out
}

/// The objects of an optional array key (`.default([])`).
fn objects(input: &mut Input, name: &str) -> Vec<(Path, Map<String, Value>)> {
    let Some(value) = input.get(name).cloned() else {
        return Vec::new();
    };
    let Some(list) = value.as_array() else {
        input.issue(&key(name), "invalid_type", "Invalid input: expected array");
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, item) in list.iter().enumerate() {
        let mut at = key(name);
        at.push(Seg::Index(index));
        match item.as_object() {
            Some(obj) => out.push((at, obj.clone())),
            None => input.issue(&at, "invalid_type", "Invalid input: expected object"),
        }
    }
    out
}

/// `productFormRequest` with its defaults (`required_name`: the PUT form).
fn form(input: &mut Input, required_name: bool) -> FullData {
    // `.default("")`: an omitted name on create is "" (zod 4 skips the checks).
    let name = if required_name || input.has("name") {
        input.string("name", Str::trimmed(1, 255))
    } else {
        Some(String::new())
    };
    let get = |input: &Input, name: &str| input.get(name).cloned();
    let price = get(input, "price");
    let price = coerce(
        input,
        &key("price"),
        price.as_ref(),
        Num {
            min: Some(0.0),
            ..Num::default()
        },
        0.0,
    );
    let taxes = get(input, "taxes");
    let taxes = coerce(
        input,
        &key("taxes"),
        taxes.as_ref(),
        Num::range(0.0, 100.0),
        0.0,
    );
    let discount = get(input, "discount");
    let discount = coerce(
        input,
        &key("discount"),
        discount.as_ref(),
        Num::range(0.0, 100.0),
        0.0,
    );
    let currency = if input.has("currency") {
        input.string("currency", Str::trimmed(3, 3))
    } else {
        Some(DEFAULT_CURRENCY.to_string())
    };
    let product_url = match input.nullish_string("productUrl", Str::default()) {
        None => Some(String::new()),
        Some(None) => None,
        Some(Some(url)) => {
            let ok = url.is_empty()
                || url::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
            if !ok {
                input.issue(&key("productUrl"), "invalid_format", "Invalid URL");
            }
            Some(url)
        }
    };
    let policy = get(input, "inventoryPolicy");
    let inventory_policy = match policy {
        None => Some("dont_track".to_string()),
        Some(value) => input.enum_at(
            &key("inventoryPolicy"),
            Some(&value),
            &["dont_track", "track"],
            true,
        ),
    };
    let quantity = get(input, "inventoryQuantity");
    let inventory_quantity = coerce(
        input,
        &key("inventoryQuantity"),
        quantity.as_ref(),
        Num {
            int: true,
            min: Some(0.0),
            max: None,
        },
        0.0,
    );
    let rank_value = get(input, "rank");
    let rank = coerce(
        input,
        &key("rank"),
        rank_value.as_ref(),
        Num {
            int: true,
            ..Num::default()
        },
        10.0,
    );
    let mut images = Vec::new();
    for (at, obj) in objects(input, "images") {
        let mode = match obj.get("mode") {
            None => Some("file".to_string()),
            value => input.enum_at(&child(&at, "mode"), value, &["link", "file"], true),
        };
        let url = match obj.get("url") {
            None => Some(String::new()),
            value => input.string_at(&child(&at, "url"), value, Str::default(), true),
        };
        if let (Some(mode), Some(url)) = (mode, url) {
            images.push(ImageInput { mode, url });
        }
    }
    let mut variant_options = Vec::new();
    for (at, obj) in objects(input, "variantOptions") {
        let name = input.string_at(&child(&at, "name"), obj.get("name"), Str::default(), true);
        let values_at = child(&at, "values");
        if obj.get("values").is_none() {
            input.issue(
                &values_at,
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
        }
        let values = strings_at(input, &values_at, obj.get("values"));
        let position = coerce(
            input,
            &child(&at, "position"),
            obj.get("position"),
            Num::default(),
            0.0,
        );
        if let Some(name) = name {
            variant_options.push(VariantOptionInput {
                name,
                values,
                position,
            });
        }
    }
    let mut variants = Vec::new();
    for (at, obj) in objects(input, "variants") {
        let combination_at = child(&at, "combination");
        let combination = match obj.get("combination") {
            Some(Value::Object(map)) if map.values().all(Value::is_string) => Some(map.clone()),
            _ => {
                input.issue(
                    &combination_at,
                    "invalid_type",
                    "Invalid input: expected record",
                );
                None
            }
        };
        let price = coerce(
            input,
            &child(&at, "price"),
            obj.get("price"),
            Num {
                min: Some(0.0),
                ..Num::default()
            },
            0.0,
        );
        let is_enabled = match obj.get("isEnabled") {
            None => Some(true),
            value => input.bool_at(&child(&at, "isEnabled"), value, true),
        };
        if let (Some(combination), Some(is_enabled)) = (combination, is_enabled) {
            variants.push(VariantInput {
                combination,
                price,
                is_enabled,
            });
        }
    }
    let mut addons = Vec::new();
    for (at, obj) in objects(input, "addons") {
        let name = match obj.get("name") {
            None => Some(String::new()),
            value => input.string_at(&child(&at, "name"), value, Str::default(), true),
        };
        let max_selections = coerce(
            input,
            &child(&at, "maxSelections"),
            obj.get("maxSelections"),
            Num {
                int: true,
                min: Some(1.0),
                max: None,
            },
            1.0,
        );
        let addon_product_ids = strings_at(
            input,
            &child(&at, "addonProductIds"),
            obj.get("addonProductIds"),
        );
        if let Some(name) = name {
            addons.push(AddonInput {
                name,
                max_selections: max_selections as i64,
                addon_product_ids,
            });
        }
    }
    let tags = get(input, "tags");
    let tags = strings_at(input, &key("tags"), tags.as_ref());
    let flag = |input: &mut Input, name: &str, default: bool| {
        Some(input.opt_bool(name).unwrap_or(default))
    };
    let product = ProductData {
        name,
        short_description: text_default(input, "shortDescription", Str::default()),
        long_description: text_default(input, "longDescription", Str::max(840)),
        price: Some(price),
        taxes: Some(taxes),
        discount: Some(discount),
        currency,
        product_url: Some(product_url),
        sku: text_default(input, "sku", Str::default()),
        inventory_policy,
        inventory_quantity: Some(inventory_quantity as i64),
        allow_out_of_stock_purchase: flag(input, "allowOutOfStockPurchase", false),
        images: Some(images),
        tags: Some(tags),
        vendor_id: nullish_digits(input, "vendorId"),
        vendor: input.nullish_string("vendor", Str::default()),
        rank: Some(rank as i64),
        category_id: nullish_digits(input, "categoryId"),
        subcategory_id: nullish_digits(input, "subcategoryId"),
        status: None,
        is_searchable: flag(input, "isSearchable", true),
        allow_special_request: flag(input, "allowSpecialRequest", false),
        is_addon_only: flag(input, "isAddonOnly", false),
    };
    FullData {
        product,
        variant_options,
        variants,
        addons,
    }
}

/// `withPublicPaging`: `page` (default 1) and `perPage` (1–50, default 50).
fn paging(input: &mut Input) -> (i64, i64) {
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, PUBLIC_LIST_MAX_PER_PAGE));
    (page.unwrap_or(1), per_page.unwrap_or(50))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "products.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let name = input.nullish_string("name", Str::default()).flatten();
            let category_id = nullish_digits(&mut input, "categoryId").flatten();
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let page = service::list(
                db,
                ProductListInput {
                    workspace_id: ws,
                    name,
                    category_id,
                    page: Some(page),
                    per_page: Some(per_page),
                    ..ProductListInput::default()
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| Value::Object(product_json(&row.product, row.vendor.as_ref())))
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "products.listSendable" => {
            let mut input = Input::from_query(call.params, call.query);
            let channel_value = input.get("channel").cloned();
            let channel = input.enum_at(&key("channel"), channel_value.as_ref(), CHANNELS, true);
            let keyword = input.opt_string("keyword", Str::trimmed(1, 255));
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let result = list_sendable(
                db,
                ListSendable {
                    workspace_id: ws,
                    channel: channel.unwrap_or_default(),
                    keyword,
                    page: Some(page),
                    per_page: Some(per_page),
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let mut out = json!({ "ready": result.ready, "data": result.data, "pageCount": result.page_count });
            if let Some(reason) = result.reason {
                out["reason"] = json!(reason);
            }
            Ok(json(200, out))
        }
        "products.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let product_id = input.id("id");
            input.finish()?;
            service::find_by_id(db, ws, product_id.unwrap_or_default())
                .await
                .map(|detail| json(200, detail_json(&detail)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "products.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = form(&mut input, false);
            input.finish()?;
            let product = service::create_full(db, ws, data)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            service::find_by_id(db, ws, product.id)
                .await
                .map(|detail| {
                    json(
                        200,
                        Value::Object(product_json(&detail.product, detail.vendor.as_ref())),
                    )
                })
                .map_err(|e| business_error(e, CREATING))
        }
        "products.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let product_id = input.id("id");
            let data = form(&mut input, true);
            input.finish()?;
            service::update_full(db, ws, product_id.unwrap_or_default(), data)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "products.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let product_id = input.id("id").unwrap_or_default();
            input.finish()?;
            service::find_by_id(db, ws, product_id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            service::delete(db, ws, vec![product_id])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "products.setActive" => {
            let mut input = Input::from_body(call.params, call.body);
            let product_id = input.id("id").unwrap_or_default();
            let is_active = input.bool("isActive");
            input.finish()?;
            service::find_by_id(db, ws, product_id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let status = if is_active == Some(true) {
                "active"
            } else {
                "draft"
            };
            service::update(
                db,
                ws,
                product_id,
                ProductData {
                    status: Some(status.to_string()),
                    ..ProductData::default()
                },
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, MUTATING))
        }
        "products.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids_value = input.get("ids").cloned();
            let mut ids = Vec::new();
            match ids_value.as_ref().and_then(Value::as_array) {
                Some(list) => {
                    for (index, item) in list.iter().enumerate() {
                        let mut at = key("ids");
                        at.push(Seg::Index(index));
                        if let Some(id) = input.id_at(&at, Some(item), true) {
                            ids.push(id);
                        }
                    }
                }
                None => input.issue(&key("ids"), "invalid_type", "Invalid input: expected array"),
            }
            input.finish()?;
            service::delete(db, ws, ids)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
