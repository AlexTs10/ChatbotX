//! `features/orders/api/public.ts`: list / export / get / create / delete,
//! the status moves and the payment link.

use chatbotx_business::contact::service::resolve_id_by_identifier;
use chatbotx_business::order::{self, Actor, ContactRef, Deps, PlaceInput, PlaceItem};
use chatbotx_business::order_payment;
use chatbotx_db::repo::order::OrderListInput;
use serde_json::{Map, Value, json};

use super::broadcasts::coerce_date;
use super::contacts::{bracket_query, is_email};
use super::input::{Input, Num, Seg, Str, child, key};
use super::{ApiState, Call, ErrorMap, Reply, business_error, date, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "orders.list",
    "orders.create",
    "orders.export",
    "orders.get",
    "orders.delete",
    "orders.getPaymentLink",
    "orders.accept",
    "orders.ship",
    "orders.conclude",
    "orders.cancel",
];

const API_TOKEN_ACTOR: Actor = Actor {
    kind: "apiToken",
    user_id: None,
};

const PAYMENT_PROVIDER_ERRORS: [(&str, u16); 5] = [
    ("paymentProviderNotConnected", 400),
    ("paymentProviderCredentialsRejected", 400),
    ("paymentProviderRejected", 400),
    ("paymentProviderUnavailable", 503),
    ("paymentAmountUnsupported", 400),
];

/// `possibleErrorsOnPlacingOrder`.
const PLACING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("discountCodeNotFound", 404),
    ("discountCodeInactive", 400),
    ("discountCodeNotStarted", 400),
    ("discountCodeExpired", 400),
    ("discountCodeUsageLimitReached", 400),
    ("discountCodeAlreadyUsed", 400),
    ("discountCodeNotEligible", 400),
    ("discountCodeMinimumNotMet", 400),
    ("discountCodeNotApplicable", 400),
    ("orderEmpty", 400),
    ("orderingUnavailable", 400),
    ("orderProductUnavailable", 400),
    ("orderOutOfStock", 400),
    ("orderBelowMinimum", 400),
    ("orderBelowVendorMinimum", 400),
    ("orderShippingTypeUnavailable", 400),
    ("paymentMethodUnavailable", 400),
    ("orderPaymentContactRequired", 400),
    PAYMENT_PROVIDER_ERRORS[0],
    PAYMENT_PROVIDER_ERRORS[1],
    PAYMENT_PROVIDER_ERRORS[2],
    PAYMENT_PROVIDER_ERRORS[3],
    PAYMENT_PROVIDER_ERRORS[4],
];

/// `possibleErrorsOnTransitioningOrder`.
const TRANSITIONING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    PAYMENT_PROVIDER_ERRORS[0],
    PAYMENT_PROVIDER_ERRORS[1],
    PAYMENT_PROVIDER_ERRORS[2],
    PAYMENT_PROVIDER_ERRORS[3],
    PAYMENT_PROVIDER_ERRORS[4],
    ("orderInvalidStatusTransition", 409),
    ("orderAwaitingPayment", 409),
    ("orderRefundFailed", 502),
    ("orderPaymentProcessing", 409),
];

/// `possibleErrorsOnDeletingOrder`.
const DELETING: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    PAYMENT_PROVIDER_ERRORS[0],
    PAYMENT_PROVIDER_ERRORS[1],
    PAYMENT_PROVIDER_ERRORS[2],
    PAYMENT_PROVIDER_ERRORS[3],
    PAYMENT_PROVIDER_ERRORS[4],
    ("orderPaidOnline", 409),
];

/// `possibleErrorsOnStartingOrderPayment`.
const STARTING_PAYMENT: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    PAYMENT_PROVIDER_ERRORS[0],
    PAYMENT_PROVIDER_ERRORS[1],
    PAYMENT_PROVIDER_ERRORS[2],
    PAYMENT_PROVIDER_ERRORS[3],
    PAYMENT_PROVIDER_ERRORS[4],
    ("orderPaymentNotOpen", 409),
    ("orderPaymentContactRequired", 400),
];

