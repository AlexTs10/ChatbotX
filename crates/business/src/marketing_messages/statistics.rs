//! `getMarketingMessagesStatistics` (`lib/statistics-{operations,range}.ts`):
//! per-campaign Meta insights of one ad account, cached 10 minutes.

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_db::repo::facebook_marketing_message::{self as repo, StatisticsListInput};
use chatbotx_db::repo::utils::SortItem;
use chatbotx_kv::durable::DurableKv;
use chrono::{Months, NaiveDate, TimeDelta, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{graph, require_grant};
use crate::errors::{BusinessError, Result};
use crate::tx;

const CACHE_TTL_SECONDS: u64 = 600;
const MAX_RANGE_MONTHS: u32 = 37;

/// `StatisticsInput` (validated).
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub ad_account_id: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub search: Option<String>,
    pub page: i64,
    pub per_page: i64,
    pub sort: Option<String>,
    pub order_desc: bool,
}

fn invalid_range() -> BusinessError {
    BusinessError::exception(
        "Choose a start date on or before the end date, within the last 37 months.",
        "mmStatisticsInvalidRange",
        400,
    )
}

/// `parseDay` (`new Date(y, m - 1, d)`: an out-of-range day rolls over).
fn parse_day(day: &str) -> Option<NaiveDate> {
    let mut parts = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let months = y * 12 + (m - 1);
    let first = NaiveDate::from_ymd_opt(
        i32::try_from(months.div_euclid(12)).ok()?,
        u32::try_from(months.rem_euclid(12) + 1).ok()?,
        1,
    )?;
    first.checked_add_signed(TimeDelta::days(d - 1))
}

fn fmt_day(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// `isRangeWithinLimit`.
fn within_limit(since: &str, until: &str) -> bool {
    if since > until {
        return false;
    }
    let Some(earliest) =
        parse_day(until).and_then(|d| d.checked_sub_months(Months::new(MAX_RANGE_MONTHS)))
    else {
        return false;
    };
    since >= fmt_day(earliest).as_str()
}

/// `statisticsPresetRange("last30", timezone)`.
fn last30(timezone: &str) -> (String, String) {
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let today = Utc::now().with_timezone(&tz).date_naive();
    (fmt_day(today - TimeDelta::days(29)), fmt_day(today))
}

/// `toNullableNumber`.
fn metric(value: &Value) -> Value {
    let number = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if !s.is_empty() => Some(chatbotx_core::js::to_number(s)),
        _ => None,
    };
    number
        .filter(|n| n.is_finite())
        .map_or(Value::Null, |n| json!(n))
}

fn metrics(row: Option<&Value>) -> Map<String, Value> {
    graph::METRIC_FIELDS
        .iter()
        .map(|(key, field)| {
            (
                (*key).to_owned(),
                row.map_or(Value::Null, |r| metric(&r[*field])),
            )
        })
        .collect()
}

