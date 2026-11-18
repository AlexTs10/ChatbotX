//! The orders, discount codes, settings and analytics screens
//! (`app/space/[workspaceId]/(e-commerce)/{orders,discount-codes,
//! ecommerce-settings,ecommerce-analytics}`, `features/{orders,
//! discount-codes,ecommerce-settings}/actions`).
//!
//! Page queries: `ordersPage`, `orderDetailPage`, `discountCodesPage`,
//! `discountCodeFormPage`, `ecommerceSettingsPage`, `ecommerceAnalyticsPage`.
//! The actions call the business services, or the ported `/v1` route when
//! the input is the same ([`public`]).

use chatbotx_business::order::{self, Actor};
use chatbotx_business::product_category::{self, ListFilter};
use chatbotx_business::store_source::resolve_store_source;
use chatbotx_business::{
    BusinessError, discount_code, ecommerce_analytics, ecommerce_settings, flow, payment_provider,
    tx,
};
use chatbotx_db::repo::order::OrderListInput;
use chatbotx_db::repo::{iso, product as product_repo};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, schema_errors};
use crate::handlers::ecommerce_analytics::is_day;
use crate::handlers::orders::deps;
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "ordersPage",
    "orderDetailPage",
    "discountCodesPage",
    "discountCodeFormPage",
    "ecommerceSettingsPage",
    "ecommerceAnalyticsPage",
    "deleteOrderAction",
    "exportOrdersAction",
    "getOrderDetailAction",
    "transitionOrderAction",
    "cancelOrderAction",
    "createDiscountCodeAction",
    "updateDiscountCodeAction",
    "setDiscountCodeActiveAction",
    "deleteDiscountCodeAction",
    "generateDiscountCodeAction",
    "updateEcommerceSettingsAction",
    "connectShopifyAction",
    "syncShopifyAction",
    "disconnectShopifyAction",
    "connectWooCommerceAction",
    "syncWooCommerceAction",
    "disconnectWooCommerceAction",
];

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The `ecommerce` permission of the e-commerce layout (`notFound()`).
fn require_ecommerce(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.has("ecommerce") {
        Ok(())
    } else {
        Err(fail(BusinessError::not_found("Page not found")))
    }
}

/// The id bound at `index` (`zodBigintAsString()`).
fn bound_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

/// `{ currency, currencySymbol, currencyPosition }` of the store.
async fn store(ctx: &Ctx<'_>) -> Result<Value, OrpcError> {
    let settings = ecommerce_settings::get(&ctx.api.db, ctx.workspace_id)
        .await
        .map_err(fail)?;
    let p = &settings["payments"];
    Ok(json!({
        "currency": p["currency"],
        "currencySymbol": p["currencySymbol"],
        "currencyPosition": p["currencyPosition"],
    }))
}

/// `z.array(enum).optional()` of the list searches; anything else is dropped.
fn enum_list(search: &Value, key: &str, options: &[&str]) -> Vec<String> {
    search[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|v| options.contains(v))
        .map(str::to_string)
        .collect()
}

fn order_list(ctx: &Ctx<'_>, search: &Value, paged: bool) -> Result<OrderListInput, OrpcError> {
    let s = In(search);
    Ok(OrderListInput {
        workspace_id: ctx.workspace_id,
        keyword: s
            .opt_str("keyword")?
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty()),
        payment_status: enum_list(search, "paymentStatus", order::PAYMENT_STATUSES),
        fulfillment_status: enum_list(search, "fulfillmentStatus", order::FULFILLMENT_STATUSES),
        page: if paged {
            Some(s.opt_positive("page")?.unwrap_or(1))
        } else {
            None
        },
        per_page: if paged {
            Some(s.opt_positive("perPage")?.unwrap_or(20).min(100))
        } else {
            None
        },
        ..OrderListInput::default()
    })
}

/// `z.string().trim().max(1000).nullish()`.
fn message(body: &Value) -> Result<Option<String>, OrpcError> {
    let text = In(body).opt_str("message")?.map(|m| m.trim().to_string());
    if text.as_ref().is_some_and(|m| m.chars().count() > 1000) {
        return Err(invalid());
    }
    Ok(text)
}