/// The order services' side effects from the API state.
pub(crate) fn deps(state: &ApiState) -> Deps<'_> {
    Deps {
        db: &state.db,
        jobs: state.jobs.as_deref(),
        events: &state.events,
        realtime: state.realtime.as_ref(),
        encryption: state.encryption.as_deref(),
        app_url: &state.app_url,
    }
}

/// An array of enum strings (`z.array(enum).optional()`).
fn enum_list(input: &mut Input, name: &str, options: &[&str]) -> Vec<String> {
    let Some(value) = input.get(name).cloned() else {
        return Vec::new();
    };
    let Value::Array(items) = value else {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected array, received string",
        );
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key(name.to_string()), Seg::Index(index)];
        if let Some(v) = input.enum_at(&path, Some(item), options, true) {
            out.push(v);
        }
    }
    out
}

fn opt_date(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned()?;
    let parsed = coerce_date(&value);
    if parsed.is_none() {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

/// `listOrdersPublicRequest` (without `page` / `perPage` for the export).
fn parse_list(input: &mut Input, workspace_id: i64, paged: bool) -> OrderListInput {
    let status = enum_list(input, "status", order::STATUSES);
    let source = enum_list(input, "source", order::SOURCES);
    let payment_status = enum_list(input, "paymentStatus", order::PAYMENT_STATUSES);
    let fulfillment_status = enum_list(input, "fulfillmentStatus", order::FULFILLMENT_STATUSES);
    let contact_id = if input.has("contactId") {
        input.id("contactId")
    } else {
        None
    };
    let from = opt_date(input, "from");
    let to = opt_date(input, "to");
    let keyword = input.opt_string("keyword", Str::trimmed(0, 255));
    let (page, per_page) = if paged {
        (
            Some(
                input
                    .opt_coerce_int("page", Num::int(1.0, f64::MAX))
                    .unwrap_or(1),
            ),
            Some(
                input
                    .opt_coerce_int("perPage", Num::int(1.0, 100.0))
                    .unwrap_or(20),
            ),
        )
    } else {
        (None, None)
    };
    OrderListInput {
        workspace_id,
        status,
        source,
        payment_status,
        fulfillment_status,
        contact_id,
        from,
        to,
        keyword,
        page,
        per_page,
        sort: None,
    }
}

/// `orderShippingAddressSchema`.
fn parse_address(input: &mut Input, value: &Value) -> Option<Value> {
    let path = key("shippingAddress");
    let Value::Object(obj) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let mut out = Map::new();
    for (field, max) in [
        ("line1", 500),
        ("line2", 500),
        ("city", 255),
        ("state", 255),
        ("zip", 32),
        ("country", 2),
    ] {
        match obj.get(field) {
            None => {}
            Some(Value::Null) => {
                out.insert(field.into(), Value::Null);
            }
            Some(v) => {
                if let Some(text) =
                    input.string_at(&child(&path, field), Some(v), Str::trimmed(0, max), true)
                {
                    out.insert(field.into(), json!(text));
                }
            }
        }
    }
    Some(Value::Object(out))
}

/// `createOrderPublicRequest`.
fn parse_create(input: &mut Input) -> (PlaceInput, Option<String>) {
    let payment_method = input.get("paymentMethod").cloned().and_then(|v| {
        input.enum_at(
            &key("paymentMethod"),
            Some(&v),
            order::PAYMENT_METHODS,
            true,
        )
    });
    let contact = input.opt_string(
        "contact",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
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
                    input.id_at(&child(&path, "productId"), obj.get("productId"), true);
                let variant_id = match obj.get("variantId") {
                    None | Some(Value::Null) => None,
                    Some(v) => input.id_at(&child(&path, "variantId"), Some(v), true),
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
        Some(_) => input.issue(
            &key("items"),
            "invalid_type",
            "Invalid input: expected array",
        ),
        None => input.issue(
            &key("items"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        ),
    }
    let customer_name = input
        .nullish_string("customerName", Str::trimmed(0, 255))
        .flatten();
    let customer_email = input
        .nullish_string("customerEmail", Str::trimmed(0, 255))
        .flatten();
    if customer_email.as_deref().is_some_and(|e| !is_email(e)) {
        input.issue(
            &key("customerEmail"),
            "invalid_format",
            "Invalid email address",
        );
    }
    let customer_phone = input
        .nullish_string("customerPhone", Str::trimmed(0, 64))
        .flatten();
    let shipping_type = match input.get("shippingType").cloned() {
        None | Some(Value::Null) => None,
        Some(v) => input.enum_at(
            &key("shippingType"),
            Some(&v),
            &["delivery", "pickup"],
            true,
        ),
    };
    let shipping_address = match input.get("shippingAddress").cloned() {
        None | Some(Value::Null) => None,
        Some(v) => parse_address(input, &v),
    };
    let note = input
        .nullish_string("note", Str::trimmed(0, 2000))
        .flatten();
    let discount_code = input
        .nullish_string("discountCode", Str::trimmed(0, 64))
        .flatten();
    let place = PlaceInput {
        source: "api",
        actor_kind: API_TOKEN_ACTOR.kind,
        items,
        customer_name,
        customer_email,
        customer_phone,
        shipping_type,
        shipping_address,
        note,
        discount_code,
        payment_method,
        ..PlaceInput::default()
    };
    (place, contact)
}

fn order_id(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let id = input.id("id");
    input.finish()?;
    Ok(id.unwrap_or_default())
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let detail = |d: &chatbotx_db::repo::order::OrderDetail| json(200, order::detail_json(d));
    match route_id {
        "orders.list" | "orders.export" => {
            let mut input = bracket_query(call.params, call.query);
            let list = parse_list(&mut input, ws, route_id == "orders.list");
            input.finish()?;
            if route_id == "orders.list" {
                let page = order::list(&state.db, list)
                    .await
                    .map_err(|e| business_error(e, super::LISTING))?;
                Ok(json(200, order::page_json(&page)))
            } else {
                let csv = order::export_csv(&state.db, list)
                    .await
                    .map_err(|e| business_error(e, super::LISTING))?;
                Ok(json(200, csv))
            }
        }
        "orders.get" => {
            let id = order_id(&call)?;
            let d = order::get_detail(&state.db, ws, id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(detail(&d))
        }
        "orders.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let (mut place, contact) = parse_create(&mut input);
            input.finish()?;
            let contact_id = match contact {
                Some(identifier) => Some(
                    resolve_id_by_identifier(&state.db, ws, &identifier)
                        .await
                        .map_err(|e| business_error(e, PLACING))?,
                ),
                None => None,
            };
            place.workspace_id = ws;
            place.contact = Some(ContactRef {
                contact_id,
                ..ContactRef::default()
            });
            let d = order::place(deps(state), place, None)
                .await
                .map_err(|e| business_error(e, PLACING))?;
            Ok(detail(&d))
        }
        "orders.getPaymentLink" => {
            let id = order_id(&call)?;
            let (url, expires_at) = order_payment::start_checkout(deps(state), ws, id)
                .await
                .map_err(|e| business_error(e, STARTING_PAYMENT))?;
            Ok(json(
                200,
                json!({ "url": url, "expiresAt": date(expires_at) }),
            ))
        }
        "orders.accept" | "orders.ship" | "orders.conclude" | "orders.cancel" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let message = input
                .nullish_string("message", Str::trimmed(0, 1000))
                .flatten();
            input.finish()?;
            let to = match route_id {
                "orders.accept" => "accepted",
                "orders.ship" => "shipped",
                "orders.conclude" => "concluded",
                _ => "cancelled",
            };
            let d = order::transition(
                deps(state),
                ws,
                id.unwrap_or_default(),
                to,
                API_TOKEN_ACTOR,
                message.as_deref(),
                false,
            )
            .await
            .map_err(|e| business_error(e, TRANSITIONING))?;
            Ok(detail(&d))
        }
        "orders.delete" => {
            let id = order_id(&call)?;
            order::delete(deps(state), ws, id)
                .await
                .map_err(|e| business_error(e, DELETING))?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
