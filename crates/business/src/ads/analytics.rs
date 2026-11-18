//! `ads-analytics/service.ts` (`getOverview`, `getTimeseries`) and
//! `merge.ts` (`mergeAdsAnalytics`): the DB funnel joined with the Meta
//! spend of the channel's connected ad accounts.

use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::DateRangeInput;
use serde_json::{Map, Value, json};

use super::graph::{self, ChannelAdAccount};
use crate::errors::Result;
use crate::pipeline::analytics::{day_keys, parse_analytics_date_range};

/// `AdsAnalyticsScope`.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub workspace_id: i64,
    pub from: String,
    pub to: String,
    pub tz: Option<String>,
    pub ad_account_id: Option<String>,
    pub channel: Option<String>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub all_channels: bool,
}

impl Scope {
    fn integration_id(&self) -> Option<i64> {
        self.integration_whatsapp_id
            .or(self.integration_messenger_id)
            .or(self.integration_instagram_id)
    }
}

/// `AD_ACCOUNT_ID_RE`.
fn is_ad_account_id(text: &str) -> bool {
    text.strip_prefix("act_")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

struct Insights {
    rows: Vec<Value>,
    ad_account_filter_applied: bool,
}

/// `list(Daily)InsightsForConnectedAdAccounts`: a failed account is logged
/// and skipped; no ads-eligible channel = no Meta read.
async fn insights(db: &Db, scope: &Scope, from: &str, to: &str, daily: bool) -> Insights {
    let empty = Insights {
        rows: Vec::new(),
        ad_account_filter_applied: false,
    };
    let Some(channel) = scope
        .channel
        .as_deref()
        .filter(|c| matches!(*c, "whatsapp" | "messenger" | "instagram"))
    else {
        return empty;
    };
    let ad_account_id = scope
        .ad_account_id
        .as_deref()
        .filter(|id| is_ad_account_id(id));
    let accounts =
        match graph::channel_ad_accounts(db, scope.workspace_id, channel, scope.integration_id())
            .await
        {
            Ok(accounts) => accounts,
            Err(error) => {
                tracing::warn!(?error, "Failed to load ad account list for Ads analytics");
                return empty;
            }
        };
    // `filterAdAccountsByIds`.
    let selected: Vec<&ChannelAdAccount> = match ad_account_id {
        Some(id) => accounts
            .iter()
            .filter(|a| a.account["id"] == json!(id))
            .collect(),
        None => accounts.iter().collect(),
    };
    let ad_account_filter_applied = ad_account_id.is_some() && !selected.is_empty();
    let mut rows = Vec::new();
    for account in selected {
        let id = account.account["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let kind = if daily {
            "insights-daily:v2"
        } else {
            "insights:v3"
        };
        let key = format!("fb-ads:{kind}:{}:{id}:{from}:{to}", scope.workspace_id);
        let source = account.sources.first();
        let result = graph::cached(key, graph::INSIGHTS_TTL, || async {
            let auth = graph::source_auth(db, scope.workspace_id, channel, source).await?;
            graph::insights(&auth, &id, from, to, daily)
                .await
                .map_err(crate::errors::BusinessError::from)
        })
        .await;
        match result {
            Ok(Value::Array(list)) => rows.extend(list),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                ?error,
                ad_account_id = id,
                "Failed to load Facebook Ads insights for CTWA analytics"
            ),
        }
    }
    Insights {
        rows,
        ad_account_filter_applied,
    }
}

fn funnel_input(scope: &Scope, since: i64, until: i64, timezone: Option<String>) -> DateRangeInput {
    DateRangeInput {
        workspace_id: scope.workspace_id,
        since,
        until,
        integration_whatsapp_id: scope.integration_whatsapp_id,
        channel: scope.channel.clone(),
        integration_messenger_id: scope.integration_messenger_id,
        integration_instagram_id: scope.integration_instagram_id,
        all_channels: scope.all_channels,
        timezone,
    }
}

fn divide_cost(spend: Option<f64>, count: f64) -> Value {
    match spend {
        Some(spend) if count > 0.0 => json!(spend / count),
        _ => Value::Null,
    }
}

fn divide_roas(revenue: f64, spend: Option<f64>) -> Value {
    match spend {
        Some(spend) if spend > 0.0 => json!(revenue / spend),
        _ => Value::Null,
    }
}

fn divide_rate(numerator: f64, denominator: f64) -> Value {
    if denominator > 0.0 {
        json!(numerator / denominator)
    } else {
        Value::Null
    }
}

