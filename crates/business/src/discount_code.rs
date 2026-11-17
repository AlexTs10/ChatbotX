//! The two `discountCodeRepository` writes the catalog services run in their
//! delete transactions (private queries; they move to `crates/db` later).

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::errors::{BusinessError, Result};

fn id_list(ids: &[i64]) -> String {
    serde_json::Value::from(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>()).to_string()
}

/// `deactivateByCategoryIds`: switches off the `category` codes of these categories.
pub fn deactivate_by_category_ids(
    conn: &Connection,
    workspace_id: i64,
    category_ids: &[i64],
) -> Result<()> {
    if category_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        r#"UPDATE "DiscountCode" SET "isActive" = 0 WHERE "workspaceId" = ?1 AND "appliesTo" = 'category'
           AND "categoryId" IN (SELECT CAST(value AS INTEGER) FROM json_each(?2))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(category_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}

/// `removeProductIds`: drops the ids from `products` codes; a code left with
/// no product is switched off.
pub fn remove_product_ids(conn: &Connection, workspace_id: i64, product_ids: &[i64]) -> Result<()> {
    if product_ids.is_empty() {
        return Ok(());
    }
    // `productIds` holds the ids as JSON strings (or numbers); compare as text.
    conn.execute(
        r#"UPDATE "DiscountCode" SET
             "productIds" = coalesce((SELECT json_group_array(l.value) FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2))), '[]'),
             "isActive" = CASE WHEN EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2)))
                               THEN "isActive" ELSE 0 END
           WHERE "workspaceId" = ?1 AND "appliesTo" = 'products'
             AND EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                         WHERE CAST(l.value AS TEXT) IN (SELECT value FROM json_each(?2)))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(product_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}

// ---- discount-code/{service,pricing,errors}.ts: pricing a code on an order --

use chatbotx_core::utils::money::{percent_of, round_money, sum_money};
use chatbotx_db::repo::models::DiscountCode;
use chatbotx_db::repo::queries::contact_filter::{
    ContactWhereInput, build_contact_where, contact_filter_has_predicate,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{find_many, find_one, iso, new_id, now_ms};
use serde_json::json;

const MONEY_SCALE: f64 = 10_000.0;

fn major(scaled: i64) -> f64 {
    scaled as f64 / MONEY_SCALE
}

fn discount_error(message: impl Into<String>, code: &str) -> BusinessError {
    BusinessError::exception(message, code, 400)
}

pub fn not_found() -> BusinessError {
    BusinessError::exception(
        "This discount code does not exist.",
        "discountCodeNotFound",
        404,
    )
}

fn not_applicable() -> BusinessError {
    discount_error(
        "This discount code does not apply to anything in the order.",
        "discountCodeNotApplicable",
    )
}

fn used_up() -> BusinessError {
    discount_error(
        "This discount code has been used up.",
        "discountCodeUsageLimitReached",
    )
}

/// `CouponLine` (major units).
#[derive(Debug, Clone, PartialEq)]
pub struct CouponLine {
    pub product_id: Option<i64>,
    pub category_ids: Vec<i64>,
    pub amount: f64,
    pub quantity: i64,
}

/// `DiscountOrderContext`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderContext {
    pub workspace_id: i64,
    pub source: String,
    pub contact_id: Option<i64>,
    pub lines: Vec<CouponLine>,
    pub shipping_type: Option<String>,
    pub shipping_total: f64,
    pub currency: String,
}

/// `AppliedDiscount` (what an order keeps of the code).
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub id: i64,
    pub code: String,
    pub discount_type: String,
    pub discount_value: f64,
    pub ends_at: Option<i64>,
    pub amount: f64,
}

/// `assertCouponUsable`.
fn assert_usable(code: &DiscountCode, now: i64) -> Result<()> {
    if !code.is_active {
        return Err(discount_error(
            "This discount code is turned off.",
            "discountCodeInactive",
        ));
    }
    if code.starts_at > now {
        return Err(discount_error(
            "This discount code is not valid yet.",
            "discountCodeNotStarted",
        )
        .with_data(json!({ "startsAt": iso(code.starts_at) })));
    }
    if code.ends_at.is_some_and(|end| end <= now) {
        return Err(discount_error(
            "This discount code has expired.",
            "discountCodeExpired",
        ));
    }
    if code
        .usage_limit
        .is_some_and(|limit| code.used_count >= limit)
    {
        return Err(used_up());
    }
    Ok(())
}