fn hash_ids(ids: &[String]) -> String {
    let mut sorted = ids.to_vec();
    sorted.sort();
    Sha256::digest(sorted.join(",").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `withCache`: a cached JSON value, else `load` (stored for 10 minutes).
async fn cached<F>(
    kv: &DurableKv,
    key: &str,
    load: F,
) -> std::result::Result<Value, graph::GraphError>
where
    F: std::future::Future<Output = std::result::Result<Value, graph::GraphError>>,
{
    if let Ok(Some(hit)) = kv.get::<Value>(key).await {
        return Ok(hit);
    }
    let value = load.await?;
    if let Err(error) = kv.put(key, &value, Some(CACHE_TTL_SECONDS)).await {
        tracing::warn!(%error, key, "Marketing Messages statistics cache write failed");
    }
    Ok(value)
}

/// `getMarketingMessagesStatistics`.
pub async fn get(db: &Arc<Db>, workspace_id: i64, input: Input) -> Result<Value> {
    if let (Some(since), Some(until)) = (&input.since, &input.until) {
        if !within_limit(since, until) {
            return Err(invalid_range());
        }
    }
    let accounts = tx::read(db, move |conn| {
        Ok(repo::list_ad_accounts_by_workspace_id(conn, workspace_id)?)
    })
    .await?;
    let selected = match &input.ad_account_id {
        Some(id) => Some(
            accounts
                .iter()
                .find(|a| &a.ad_account_id == id)
                .ok_or_else(|| {
                    BusinessError::exception(
                        "This workspace has no Marketing Messages campaigns on that ad account.",
                        "mmStatisticsUnknownAdAccount",
                        400,
                    )
                })?,
        ),
        None => accounts.first(),
    };
    let Some(selected) = selected else {
        let (since, until) = last30("UTC");
        return Ok(json!({
            "adAccountId": null,
            "adAccounts": [],
            "currency": null,
            "timezone": "UTC",
            "since": input.since.unwrap_or(since),
            "until": input.until.unwrap_or(until),
            "rows": [],
            "aggregate": null,
            "aggregateUnavailable": false,
            "pagination": { "page": input.page, "perPage": input.per_page, "total": 0, "pageCount": 0 },
        }));
    };
    let grant = require_grant(db, workspace_id).await?;
    let kv = DurableKv::new(db.clone(), Arc::new(chatbotx_kv::SystemClock));
    let meta = cached(&kv, &format!("mm-stats:accounts:{workspace_id}"), async {
        let accounts = graph::mm_ad_accounts(&grant.access_token, &grant.version).await?;
        Ok(Value::Array(
            accounts
                .iter()
                .map(|a| json!({ "id": a.id, "name": a.name, "timezoneName": a.timezone_name }))
                .collect(),
        ))
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(message = %error.message, workspace_id, "Marketing Messages statistics ad account lookup failed");
        Value::Array(Vec::new())
    });
    let meta_by_id: HashMap<&str, &Value> = meta
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["id"].as_str()?, a)))
        .collect();
    let ad_accounts: Vec<Value> = accounts
        .iter()
        .map(|a| json!({ "id": a.ad_account_id, "name": meta_by_id.get(a.ad_account_id.as_str()).map_or(Value::Null, |m| m["name"].clone()) }))
        .collect();
    let timezone = meta_by_id
        .get(selected.ad_account_id.as_str())
        .and_then(|m| m["timezoneName"].as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .or_else(|| selected.timezone_name.clone().filter(|t| !t.is_empty()))
        .unwrap_or_else(|| "UTC".to_owned());
    let (default_since, default_until) = last30(&timezone);
    let since = input.since.clone().unwrap_or(default_since);
    let until = input.until.clone().unwrap_or(default_until);
    if !within_limit(&since, &until) {
        return Err(invalid_range());
    }
    let list = StatisticsListInput {
        workspace_id,
        ad_account_id: selected.ad_account_id.clone(),
        search: input.search.clone(),
        page: Some(input.page),
        per_page: Some(input.per_page),
        sort: input.sort.clone().map(|id| {
            vec![SortItem {
                id,
                desc: input.order_desc,
            }]
        }),
    };
    let page = tx::read(db, move |conn| Ok(repo::list_for_statistics(conn, &list)?)).await?;
    let ad_ids: Vec<String> = page.data.iter().map(|r| r.campaign_id.clone()).collect();
    let mut rows_by_ad: HashMap<String, Value> = HashMap::new();
    let mut aggregate: Option<Value> = None;
    let mut aggregate_unavailable = false;
    let mut currency: Option<String> = None;
    if !ad_ids.is_empty() {
        let key_base = format!(
            "mm-stats:{workspace_id}:{}:{since}:{until}",
            selected.ad_account_id
        );
        let ids_hash = hash_ids(&ad_ids);
        let fetch = |level: &'static str| {
            let (grant, act, ids, since, until) =
                (&grant, &selected.ad_account_id, &ad_ids, &since, &until);
            async move {
                graph::insights(
                    &grant.access_token,
                    &grant.version,
                    act,
                    ids,
                    since,
                    until,
                    level,
                )
                .await
                .map(Value::Array)
            }
        };
        // ponytail: the two reads run one after another (`Promise.allSettled` in the original).
        let ad_rows = cached(&kv, &format!("{key_base}:ad:{ids_hash}"), fetch("ad")).await;
        let account_rows = cached(
            &kv,
            &format!("{key_base}:account:{ids_hash}"),
            fetch("account"),
        )
        .await;
        for row in ad_rows?.as_array().into_iter().flatten() {
            if let Some(id) = row["ad_id"].as_str() {
                if currency.is_none() {
                    currency = row["account_currency"].as_str().map(str::to_owned);
                }
                rows_by_ad.insert(id.to_owned(), row.clone());
            }
        }
        match account_rows {
            Ok(rows) => aggregate = rows.as_array().and_then(|r| r.first()).cloned(),
            Err(error) => {
                aggregate_unavailable = true;
                tracing::warn!(message = %error.message, workspace_id, "Marketing Messages statistics aggregate failed");
            }
        }
    }
    let currency = currency
        .or_else(|| {
            aggregate
                .as_ref()
                .and_then(|a| a["account_currency"].as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| selected.currency.clone());
    let rows: Vec<Value> = page
        .data
        .iter()
        .map(|row| {
            let mut out = metrics(rows_by_ad.get(&row.campaign_id));
            out.insert("id".into(), json!(row.id.to_string()));
            out.insert("campaignId".into(), json!(row.campaign_id));
            out.insert("name".into(), json!(row.name));
            out.insert("pageId".into(), json!(row.page_id));
            Value::Object(out)
        })
        .collect();
    Ok(json!({
        "adAccountId": selected.ad_account_id,
        "adAccounts": ad_accounts,
        "currency": currency,
        "timezone": timezone,
        "since": since,
        "until": until,
        "rows": rows,
        "aggregate": aggregate.as_ref().map_or(Value::Null, |a| Value::Object(metrics(Some(a)))),
        "aggregateUnavailable": aggregate_unavailable,
        "pagination": { "page": input.page, "perPage": input.per_page, "total": page.total, "pageCount": page.page_count },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_limit_and_day_rollover() {
        assert!(within_limit("2026-10-01", "2026-10-31"));
        assert!(!within_limit("2026-10-31", "2026-10-01"));
        assert!(!within_limit("2023-01-01", "2026-10-31"));
        assert!(within_limit("2023-09-30", "2026-10-31"));
        assert_eq!(
            parse_day("2026-02-31").map(fmt_day).as_deref(),
            Some("2026-03-03")
        );
        assert_eq!(metric(&json!("")), Value::Null);
        assert_eq!(metric(&json!("1.5")), json!(1.5));
    }
}