fn divide_cpm(spend: Option<f64>, impressions: f64) -> Value {
    match divide_cost(spend, impressions).as_f64() {
        Some(rate) => json!(rate * 1000.0),
        None => Value::Null,
    }
}

fn num(row: &Map<String, Value>, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn opt(row: &Map<String, Value>, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

/// The ratio fields of one row / the totals.
fn ratios(
    out: &mut Map<String, Value>,
    spend: Option<f64>,
    f: &Map<String, Value>,
    clicks: f64,
    impressions: f64,
) {
    out.insert("costPerLead".into(), divide_cost(spend, num(f, "leads")));
    out.insert(
        "costPerPurchase".into(),
        divide_cost(spend, num(f, "purchases")),
    );
    out.insert("roas".into(), divide_roas(num(f, "revenue"), spend));
    out.insert("cpc".into(), divide_cost(spend, clicks));
    out.insert("ctr".into(), divide_rate(clicks, impressions));
    out.insert("cpm".into(), divide_cpm(spend, impressions));
    out.insert(
        "costPerConversation".into(),
        divide_cost(spend, num(f, "conversations")),
    );
}

/// `mergeAdsAnalytics`.
fn merge(
    funnel: &Value,
    insights: &[Value],
    integration_filter: bool,
    ad_account_filter: bool,
) -> Value {
    let mut rows: Vec<(String, Map<String, Value>)> = Vec::new();
    let key_of = |ad_id: &Value| ad_id.as_str().unwrap_or("__unattributed__").to_owned();
    let empty = |ad_id: Value| {
        let mut row = Map::new();
        row.insert("adId".into(), ad_id);
        row.insert("adName".into(), Value::Null);
        for field in ["conversations", "leads", "purchases", "revenue"] {
            row.insert(field.into(), json!(0));
        }
        for field in ["spend", "impressions", "clicks"] {
            row.insert(field.into(), Value::Null);
        }
        row
    };
    for f in funnel["perAd"].as_array().into_iter().flatten() {
        let mut row = empty(f["adId"].clone());
        for field in [
            "adName",
            "conversations",
            "leads",
            "purchases",
            "revenue",
            "channels",
        ] {
            if let Some(value) = f.get(field) {
                row.insert(field.into(), value.clone());
            }
        }
        if row["adName"].is_null() {
            row.insert("adName".into(), Value::Null);
        }
        rows.push((key_of(&f["adId"]), row));
    }
    for insight in insights {
        let key = key_of(&insight["ad_id"]);
        let index = match rows.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                rows.push((key, empty(insight["ad_id"].clone())));
                rows.len() - 1
            }
        };
        let row = &mut rows[index].1;
        if let Some(name) = insight["ad_name"].as_str() {
            row.insert("adName".into(), json!(name));
        }
        for (field, source) in [
            ("spend", "spend"),
            ("impressions", "impressions"),
            ("clicks", "clicks"),
        ] {
            let total =
                opt(row, field).unwrap_or_default() + insight[source].as_f64().unwrap_or_default();
            row.insert(field.into(), json!(total));
        }
    }
    let mut per_ad: Vec<Map<String, Value>> = rows
        .into_iter()
        .map(|(_, row)| row)
        .filter(|row| {
            (!integration_filter
                || num(row, "conversations") > 0.0
                || num(row, "leads") > 0.0
                || num(row, "purchases") > 0.0)
                && (!ad_account_filter || !row["spend"].is_null())
        })
        .map(|mut row| {
            let (spend, clicks, impressions) = (
                opt(&row, "spend"),
                num(&row, "clicks"),
                num(&row, "impressions"),
            );
            let snapshot = row.clone();
            ratios(&mut row, spend, &snapshot, clicks, impressions);
            row
        })
        .collect();
    let spend: f64 = per_ad.iter().map(|r| num(r, "spend")).sum();
    let impressions: f64 = per_ad.iter().map(|r| num(r, "impressions")).sum();
    let clicks: f64 = per_ad.iter().map(|r| num(r, "clicks")).sum();
    let mut totals = Map::new();
    for field in ["conversations", "leads", "purchases", "revenue"] {
        let value = if ad_account_filter {
            json!(per_ad.iter().map(|r| num(r, field)).sum::<f64>())
        } else {
            funnel["totals"][field].clone()
        };
        totals.insert(field.into(), value);
    }
    let snapshot = totals.clone();
    totals.insert("spend".into(), json!(spend));
    totals.insert("impressions".into(), json!(impressions));
    totals.insert("clicks".into(), json!(clicks));
    ratios(&mut totals, Some(spend), &snapshot, clicks, impressions);
    per_ad.sort_by(|a, b| num(b, "spend").total_cmp(&num(a, "spend")));
    let unknown_currency = insights
        .iter()
        .any(|i| i["account_currency"].as_str().is_none_or(str::is_empty));
    let mut currencies: Vec<&str> = insights
        .iter()
        .filter_map(|i| i["account_currency"].as_str())
        .collect();
    currencies.sort_unstable();
    currencies.dedup();
    let spend_currency = (!unknown_currency && currencies.len() == 1).then(|| currencies[0]);
    json!({ "totals": totals, "perAd": per_ad, "spendCurrency": spend_currency })
}

