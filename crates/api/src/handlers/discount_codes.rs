//! `features/discount-codes/api/public.ts`: list / get / generateCode /
//! create / update / setActive / delete / validate.

use chatbotx_business::contact::service::resolve_id_by_identifier;
use chatbotx_business::discount_code::{self, is_code};
use chatbotx_business::order::{self, PlaceItem};
use serde_json::{Map, Value, json};

use super::broadcasts::{coerce_date, parse_contact_filter};
use super::input::{Input, Num, Seg, Str, child, key};
use super::{ApiState, Call, ErrorMap, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "discountCodes.list",
    "discountCodes.create",
    "discountCodes.get",
    "discountCodes.update",
    "discountCodes.delete",
    "discountCodes.generateCode",
    "discountCodes.setActive",
    "discountCodes.validate",
];

/// `possibleErrorsOnCreatingDiscountCode`.
const CREATING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("discountCodeDuplicated", 409),
];

/// `possibleErrorsOnMutatingDiscountCode`.
const MUTATING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("discountCodeDuplicated", 409),
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

/// `possibleErrorsOnFindingResource` + `possibleIdempotencyErrors`.
const VALIDATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

const TYPES: &[&str] = &["percentage", "fixedAmount", "freeShipping"];
const APPLIES_TO: &[&str] = &["entireOrder", "category", "products"];
const MINIMUM_TYPES: &[&str] = &["none", "amount", "quantity"];
const ELIGIBILITIES: &[&str] = &["everyone", "specificCustomers"];

const NON_NEGATIVE: Num = Num {
    int: false,
    min: Some(0.0),
    max: None,
};

/// `discountCodeWriteRequest` as the service input (`toDiscountCodeInput`),
/// or with `partial` only the fields sent (`toDiscountCodePatch`).
fn parse_write(input: &mut Input, partial: bool) -> Map<String, Value> {
    let mut out = Map::new();
    let sent = |input: &Input, name: &str| !partial || input.has(name);
    if sent(input, "code") {
        let spec = Str {
            trim: true,
            pattern: Some(is_code),
            ..Str::default()
        };
        if let Some(code) = input.string("code", spec) {
            out.insert("code".into(), json!(code));
        }
    }
    for (name, default) in [
        ("autoApply", false),
        ("oncePerCustomer", false),
        ("isActive", true),
    ] {
        if sent(input, name) {
            let value = input.opt_bool(name);
            if input.has(name) || !partial {
                out.insert(name.into(), json!(value.unwrap_or(default)));
            }
        }
    }
    for (name, options, default) in [
        ("discountType", TYPES, None),
        ("appliesTo", APPLIES_TO, Some("entireOrder")),
        ("minimumType", MINIMUM_TYPES, Some("none")),
        ("eligibility", ELIGIBILITIES, Some("everyone")),
    ] {
        if sent(input, name) {
            let value = input.get(name).cloned();
            let required = default.is_none();
            let parsed = input.enum_at(&key(name), value.as_ref(), options, required);
            if let Some(v) = parsed.or_else(|| default.map(str::to_string)) {
                out.insert(name.into(), json!(v));
            }
        }
    }
    for name in ["discountValue", "minimumValue"] {
        if sent(input, name) {
            let value = input.opt_number(name, NON_NEGATIVE).unwrap_or(0.0);
            out.insert(name.into(), json!(value));
        }
    }
    if sent(input, "categoryId") {
        out.insert(
            "categoryId".into(),
            json!(match input.get("categoryId").cloned() {
                None | Some(Value::Null) => None,
                Some(value) => input.digits_id_at(&key("categoryId"), Some(&value), true),
            }),
        );
    }
    if sent(input, "productIds") {
        let mut ids = Vec::new();
        match input.get("productIds").cloned() {
            None => {}
            Some(Value::Array(list)) => {
                if list.len() > 500 {
                    input.issue(
                        &key("productIds"),
                        "too_big",
                        "Too big: expected array to have <=500 items",
                    );
                }
                for (index, item) in list.iter().enumerate() {
                    let path = vec![Seg::Key("productIds".into()), Seg::Index(index)];
                    if let Some(id) = input.digits_id_at(&path, Some(item), true) {
                        ids.push(id);
                    }
                }
            }
            Some(_) => input.issue(
                &key("productIds"),
                "invalid_type",
                "Invalid input: expected array",
            ),
        }
        out.insert("productIds".into(), json!(ids));
    }
    if sent(input, "usageLimit") {
        let limit = match input.get("usageLimit").cloned() {
            None | Some(Value::Null) => None,
            Some(v) => input.number_at(
                &key("usageLimit"),
                Some(&v),
                Num {
                    int: true,
                    min: Some(1.0),
                    max: None,
                },
                true,
            ),
        };
        out.insert("usageLimit".into(), json!(limit.map(|n| n as i64)));
    }
    if sent(input, "customerFilter") {
        let present = !matches!(input.get("customerFilter"), None | Some(Value::Null));
        let filter = if present {
            parse_contact_filter(input, "customerFilter", true)
        } else {
            None
        };
        out.insert("customerFilter".into(), filter.unwrap_or(Value::Null));
    }
    for name in ["startsAt", "endsAt"] {
        match input.get(name).cloned() {
            // `startsAt` defaults to now in the service.
            None => {
                if name == "endsAt" && !partial {
                    out.insert(name.into(), Value::Null);
                }
            }
            Some(Value::Null) if name == "endsAt" => {
                out.insert(name.into(), Value::Null);
            }
            Some(value) => match coerce_date(&value) {
                Some(ms) => {
                    out.insert(name.into(), json!(ms));
                }
                None => input.issue(
                    &key(name),
                    "invalid_type",
                    "Invalid input: expected date, received Date",
                ),
            },
        }
    }
    out
}