fn product_ids(code: &DiscountCode) -> Vec<i64> {
    code.product_ids
        .0
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.as_i64())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `computeCouponDiscount`.
pub fn compute(code: &DiscountCode, order: &OrderContext) -> Result<f64> {
    let currency = order.currency.as_str();
    let lines: Vec<&CouponLine> =
        if code.discount_type == "freeShipping" || code.applies_to == "entireOrder" {
            order.lines.iter().collect()
        } else if code.applies_to == "category" {
            match code.category_id {
                Some(category) => order
                    .lines
                    .iter()
                    .filter(|l| l.category_ids.contains(&category))
                    .collect(),
                None => Vec::new(),
            }
        } else {
            let ids = product_ids(code);
            order
                .lines
                .iter()
                .filter(|l| l.product_id.is_some_and(|p| ids.contains(&p)))
                .collect()
        };
    if lines.is_empty() {
        return Err(not_applicable());
    }
    let amount = sum_money(
        &lines.iter().map(|l| l.amount).collect::<Vec<_>>(),
        currency,
    );
    let quantity: i64 = lines.iter().map(|l| l.quantity).sum();
    let minimum = major(code.minimum_value);
    let js = crate::payments::js_number;
    if code.minimum_type == "amount" && amount < minimum {
        return Err(discount_error(
            format!("This discount code needs a purchase of at least {}.", js(minimum)),
            "discountCodeMinimumNotMet",
        )
        .with_data(json!({ "minimumType": "amount", "minimum": minimum, "missing": round_money(minimum - amount, currency) })));
    }
    if code.minimum_type == "quantity" && (quantity as f64) < minimum {
        return Err(discount_error(
            format!("This discount code needs at least {} items.", js(minimum)),
            "discountCodeMinimumNotMet",
        )
        .with_data(json!({ "minimumType": "quantity", "minimum": minimum, "missing": minimum - quantity as f64 })));
    }
    let value = major(code.discount_value);
    match code.discount_type.as_str() {
        "percentage" => Ok(percent_of(amount, value, currency)),
        "fixedAmount" => Ok(round_money(value.min(amount), currency)),
        "freeShipping"
            if order.shipping_type.as_deref() == Some("delivery") && order.shipping_total > 0.0 =>
        {
            Ok(order.shipping_total)
        }
        _ => Err(not_applicable()),
    }
}

/// `assertEligible`.
fn assert_eligible(conn: &Connection, code: &DiscountCode, order: &OrderContext) -> Result<()> {
    if code.eligibility != "specificCustomers" && !code.once_per_customer {
        return Ok(());
    }
    let not_eligible = || {
        discount_error(
            "This discount code is not available to this customer.",
            "discountCodeNotEligible",
        )
    };
    let Some(contact_id) = order.contact_id else {
        return Err(not_eligible());
    };
    if code.eligibility == "specificCustomers" {
        // ponytail: `withWorkingHoursTimezone` (working-hours conditions in
        // the workspace zone) is not applied here; they read as UTC.
        let criteria = code
            .customer_filter
            .as_ref()
            .map(|f| crate::contact_filter::to_filter_criteria(&f.0));
        let matches = match criteria {
            Some(c)
                if c.conditions.is_empty()
                    || contact_filter_has_predicate(&c, Some(order.workspace_id)) =>
            {
                let mut sql = Sql::raw(r#"SELECT 1 FROM "Contact" WHERE "Contact"."id" = "#);
                sql.push_bind(contact_id);
                sql.push(" AND ");
                sql.append(build_contact_where(&ContactWhereInput {
                    workspace_id: order.workspace_id,
                    keyword: None,
                    contact_filter: Some(c),
                    now: now_ms(),
                }));
                sql.push(" LIMIT 1");
                !sql.query_map(conn, |r| r.get::<_, i64>(0))?.is_empty()
            }
            _ => false,
        };
        if !matches {
            return Err(not_eligible());
        }
    }
    if code.once_per_customer {
        let used: i64 = conn.query_row(
            r#"SELECT COUNT(*) FROM "DiscountCodeRedemption" WHERE "discountCodeId" = ?1 AND "contactId" = ?2"#,
            params![code.id, contact_id],
            |r| r.get(0),
        )?;
        if used > 0 {
            return Err(discount_error(
                "This discount code can be used once per customer.",
                "discountCodeAlreadyUsed",
            ));
        }
    }
    Ok(())
}

/// `price`: the code re-read (the one writer is the lock), usable, eligible.
fn price(
    conn: &Connection,
    code: &DiscountCode,
    order: &OrderContext,
    now: i64,
) -> Result<Applied> {
    let current: DiscountCode = find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&code.id, &order.workspace_id],
    )?
    .ok_or_else(not_found)?;
    assert_usable(&current, now)?;
    assert_eligible(conn, &current, order)?;
    let amount = compute(&current, order)?;
    Ok(Applied {
        id: current.id,
        code: current.code.clone(),
        discount_type: current.discount_type.clone(),
        discount_value: major(current.discount_value),
        ends_at: current.ends_at,
        amount,
    })
}