/// `getOverview`.
pub async fn overview(db: &Db, scope: Scope) -> Result<Value> {
    let range = parse_analytics_date_range(
        &scope.from,
        &scope.to,
        scope.tz.as_deref().unwrap_or_default(),
        chatbotx_db::repo::now_ms(),
    );
    let funnel =
        super::funnel::funnel(db, funnel_input(&scope, range.since, range.until, None)).await?;
    let found = insights(db, &scope, &range.from, &range.to, false).await;
    Ok(merge(
        &funnel,
        &found.rows,
        scope.integration_id().is_some(),
        found.ad_account_filter_applied,
    ))
}

/// `getTimeseries`: one row per day of the range.
pub async fn timeseries(db: &Db, scope: Scope) -> Result<Vec<Value>> {
    let range = parse_analytics_date_range(
        &scope.from,
        &scope.to,
        scope.tz.as_deref().unwrap_or_default(),
        chatbotx_db::repo::now_ms(),
    );
    let funnel_rows = super::funnel::funnel_timeseries(
        db,
        funnel_input(
            &scope,
            range.since,
            range.until,
            Some(range.timezone.clone()),
        ),
    )
    .await?;
    let found = insights(db, &scope, &range.from, &range.to, true).await;
    let surviving: Option<Vec<&str>> = found.ad_account_filter_applied.then(|| {
        found
            .rows
            .iter()
            .filter_map(|r| r["ad_id"].as_str())
            .collect()
    });
    let mut days: Vec<Value> = day_keys(&range.from, &range.to)
        .into_iter()
        .map(|date| json!({ "date": date, "conversations": 0, "leads": 0, "purchases": 0, "spend": null }))
        .collect();
    for row in &funnel_rows {
        if let Some(ids) = &surviving
            && !row["adId"].as_str().is_some_and(|id| ids.contains(&id))
        {
            continue;
        }
        if let Some(day) = days.iter_mut().find(|d| d["date"] == row["date"]) {
            for field in ["conversations", "leads", "purchases"] {
                day[field] = json!(
                    day[field].as_i64().unwrap_or_default()
                        + row[field].as_i64().unwrap_or_default()
                );
            }
        }
    }
    for row in &found.rows {
        let Some(date) = row["date_start"].as_str() else {
            continue;
        };
        if let Some(day) = days.iter_mut().find(|d| d["date"] == json!(date)) {
            day["spend"] = json!(
                day["spend"].as_f64().unwrap_or_default()
                    + row["spend"].as_f64().unwrap_or_default()
            );
        }
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_spend_into_the_funnel() {
        let funnel = json!({
            "totals": { "conversations": 2, "leads": 1, "purchases": 1, "revenue": 30 },
            "perAd": [{ "adId": "1", "adName": null, "conversations": 2, "leads": 1, "purchases": 1, "revenue": 30 }],
        });
        let insights = [
            json!({ "ad_id": "1", "ad_name": "One", "account_currency": "USD", "spend": 20.0, "impressions": 1000.0, "clicks": 50.0 }),
            json!({ "ad_id": "7", "ad_name": "Spend", "account_currency": "USD", "spend": 5.0, "impressions": 100.0, "clicks": 0.0 }),
        ];
        let out = merge(&funnel, &insights, false, false);
        assert_eq!(out["totals"]["spend"], json!(25.0));
        assert_eq!(out["totals"]["roas"], json!(30.0 / 25.0));
        assert_eq!(out["perAd"][1]["cpc"], Value::Null);
        assert_eq!(out["perAd"][1]["roas"], json!(0.0));
        assert_eq!(out["spendCurrency"], json!("USD"));
        assert!(is_ad_account_id("act_1") && !is_ad_account_id("123"));
    }
}