/// The SPA form posts dates as ISO strings and `""` for "none"; the `/v1`
/// write schema takes the same body.
async fn write_code(
    ctx: &Ctx<'_>,
    route: &'static str,
    id: Option<i64>,
    body: &Value,
) -> Result<Value, OrpcError> {
    let params: Vec<(&'static str, String)> =
        id.map(|id| ("id", id.to_string())).into_iter().collect();
    let row = public(ctx, route, &params, None, Some(body))
        .await
        .map_err(|e| action_error(schema_errors(e)))?;
    Ok(json!({ "id": row["id"] }))
}

/// A `validation` exception with a field is `validationErrors`, as [`action_fail`].
fn action_error(mut error: OrpcError) -> OrpcError {
    if let Some(field) = error
        .data
        .as_ref()
        .and_then(|d| d.get("field"))
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        error.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                field: { "_errors": [error.message] },
            }
        }));
    }
    error
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id;
    let db = &ctx.api.db;
    let user = Actor {
        kind: "user",
        user_id: Some(ctx.user_id),
    };
    match name {
        "ordersPage" => {
            require_ecommerce(ctx)?;
            let list = order::list(db, order_list(ctx, arg(args, 1), true)?)
                .await
                .map_err(fail)?;
            let links = chatbotx_business::external_store::store_links(db, &ctx.api.app_url, ws)
                .await
                .map_err(fail)?;
            Ok(
                json!({ "orders": order::page_json(&list), "store": store(ctx).await?, "externalStore": links }),
            )
        }
        "orderDetailPage" => {
            require_ecommerce(ctx)?;
            let id = In(arg(args, 1)).id("orderId")?;
            let detail = order::get_detail(db, ws, id).await.map_err(fail)?;
            Ok(json!({ "order": order::detail_json(&detail), "store": store(ctx).await? }))
        }
        "discountCodesPage" => {
            require_ecommerce(ctx)?;
            let s = In(arg(args, 1));
            let code = s
                .opt_str("code")?
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty());
            let (rows, page_count) = discount_code::list(
                db,
                ws,
                code,
                s.opt_positive("page")?,
                s.opt_positive("perPage")?,
            )
            .await
            .map_err(fail)?;
            Ok(json!({
                "discountCodes": super::page(rows.iter().map(discount_code::resource_json), page_count),
                "store": store(ctx).await?,
            }))
        }
        "discountCodeFormPage" => {
            require_ecommerce(ctx)?;
            let discount_code = match In(arg(args, 1)).opt_id("id")? {
                Some(id) => Some(discount_code::resource_json(
                    &discount_code::get(db, ws, id).await.map_err(fail)?,
                )),
                None => None,
            };
            let categories = product_category::list(db, ws, ListFilter::default())
                .await
                .map_err(fail)?;
            let products = tx::read(db, move |conn| {
                let source = resolve_store_source(conn, ws)?;
                Ok(product_repo::list_form_options(conn, ws, Some(&source))?.0)
            })
            .await
            .map_err(fail)?;
            let settings = ecommerce_settings::get(db, ws).await.map_err(fail)?;
            let p = &settings["payments"];
            Ok(json!({
                "categories": categories.iter().map(|c| json!({
                    "id": c.id.to_string(),
                    "name": c.name,
                    "parentId": c.parent_id.map(|id| id.to_string()),
                })).collect::<Vec<_>>(),
                "products": products.into_iter().map(|(id, name)| json!({ "id": id.to_string(), "name": name })).collect::<Vec<_>>(),
                "currency": if p["currencySymbol"].is_string() { p["currencySymbol"].clone() } else { p["currency"].clone() },
                "discountCode": discount_code,
            }))
        }
        "ecommerceSettingsPage" => {
            require_ecommerce(ctx)?;
            let settings = public(ctx, "ecommerceSettings.get", &[], None, None).await?;
            let flows = flow::list_published_options(db, ws).await.map_err(fail)?;
            let open_hours = public(ctx, "openHours.list", &[], None, None).await?["data"].take();
            let providers = payment_provider::list(db, ws).await.map_err(fail)?;
            let shopify_status = chatbotx_business::shopify::get_status(db, &ctx.api.app_url, ws)
                .await
                .map_err(fail)?;
            let woocommerce_status =
                chatbotx_business::woocommerce::get_status(db, &ctx.api.app_url, ws)
                    .await
                    .map_err(fail)?;
            Ok(json!({
                "settings": settings,
                "flowOptions": flows.iter().map(|f| json!({ "id": f.id.to_string(), "name": f.name })).collect::<Vec<_>>(),
                "openHours": open_hours,
                "connectedProviders": providers.iter().map(|p| p["provider"].clone()).collect::<Vec<_>>(),
                "shopifyStatus": shopify_status,
                "wooCommerceStatus": woocommerce_status,
            }))
        }
        "ecommerceAnalyticsPage" => {
            require_ecommerce(ctx)?;
            let s = In(arg(args, 1));
            let day = |key: &str| -> Result<Option<String>, OrpcError> {
                Ok(s.opt_str(key)?.filter(|d| is_day(d)))
            };
            let data = ecommerce_analytics::get_dashboard(db, ws, day("from")?, day("to")?)
                .await
                .map_err(fail)?;
            let created_at = tx::read(db, move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "createdAt" FROM "Workspace" WHERE "id" = ?1"#,
                        params![ws],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .unwrap_or_default())
            })
            .await
            .map_err(fail)?;
            Ok(json!({
                "data": data,
                "store": store(ctx).await?,
                "referenceNow": iso(chatbotx_db::repo::now_ms()),
                "workspaceCreatedAt": iso(created_at),
            }))
        }
        "deleteOrderAction" => order::delete(deps(ctx.api), ws, bound_id(args, 1)?)
            .await
            .map(|()| Value::Null)
            .map_err(action_fail),
        "getOrderDetailAction" => order::get_detail(db, ws, bound_id(args, 1)?)
            .await
            .map(|d| order::detail_json(&d))
            .map_err(action_fail),
        "exportOrdersAction" => order::export_csv(db, order_list(ctx, arg(args, 1), false)?)
            .await
            .map_err(action_fail),
        "transitionOrderAction" | "cancelOrderAction" => {
            let id = bound_id(args, 1)?;
            let body = arg(args, 2);
            let to: &'static str = if name == "cancelOrderAction" {
                "cancelled"
            } else {
                match body["to"].as_str() {
                    Some("accepted") => "accepted",
                    Some("shipped") => "shipped",
                    Some("concluded") => "concluded",
                    _ => return Err(invalid()),
                }
            };
            let message = message(body)?;
            order::transition(deps(ctx.api), ws, id, to, user, message.as_deref(), false)
                .await
                .map(|_| Value::Null)
                .map_err(action_fail)
        }
        "createDiscountCodeAction" => {
            write_code(ctx, "discountCodes.create", None, arg(args, 1)).await
        }
        // The form always sends every field, so this replaces the whole code.
        "updateDiscountCodeAction" => {
            let id = bound_id(args, 1)?;
            write_code(ctx, "discountCodes.update", Some(id), arg(args, 2)).await
        }
        "setDiscountCodeActiveAction" => {
            let id = bound_id(args, 1)?;
            let active = In(arg(args, 2)).opt_bool("isActive")?.ok_or_else(invalid)?;
            discount_code::set_active(db, ws, id, active)
                .await
                .map(|_| Value::Null)
                .map_err(action_fail)
        }
        "deleteDiscountCodeAction" => discount_code::delete(db, ws, bound_id(args, 1)?)
            .await
            .map(|_| Value::Null)
            .map_err(action_fail),
        "generateDiscountCodeAction" => discount_code::generate_code(db, ws)
            .await
            .map(|code| json!({ "code": code }))
            .map_err(action_fail),
        // The same calls `shopify.*` / `woocommerce.*` make.
        "connectShopifyAction"
        | "syncShopifyAction"
        | "disconnectShopifyAction"
        | "connectWooCommerceAction"
        | "syncWooCommerceAction"
        | "disconnectWooCommerceAction" => {
            let store = if name.contains("Shopify") {
                "shopify"
            } else {
                "woocommerce"
            };
            let verb = if name.starts_with("connect") {
                "connect"
            } else if name.starts_with("sync") {
                "sync"
            } else {
                "disconnect"
            };
            let route = format!("{store}.{verb}");
            let body = (verb != "disconnect").then(|| arg(args, 1));
            public(ctx, &route, &[], None, body)
                .await
                .map(|v| if verb == "disconnect" { Value::Null } else { v })
                .map_err(|e| action_error(schema_errors(e)))
        }
        _ => public(
            ctx,
            "ecommerceSettings.update",
            &[],
            None,
            Some(arg(args, 1)),
        )
        .await
        .map(|_| Value::Null)
        .map_err(|e| action_error(schema_errors(e))),
    }
}
