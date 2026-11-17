//! `ecommerce-analytics/service.ts`: the store's sales dashboard. Cancelled
//! orders never count; amounts are in the store currency (other currencies
//! are only counted as `otherCurrencyOrders`), days in the workspace timezone.

use chatbotx_core::utils::money::round_money;
use chatbotx_db::Db;
use chatbotx_db::repo::order_analytics::{self as repo, SalesScope, SeriesGranularity};
use serde_json::{Value, json};

use crate::ecommerce_settings;
use crate::errors::Result;
use crate::pipeline::analytics::{parse_analytics_date_range, resolve_timezone};
use crate::pipeline::query::workspace_timezone;
use crate::tx;

const TOP_LIMIT: i64 = 10;
const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

fn major(scaled: i64) -> f64 {
    scaled as f64 / 10_000.0
}

/// `percentChange`: one decimal; null when the previous period had nothing.
pub fn percent_change(current: f64, previous: f64) -> Option<f64> {
    if previous == 0.0 {
        return (current == 0.0).then_some(0.0);
    }
    Some((((current - previous) / previous) * 1000.0).round() / 10.0)
}

/// `rangeBucketKeys`: every hour or day of the range.
pub fn range_bucket_keys(from: &str, to: &str, hourly: bool) -> Vec<String> {
    if hourly {
        return (0..24).map(|hour| format!("{from}T{hour:02}")).collect();
    }
    let parse = |day: &str| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok();
    let (Some(mut day), Some(end)) = (parse(from), parse(to)) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    while day <= end {
        keys.push(day.format("%Y-%m-%d").to_string());
        day += chrono::Duration::milliseconds(MS_PER_DAY);
    }
    keys
}

/// `getDashboard`: `from` / `to` are `YYYY-MM-DD` workspace days, both today by default.
pub async fn get_dashboard(
    db: &Db,
    workspace_id: i64,
    from: Option<String>,
    to: Option<String>,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let settings = ecommerce_settings::get_in(conn, workspace_id)?;
        let tz = workspace_timezone(conn, workspace_id)?.unwrap_or_default();
        let now = chatbotx_db::repo::now_ms();
        let (zone, _) = resolve_timezone(&tz);
        let today = chrono::DateTime::from_timestamp_millis(now)
            .unwrap_or_default()
            .with_timezone(&zone)
            .format("%Y-%m-%d")
            .to_string();
        let from_key = from.clone().unwrap_or_else(|| today.clone());
        let to_key = to.or(from).unwrap_or(today);
        let range = parse_analytics_date_range(&from_key, &to_key, &tz, now);
        let currency = settings["payments"]["currency"].as_str().unwrap_or("EUR").to_string();
        let scope = SalesScope {
            workspace_id,
            currency: currency.clone(),
            since: Some(range.since),
            until: Some(range.until),
        };
        let length = range.until - range.since + 1;
        let previous_scope = SalesScope {
            since: Some(range.since - length),
            until: Some(range.since - 1),
            ..scope.clone()
        };
        let hourly = range.from == range.to;
        let all_time = SalesScope {
            since: None,
            until: None,
            ..scope.clone()
        };
        let lifetime = repo::summary(conn, &all_time)?;
        let until_end = repo::summary(
            conn,
            &SalesScope {
                until: Some(range.until),
                ..all_time.clone()
            },
        )?;
        let current = repo::summary(conn, &scope)?;
        let previous = repo::summary(conn, &previous_scope)?;
        let granularity = if hourly { SeriesGranularity::Hour } else { SeriesGranularity::Day };
        let series = repo::series(conn, &scope, &range.timezone, granularity)?;
        let top_products = repo::top_products(conn, &scope, TOP_LIMIT)?;
        let top_vendors = repo::top_vendors(conn, &scope, TOP_LIMIT)?;
        let other_currency_orders = repo::count_other_currency_orders(conn, &scope)?;

        let money = |scaled: i64| round_money(major(scaled), &currency);
        let stat = |value: Value, change: Option<f64>| json!({ "value": value, "change": change });
        let sales = |s: i64| major(s);
        Ok(json!({
            "currency": currency,
            "otherCurrencyOrders": other_currency_orders,
            "from": range.from,
            "to": range.to,
            "timezone": range.timezone,
            "granularity": if hourly { "hour" } else { "day" },
            "lifetime": stat(json!(money(lifetime.sales)), percent_change(sales(until_end.sales), sales(until_end.sales - current.sales))),
            "sales": stat(json!(money(current.sales)), percent_change(sales(current.sales), sales(previous.sales))),
            "orders": stat(json!(current.orders), percent_change(current.orders as f64, previous.orders as f64)),
            "products": stat(json!(current.products), percent_change(current.products as f64, previous.products as f64)),
            "series": range_bucket_keys(&range.from, &range.to, hourly).into_iter().map(|key| {
                let point = series.iter().find(|p| p.key == key);
                json!({
                    "key": key,
                    "sales": money(point.map_or(0, |p| p.sales)),
                    "orders": point.map_or(0, |p| p.orders),
                })
            }).collect::<Vec<_>>(),
            "topProducts": top_products.iter().map(|row| json!({
                "productId": row.product_id.map(|id| id.to_string()),
                "name": row.name,
                "quantity": row.quantity,
                "sales": money(row.sales),
            })).collect::<Vec<_>>(),
            "topVendors": top_vendors.iter().map(|row| json!({
                "vendorId": row.vendor_id.map(|id| id.to_string()),
                "name": row.name,
                "sales": money(row.sales),
            })).collect::<Vec<_>>(),
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_change_and_buckets() {
        assert_eq!(percent_change(0.0, 0.0), Some(0.0));
        assert_eq!(percent_change(5.0, 0.0), None);
        assert_eq!(percent_change(15.0, 10.0), Some(50.0));
        assert_eq!(percent_change(1.0, 3.0), Some(-66.7));
        assert_eq!(
            range_bucket_keys("2026-01-01", "2026-01-01", true).len(),
            24
        );
        assert_eq!(
            range_bucket_keys("2026-01-01", "2026-01-01", true)[9],
            "2026-01-01T09"
        );
        assert_eq!(
            range_bucket_keys("2026-02-27", "2026-03-01", false),
            ["2026-02-27", "2026-02-28", "2026-03-01"]
        );
    }
}
