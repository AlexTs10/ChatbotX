//! `contact-referral-event/service.ts` `getAdStats` and `listRecentAds`.

use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::{self as repo, AdStatsRange, ListRecentAdsInput};
use serde_json::{Value, json};

use crate::errors::Result;
use crate::pipeline::analytics::parse_analytics_date_range;
use crate::tx;

/// `getAdStats`: `from` / `to` default to the last 7 days in `tz`.
pub async fn ad_stats(
    db: &Db,
    workspace_id: i64,
    from: Option<String>,
    to: Option<String>,
    tz: Option<String>,
    channel: Option<String>,
    limit: i64,
) -> Result<Vec<Value>> {
    let range = parse_analytics_date_range(
        from.as_deref().unwrap_or_default(),
        to.as_deref().unwrap_or_default(),
        tz.as_deref().unwrap_or_default(),
        chatbotx_db::repo::now_ms(),
    );
    let range = AdStatsRange {
        workspace_id,
        since: range.since,
        until: range.until,
        channel,
    };
    tx::read(db, move |conn| {
        let touches = repo::aggregate_ad_touch_stats(conn, &range, limit)?;
        let ad_ids: Vec<String> = touches.iter().map(|t| t.ad_id.clone()).collect();
        let conversions = repo::aggregate_meta_linked_conversions(conn, &range, &ad_ids)?;
        Ok(touches
            .into_iter()
            .map(|row| {
                let (mut leads, mut purchases, mut revenue) = (0, 0, Vec::new());
                for item in conversions.iter().filter(|c| c.ad_id == row.ad_id) {
                    if item.event_type == "lead" {
                        leads += item.count;
                        continue;
                    }
                    purchases += item.count;
                    if item.amount != "0" {
                        revenue.push(json!({ "currency": item.currency, "amount": item.amount }));
                    }
                }
                json!({
                    "adId": row.ad_id,
                    "title": row.title,
                    "anyTouchContacts": row.any_touch_contacts,
                    "firstTouchContacts": row.first_touch_contacts,
                    "leads": leads,
                    "purchases": purchases,
                    "revenue": revenue,
                })
            })
            .collect())
    })
    .await
}

/// `listRecentAds` as `{ id, title }`.
pub async fn recent_ads(
    db: &Db,
    workspace_id: i64,
    search: Option<String>,
    limit: i64,
) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let ads = repo::list_recent_ads(
            conn,
            &ListRecentAdsInput {
                workspace_id,
                limit,
                search,
                ad_ids: None,
            },
        )?;
        Ok(ads
            .into_iter()
            .map(|ad| json!({ "id": ad.ad_id, "title": ad.title }))
            .collect())
    })
    .await
}
