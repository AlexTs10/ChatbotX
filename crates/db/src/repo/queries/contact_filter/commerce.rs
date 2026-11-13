//! Port of `queries/contact-filter/commerce.ts`: purchase and web shop cart conditions.
//!
//! Money: `Order.total` is the stored x 10,000 integer (P2.5), so the spent sum is read in
//! major units (`SUM(..) / 10000.0`) to compare with the typed filter value.
//! `Product.price` / `ProductVariant.price` are REAL major units.

use std::sync::LazyLock;

use chrono_tz::Tz;
use regex::Regex;
use rusqlite::types::Value;
use serde_json::Value as Json;

use super::predicates::{datetime_aggregate_comparison, number_aggregate_comparison};
use super::sqlx::{exists, in_list, string_values};
use crate::repo::sql::Sql;

const COUNTED_ORDERS: &str = "\"Order\".\"contactId\" = \"Contact\".\"id\" AND \"Order\".\"workspaceId\" = \"Contact\".\"workspaceId\" AND \"Order\".\"status\" <> 'cancelled'";

const CART_ITEMS: &str = "SELECT 1 FROM \"Cart\", json_each(\"Cart\".\"items\") AS \"cartItem\" WHERE \"Cart\".\"contactId\" = \"Contact\".\"id\" AND \"Cart\".\"workspaceId\" = \"Contact\".\"workspaceId\"";

/// `PRODUCT_ID_PATTERN`: a positive bigint within range.
static PRODUCT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{1,18}$").expect("regex"));

fn store_currency_orders() -> String {
    format!(
        "{COUNTED_ORDERS} AND \"Order\".\"currency\" = COALESCE((SELECT \"EcommerceSettings\".\"currency\" FROM \"EcommerceSettings\" WHERE \"EcommerceSettings\".\"workspaceId\" = \"Contact\".\"workspaceId\"), \"Order\".\"currency\")"
    )
}

/// The purchase / cart condition of `field`.
pub(crate) fn commerce_where(
    field: &str,
    operator: &str,
    value: Option<&Json>,
    zone: Tz,
) -> Option<Sql> {
    match field {
        "totalSpent" | "numberOfOrders" => order_number_where(field, operator, value),
        "bought" => last_purchase_where(operator, value, zone),
        "boughtItems" => bought_items_where(operator, value),
        "shoppingCartContainsItems" => cart_contains_where(operator, value),
        "shoppingCartIsEmpty" => (operator == "eq")
            .then(|| exists(Sql::raw(CART_ITEMS), super::predicates::is_true(value))),
        "shoppingCartTotal" => cart_value_where(true, operator, value),
        "shoppingCartSubtotal" => cart_value_where(false, operator, value),
        _ => None,
    }
}

/// `buildOrderNumberWhere`.
fn order_number_where(field: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    number_aggregate_comparison(operator, value, "0")?;
    let of = if field == "totalSpent" {
        format!(
            "COALESCE((SELECT SUM(\"Order\".\"total\") / 10000.0 FROM \"Order\" WHERE {}), 0)",
            store_currency_orders()
        )
    } else {
        format!("COALESCE((SELECT COUNT(*) FROM \"Order\" WHERE {COUNTED_ORDERS}), 0)")
    };
    Some(number_aggregate_comparison(operator, value, &of).unwrap_or_else(|| Sql::raw("FALSE")))
}

/// `buildLastPurchaseWhere`.
fn last_purchase_where(operator: &str, value: Option<&Json>, zone: Tz) -> Option<Sql> {
    datetime_aggregate_comparison(operator, value, "NULL", zone)?;
    let of = format!("(SELECT MAX(\"Order\".\"createdAt\") FROM \"Order\" WHERE {COUNTED_ORDERS})");
    Some(
        datetime_aggregate_comparison(operator, value, &of, zone)
            .unwrap_or_else(|| Sql::raw("FALSE")),
    )
}

/// `boughtItemExists`.
fn bought_item_exists(predicate: Option<Sql>, negate: bool) -> Sql {
    let mut inner = Sql::raw(format!(
        "SELECT 1 FROM \"OrderItem\" INNER JOIN \"Order\" ON \"Order\".\"id\" = \"OrderItem\".\"orderId\" WHERE {COUNTED_ORDERS}"
    ));
    if let Some(p) = predicate {
        inner.push(" AND ");
        inner.append(p);
    }
    exists(inner, negate)
}

/// `buildBoughtItemsWhere`.
fn bought_items_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator == "isEmpty" {
        return Some(bought_item_exists(None, true));
    }
    let values = string_values(value);
    if values.is_empty() {
        return None;
    }
    let negative = matches!(operator, "ne" | "notIn");
    if !(negative || matches!(operator, "eq" | "in")) {
        return None;
    }
    let ids: Vec<Value> = values
        .iter()
        .filter(|v| PRODUCT_ID.is_match(v))
        .filter_map(|v| v.parse::<i64>().ok())
        .map(Value::Integer)
        .collect();
    let predicate = if ids.is_empty() {
        Sql::raw("FALSE")
    } else {
        in_list("\"OrderItem\".\"productId\"", ids, false)
    };
    Some(bought_item_exists(Some(predicate), negative))
}

/// `buildCartContainsWhere`.
fn cart_contains_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator == "isEmpty" {
        return Some(exists(Sql::raw(CART_ITEMS), true));
    }
    let values = string_values(value);
    if values.is_empty() {
        return None;
    }
    let negative = matches!(operator, "ne" | "notIn");
    if !(negative || matches!(operator, "eq" | "in")) {
        return None;
    }
    let mut inner = Sql::raw(format!("{CART_ITEMS} AND "));
    inner.append(in_list(
        "CAST(\"cartItem\".value ->> 'productId' AS TEXT)",
        values.into_iter().map(Value::Text).collect(),
        false,
    ));
    Some(exists(inner, negative))
}

/// `buildCartValueWhere`: the cart priced at today's product / variant price.
fn cart_value_where(with_discount: bool, operator: &str, value: Option<&Json>) -> Option<Sql> {
    number_aggregate_comparison(operator, value, "0")?;
    let mut line = "CAST(\"cartItem\".value ->> 'quantity' AS REAL) * COALESCE(\"ProductVariant\".\"price\", \"Product\".\"price\")".to_owned();
    if with_discount {
        line.push_str(" * (1 - \"Product\".\"discount\" / 100.0)");
    }
    let of = format!(
        "COALESCE((SELECT SUM({line}) FROM \"Cart\", json_each(\"Cart\".\"items\") AS \"cartItem\" INNER JOIN \"Product\" ON \"Product\".\"id\" = CAST(\"cartItem\".value ->> 'productId' AS INTEGER) AND \"Product\".\"workspaceId\" = \"Cart\".\"workspaceId\" LEFT JOIN \"ProductVariant\" ON \"ProductVariant\".\"id\" = CAST(NULLIF(\"cartItem\".value ->> 'variantId', '') AS INTEGER) WHERE \"Cart\".\"contactId\" = \"Contact\".\"id\" AND \"Cart\".\"workspaceId\" = \"Contact\".\"workspaceId\"), 0)"
    );
    Some(number_aggregate_comparison(operator, value, &of).unwrap_or_else(|| Sql::raw("FALSE")))
}
