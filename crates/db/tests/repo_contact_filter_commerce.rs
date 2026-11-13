//! Port of `packages/database/__tests__/contact-filter-commerce.test.ts` (P3.2b slice C1).
//! The TS rendered the RAW predicate with `PgDialect`; here the SQLite text and binds of the
//! same one-condition filter (adapted: `jsonb_array_elements` -> `json_each`, `::bigint`
//! -> `CAST(.. AS INTEGER)`), prepared on the migrated schema. Money: `Order.total` is the
//! stored x 10,000 integer, so the sum is compared in major units (`SUM(..) / 10000.0`).

mod common;
#[path = "common/repo8.rs"]
mod repo8;

use chatbotx_db::repo::sql::Sql;
use repo8::{and1, apply, has, has_param, int, lacks, lacks_param, prepares, text};
use serde_json::{Value as Json, json};

/// The predicate of a one-condition filter without the AND parentheses, or `None`.
fn raw(c: Json) -> Option<Sql> {
    let q = apply(and1(c))?;
    prepares(&q);
    let text = q.text[1..q.text.len() - 1].to_owned();
    Some(Sql::new(text, q.params))
}

fn r(c: Json) -> Sql {
    raw(c).expect("a predicate")
}

fn starts_exists(q: &Sql) {
    assert!(q.text.starts_with("EXISTS ("), "{}", q.text);
}

fn starts_not_exists(q: &Sql) {
    assert!(q.text.starts_with("NOT EXISTS ("), "{}", q.text);
}

// ts:23 adapted
#[test]
fn purchase_conditions_total_spent_sums_the_contact_s_orders_that_are_not_cancelled_0_when_none() {
    let q = r(json!({ "field": "totalSpent", "operator": "gte", "value": "100" }));
    has(&q, "SUM(\"Order\".\"total\")");
    has(&q, "\"Order\".\"status\" <> 'cancelled'");
    has(&q, "COALESCE(");
    has(&q, ">=");
    has_param(&q, int(100));
}

// ts:32 adapted
#[test]
fn purchase_conditions_number_of_orders_counts_them() {
    has(
        &r(json!({ "field": "numberOfOrders", "operator": "eq", "value": "0" })),
        "COUNT(*)",
    );
}

// ts:41 adapted
#[test]
fn purchase_conditions_total_spent_only_sums_orders_in_the_store_currency() {
    has(
        &r(json!({ "field": "totalSpent", "operator": "gt", "value": "0" })),
        "\"EcommerceSettings\".\"currency\"",
    );
}

// ts:46 adapted
#[test]
fn purchase_conditions_counts_last_purchase_and_bought_items_ignore_the_currency() {
    for field in ["numberOfOrders", "bought", "boughtItems"] {
        let (op, value) = if field == "bought" {
            ("gte", "2026-10-01")
        } else {
            ("eq", "1")
        };
        lacks(
            &r(json!({ "field": field, "operator": op, "value": value })),
            "\"EcommerceSettings\"",
        );
    }
}

// ts:57 ported
#[test]
fn purchase_conditions_a_non_numeric_amount_gives_no_predicate() {
    assert!(raw(json!({ "field": "totalSpent", "operator": "gt", "value": "lots" })).is_none());
}

// ts:63 adapted
#[test]
fn purchase_conditions_bought_compares_the_latest_order_s_date() {
    has(
        &r(json!({ "field": "bought", "operator": "gte", "value": "2026-10-01" })),
        "MAX(\"Order\".\"createdAt\")",
    );
}

// ts:72 adapted
#[test]
fn purchase_conditions_bought_the_items_a_product_of_a_counted_order() {
    let q = r(json!({ "field": "boughtItems", "operator": "eq", "value": "42" }));
    starts_exists(&q);
    has(&q, "\"OrderItem\".\"productId\" IN (");
}

// ts:78 adapted
#[test]
fn purchase_conditions_not_bought_the_items_also_finds_contacts_who_never_bought() {
    starts_not_exists(&r(
        json!({ "field": "boughtItems", "operator": "ne", "value": "42" }),
    ));
}

// ts:83 adapted
#[test]
fn purchase_conditions_a_malformed_product_id_is_dropped_instead_of_failing_the_query() {
    let q = r(
        json!({ "field": "boughtItems", "operator": "in", "value": ["abc", "42", "99999999999999999999"] }),
    );
    lacks(&q, "abc");
    has_param(&q, int(42));
    lacks_param(&q, text("99999999999999999999"));
    assert_eq!(q.params.len(), 1);
    let none = r(json!({ "field": "boughtItems", "operator": "eq", "value": "abc" }));
    starts_exists(&none);
    lacks(&none, "\"OrderItem\".\"productId\" IN (");
}

// ts:97 adapted
#[test]
fn purchase_conditions_never_bought_anything() {
    let q = r(json!({ "field": "boughtItems", "operator": "isEmpty" }));
    starts_not_exists(&q);
    lacks(&q, "\"OrderItem\".\"productId\" IN");
}

// ts:105 adapted
#[test]
fn web_shop_cart_conditions_cart_contains_a_product() {
    let q = r(json!({ "field": "shoppingCartContainsItems", "operator": "eq", "value": "42" }));
    starts_exists(&q);
    has(&q, "json_each(\"Cart\".\"items\")");
    has(&q, "CAST(\"cartItem\".value ->> 'productId' AS TEXT) IN (");
    has_param(&q, text("42"));
}

// ts:117 adapted
#[test]
fn web_shop_cart_conditions_cart_without_the_product_includes_contacts_without_a_cart() {
    starts_not_exists(&r(
        json!({ "field": "shoppingCartContainsItems", "operator": "ne", "value": "42" }),
    ));
}

// ts:127 adapted
#[test]
fn web_shop_cart_conditions_cart_is_empty_no_item_at_all() {
    starts_not_exists(&r(
        json!({ "field": "shoppingCartIsEmpty", "operator": "eq", "value": "true" }),
    ));
    starts_exists(&r(
        json!({ "field": "shoppingCartIsEmpty", "operator": "eq", "value": "false" }),
    ));
}

// ts:138 adapted
#[test]
fn web_shop_cart_conditions_cart_value_is_priced_at_today_s_product_or_variant_price() {
    let subtotal = r(json!({ "field": "shoppingCartSubtotal", "operator": "gt", "value": "50" }));
    has(
        &subtotal,
        "COALESCE(\"ProductVariant\".\"price\", \"Product\".\"price\")",
    );
    lacks(&subtotal, "\"Product\".\"discount\"");
    let total = r(json!({ "field": "shoppingCartTotal", "operator": "gt", "value": "50" }));
    has(&total, "\"Product\".\"discount\"");
}