/// `findByCode` (case-insensitive, the store's source).
pub fn find_by_code(
    conn: &Connection,
    workspace_id: i64,
    code: &str,
    source: &str,
) -> Result<Option<DiscountCode>> {
    Ok(find_one(
        conn,
        r#""workspaceId" = ?1 AND "source" = ?2 AND lower("code") = lower(?3)"#,
        &[&workspace_id, &source, &code.trim()],
    )?)
}

/// `resolve`: the typed code (refused when unusable), else the automatic
/// code that takes off the most.
pub fn resolve(
    conn: &Connection,
    order: &OrderContext,
    code: Option<&str>,
    auto_apply: bool,
    now: i64,
) -> Result<Option<Applied>> {
    if let Some(typed) = code.map(str::trim).filter(|c| !c.is_empty()) {
        let found =
            find_by_code(conn, order.workspace_id, typed, &order.source)?.ok_or_else(not_found)?;
        return price(conn, &found, order, now).map(Some);
    }
    if !auto_apply {
        return Ok(None);
    }
    let candidates: Vec<DiscountCode> = find_many(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "source" = ?2 AND "autoApply" = 1 AND "isActive" = 1
           AND "startsAt" <= ?3 AND ("endsAt" IS NULL OR "endsAt" > ?3)
           AND ("usageLimit" IS NULL OR "usedCount" < "usageLimit")
           ORDER BY "createdAt" ASC, "id" ASC"#,
        &[&order.workspace_id, &order.source, &now],
    )?;
    let mut best: Option<Applied> = None;
    for candidate in &candidates {
        match price(conn, candidate, order, now) {
            Ok(applied)
                if applied.amount > 0.0
                    && best.as_ref().is_none_or(|b| applied.amount > b.amount) =>
            {
                best = Some(applied);
            }
            Ok(_) | Err(BusinessError::Exception { .. }) => {}
            Err(other) => return Err(other),
        }
    }
    Ok(best)
}

/// `redeem`: counted against the limit, one redemption row.
pub fn redeem(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    contact_id: Option<i64>,
    applied: &Applied,
) -> Result<()> {
    let counted = conn.execute(
        r#"UPDATE "DiscountCode" SET "usedCount" = "usedCount" + 1, "updatedAt" = ?2
           WHERE "id" = ?1 AND ("usageLimit" IS NULL OR "usedCount" < "usageLimit")"#,
        params![applied.id, now_ms()],
    )?;
    if counted == 0 {
        return Err(used_up());
    }
    conn.execute(
        r#"INSERT INTO "DiscountCodeRedemption" ("id","workspaceId","discountCodeId","orderId","contactId","amount")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
        params![new_id(), workspace_id, applied.id, order_id, contact_id, (applied.amount * MONEY_SCALE).round() as i64],
    )?;
    Ok(())
}

/// `release`: an order's redemption goes, and the code's count with it.
pub fn release(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<()> {
    let code: Option<i64> = conn
        .query_row(
            r#"DELETE FROM "DiscountCodeRedemption" WHERE "workspaceId" = ?1 AND "orderId" = ?2 RETURNING "discountCodeId""#,
            params![workspace_id, order_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(code) = code {
        conn.execute(
            r#"UPDATE "DiscountCode" SET "usedCount" = max("usedCount" - 1, 0), "updatedAt" = ?2 WHERE "id" = ?1"#,
            params![code, now_ms()],
        )?;
    }
    Ok(())
}