/// `validateDiscountCodePublicRequest`.
fn parse_validate(input: &mut Input) -> (String, Vec<PlaceItem>, Option<String>, Option<String>) {
    let code = input
        .string("code", Str::trimmed(1, 64))
        .unwrap_or_default();
    let mut items = Vec::new();
    match input.get("items").cloned() {
        Some(Value::Array(list)) => {
            if list.is_empty() {
                input.issue(
                    &key("items"),
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
            } else if list.len() > 100 {
                input.issue(
                    &key("items"),
                    "too_big",
                    "Too big: expected array to have <=100 items",
                );
            }
            for (index, item) in list.iter().enumerate() {
                let path = vec![Seg::Key("items".into()), Seg::Index(index)];
                let Value::Object(obj) = item else {
                    input.issue(&path, "invalid_type", "Invalid input: expected object");
                    continue;
                };
                let product_id =
                    input.digits_id_at(&child(&path, "productId"), obj.get("productId"), true);
                let variant_id = match obj.get("variantId") {
                    None | Some(Value::Null) => None,
                    Some(v) => input.digits_id_at(&child(&path, "variantId"), Some(v), true),
                };
                let quantity = input.number_at(
                    &child(&path, "quantity"),
                    obj.get("quantity"),
                    Num::int(1.0, 10_000.0),
                    true,
                );
                if let (Some(product_id), Some(quantity)) = (product_id, quantity) {
                    items.push(PlaceItem {
                        product_id,
                        variant_id,
                        quantity: quantity as i64,
                    });
                }
            }
        }
        _ => input.issue(
            &key("items"),
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    let contact = input.opt_string(
        "contact",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let shipping_type = input.get("shippingType").cloned().and_then(|v| {
        input.enum_at(
            &key("shippingType"),
            Some(&v),
            &["delivery", "pickup"],
            true,
        )
    });
    (code, items, contact, shipping_type)
}

fn code_id(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let id = input.digits_id("id");
    input.finish()?;
    Ok(id.unwrap_or_default())
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let one = |row: &chatbotx_db::repo::models::DiscountCode| {
        json(200, discount_code::resource_json(row))
    };
    match route_id {
        "discountCodes.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let code = input.opt_string("code", Str::trimmed(0, 64));
            let page = input.opt_coerce_int("page", Num::int(1.0, f64::MAX));
            let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 100.0));
            input.finish()?;
            let (rows, page_count) = discount_code::list(&state.db, ws, code, page, per_page)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = rows.iter().map(discount_code::resource_json).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "discountCodes.get" => {
            let id = code_id(&call)?;
            let row = discount_code::get(&state.db, ws, id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(one(&row))
        }
        "discountCodes.generateCode" => {
            let code = discount_code::generate_code(&state.db, ws)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, json!({ "code": code })))
        }
        "discountCodes.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_write(&mut input, false);
            input.finish()?;
            let row = discount_code::create(&state.db, ws, data)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            Ok(one(&row))
        }
        "discountCodes.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let data = parse_write(&mut input, true);
            input.finish()?;
            let row = discount_code::update(&state.db, ws, id.unwrap_or_default(), data)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(one(&row))
        }
        "discountCodes.setActive" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let is_active = input.bool("isActive");
            input.finish()?;
            let row = discount_code::set_active(
                &state.db,
                ws,
                id.unwrap_or_default(),
                is_active.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(one(&row))
        }
        "discountCodes.delete" => {
            let id = code_id(&call)?;
            let row = discount_code::delete(&state.db, ws, id)
                .await
                .map_err(|e| business_error(e, DELETING))?;
            Ok(one(&row))
        }
        "discountCodes.validate" => {
            let mut input = Input::from_body(call.params, call.body);
            let (code, items, contact, shipping_type) = parse_validate(&mut input);
            input.finish()?;
            let contact_id = match contact {
                Some(identifier) => Some(
                    resolve_id_by_identifier(&state.db, ws, &identifier)
                        .await
                        .map_err(|e| business_error(e, VALIDATING))?,
                ),
                None => None,
            };
            let quote = order::quote(
                &state.db,
                ws,
                items,
                shipping_type,
                contact_id,
                Some(code),
                false,
            )
            .await
            .map_err(|e| business_error(e, VALIDATING))?;
            Ok(json(200, validate_json(&quote)))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `validateDiscountCodePublicResponse` of an order quote.
pub(crate) fn validate_json(quote: &order::Quote) -> Value {
    let discount = quote.draft.discount.as_ref();
    let error = quote
        .discount_error
        .as_ref()
        .map(|(code, data)| match data {
            Some(data) => json!({ "code": code, "data": data }),
            None => json!({ "code": code }),
        });
    json!({
        "valid": quote.discount_error.is_none() && discount.is_some(),
        "amount": discount.map_or(0.0, |d| d.amount),
        "currency": quote.draft.currency,
        "error": error,
        "total": quote.draft.totals.total,
    })
}
