//! Port of `repositories/order/analytics.ts` (`orderAnalyticsRepository`; no TS tests,
//! smoke test `tests/repo_p33_smoke.rs`). Sales sums are money x 10,000 integers.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::Result;
use super::queries::date_bucket::{Granularity, zoned_bucket_key, zoned_hour_key};
use super::sql::Sql;

/// `SalesScope` (`since` / `until` inclusive, unix ms).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SalesScope {
    pub workspace_id: i64,
    pub currency: String,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

/// Result of `summary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SalesSummary {
    pub sales: i64,
    pub orders: i64,
    pub products: i64,
}

/// One point of `series`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesPoint {
    pub key: String,
    pub sales: i64,
    pub orders: i64,
}

/// One row of `topProducts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopProduct {
    pub product_id: Option<i64>,
    pub name: String,
    pub quantity: i64,
    pub sales: i64,
}

/// One row of `topVendors`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopVendor {
    pub vendor_id: Option<i64>,
    pub name: String,
    pub sales: i64,
}

/// The bucket of `series`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesGranularity {
    Hour,
    Day,
}

/// `countedOrders`: not cancelled orders of the scope (`same_currency` = false counts the
/// other currencies).
fn counted_orders(scope: &SalesScope, same_currency: bool) -> Sql {
    let op = if same_currency { "=" } else { "<>" };
    let mut sql = Sql::new(
        format!("o.\"workspaceId\" = ? AND o.\"status\" <> 'cancelled' AND o.\"currency\" {op} ?"),
        vec![
            Value::Integer(scope.workspace_id),
            Value::Text(scope.currency.clone()),
        ],
    );
    if let Some(since) = scope.since {
        sql.push(" AND o.\"createdAt\" >= ");
        sql.push_bind(since);
    }
    if let Some(until) = scope.until {
        sql.push(" AND o.\"createdAt\" <= ");
        sql.push_bind(until);
    }
    sql
}

fn scalar(conn: &Connection, sql: Sql) -> Result<i64> {
    Ok(sql
        .query_map(conn, |row| row.get::<_, Option<i64>>(0))?
        .into_iter()
        .next()
        .flatten()
        .unwrap_or(0))
}

/// `countOtherCurrencyOrders`.
pub fn count_other_currency_orders(conn: &Connection, scope: &SalesScope) -> Result<i64> {
    let mut sql = Sql::raw("SELECT count(*) FROM \"Order\" o WHERE ");
    sql.append(counted_orders(scope, false));
    scalar(conn, sql)
}

/// `summary`: sales, order count and product quantity of the scope.
pub fn summary(conn: &Connection, scope: &SalesScope) -> Result<SalesSummary> {
    let mut orders = Sql::raw("SELECT sum(o.\"total\"), count(*) FROM \"Order\" o WHERE ");
    orders.append(counted_orders(scope, true));
    let (sales, count) = orders
        .query_map(conn, |row| {
            Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?))
        })?
        .into_iter()
        .next()
        .unwrap_or((None, 0));
    let mut items = Sql::raw(
        "SELECT sum(i.\"quantity\") FROM \"OrderItem\" i JOIN \"Order\" o ON o.\"id\" = i.\"orderId\" WHERE ",
    );
    items.append(counted_orders(scope, true));
    Ok(SalesSummary {
        sales: sales.unwrap_or(0),
        orders: count,
        products: scalar(conn, items)?,
    })
}

/// `series`: sales and order count per local hour or day, in key order.
pub fn series(
    conn: &Connection,
    scope: &SalesScope,
    timezone: &str,
    granularity: SeriesGranularity,
) -> Result<Vec<SalesPoint>> {
    let key = match granularity {
        SeriesGranularity::Hour => zoned_hour_key("o.\"createdAt\"", timezone),
        SeriesGranularity::Day => zoned_bucket_key("o.\"createdAt\"", Granularity::Day, timezone),
    };
    let mut sql = Sql::raw("SELECT ");
    sql.append(key);
    sql.push(", sum(o.\"total\"), count(*) FROM \"Order\" o WHERE ");
    sql.append(counted_orders(scope, true));
    sql.push(" GROUP BY 1 ORDER BY 1");
    sql.query_map(conn, |row| {
        Ok(SalesPoint {
            key: row.get(0)?,
            sales: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            orders: row.get(2)?,
        })
    })
}

/// `topProducts`: best-selling products (by line total) of the scope.
pub fn top_products(conn: &Connection, scope: &SalesScope, limit: i64) -> Result<Vec<TopProduct>> {
    let mut sql = Sql::raw(
        "SELECT i.\"productId\", max(i.\"name\"), sum(i.\"quantity\"), sum(i.\"lineTotal\") AS sales FROM \"OrderItem\" i JOIN \"Order\" o ON o.\"id\" = i.\"orderId\" WHERE ",
    );
    sql.append(counted_orders(scope, true));
    sql.push(" GROUP BY i.\"productId\", CASE WHEN i.\"productId\" IS NULL THEN i.\"name\" END ORDER BY sales DESC LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| {
        Ok(TopProduct {
            product_id: row.get(0)?,
            name: row.get(1)?,
            quantity: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            sales: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
        })
    })
}

/// `topVendors`: best-selling vendors (by line total) of the scope.
pub fn top_vendors(conn: &Connection, scope: &SalesScope, limit: i64) -> Result<Vec<TopVendor>> {
    let mut sql = Sql::raw(
        "SELECT i.\"vendorId\", max(i.\"vendorName\"), sum(i.\"lineTotal\") AS sales FROM \"OrderItem\" i JOIN \"Order\" o ON o.\"id\" = i.\"orderId\" WHERE ",
    );
    sql.append(counted_orders(scope, true));
    sql.push(" AND i.\"vendorName\" IS NOT NULL GROUP BY i.\"vendorId\", CASE WHEN i.\"vendorId\" IS NULL THEN i.\"vendorName\" END ORDER BY sales DESC LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| {
        Ok(TopVendor {
            vendor_id: row.get(0)?,
            name: row.get(1)?,
            sales: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
        })
    })
}
