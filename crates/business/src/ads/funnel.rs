//! `ads-conversion/service.ts` reports: `getCtwaFunnel`,
//! `getCtwaFunnelTimeseries`, `getCapiDeliverySummary`, `listExportRows` /
//! `listAllChannelExportRows`, `findWorkspaceEventOrFail`.

use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::{
    self as repo, ConversationCount, DEFAULT_ADS_CONVERSION_CHANNEL, DateRangeInput,
    ExportSegmentInput,
};
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn is_ad_referral(channel: Option<&str>) -> bool {
    matches!(channel, Some("messenger" | "instagram"))
}

/// `countConversationsByAd` / `...ByDayAndAd`: none for `facebook`, the ad
/// referral count for Messenger / Instagram, the CTWA count otherwise.
fn conversations(
    conn: &Connection,
    input: &DateRangeInput,
    by_day: bool,
) -> Result<Vec<ConversationCount>> {
    let channel = input.channel.as_deref();
    if channel == Some("facebook") {
        return Ok(Vec::new());
    }
    Ok(
        match (input.all_channels, is_ad_referral(channel), by_day) {
            (true, _, false) => repo::count_all_channel_conversations_by_ad(conn, input)?,
            (true, _, true) => repo::count_all_channel_conversations_by_day_and_ad(conn, input)?,
            (false, true, false) => repo::count_ad_conversations_by_ad(conn, input)?,
            (false, true, true) => repo::count_ad_conversations_by_day_and_ad(conn, input)?,
            (false, false, false) => repo::count_ctwa_conversations_by_ad(conn, input)?,
            (false, false, true) => repo::count_ctwa_conversations_by_day_and_ad(conn, input)?,
        },
    )
}

/// Rows keyed by `key`, in first-seen order.
fn row_for(
    rows: &mut Vec<(String, Map<String, Value>)>,
    key: String,
    init: impl FnOnce() -> Map<String, Value>,
) -> &mut Map<String, Value> {
    let index = match rows.iter().position(|(k, _)| *k == key) {
        Some(index) => index,
        None => {
            rows.push((key, init()));
            rows.len() - 1
        }
    };
    &mut rows[index].1
}

fn add(row: &mut Map<String, Value>, field: &str, amount: f64) {
    let current = row[field].as_f64().unwrap_or_default();
    row.insert(field.to_owned(), number(current + amount));
}

/// An integral count as a JSON integer, else a float.
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn add_channel(row: &mut Map<String, Value>, channel: Option<&str>) {
    if let (Some(channel), Some(Value::Array(list))) = (channel, row.get_mut("channels"))
        && !list.iter().any(|c| c == channel)
    {
        list.push(json!(channel));
    }
}

/// `getCtwaFunnel`.
pub async fn funnel(db: &Db, input: DateRangeInput) -> Result<Value> {
    tx::read(db, move |conn| {
        let conversation_rows = conversations(conn, &input, false)?;
        let event_rows = repo::count_conversion_events_by_ad(conn, &input)?;
        let mut rows: Vec<(String, Map<String, Value>)> = Vec::new();
        let all = input.all_channels;
        let init = |ad_id: &Option<String>| {
            let ad_id = ad_id.clone();
            move || {
                let mut row = Map::new();
                row.insert("adId".into(), json!(ad_id));
                row.insert("adName".into(), Value::Null);
                for field in ["conversations", "leads", "purchases", "revenue"] {
                    row.insert(field.into(), json!(0));
                }
                if all {
                    row.insert("channels".into(), json!([]));
                }
                row
            }
        };
        let key =
            |ad_id: &Option<String>| ad_id.clone().unwrap_or_else(|| "__unattributed__".into());
        for c in &conversation_rows {
            let row = row_for(&mut rows, key(&c.ad_id), init(&c.ad_id));
            add(row, "conversations", c.conversations as f64);
            add_channel(row, c.channel.as_deref());
        }
        for e in &event_rows {
            let row = row_for(&mut rows, key(&e.ad_id), init(&e.ad_id));
            add_channel(row, e.channel.as_deref());
            if e.event_type == "lead" {
                add(row, "leads", e.count as f64);
            } else {
                add(row, "purchases", e.count as f64);
                let revenue = e
                    .purchase_value
                    .as_deref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .unwrap_or_default();
                add(row, "revenue", revenue);
            }
        }
        let per_ad: Vec<Value> = rows
            .into_iter()
            .map(|(_, row)| Value::Object(row))
            .collect();
        let total = |field: &str| {
            number(
                per_ad
                    .iter()
                    .map(|r| r[field].as_f64().unwrap_or_default())
                    .sum(),
            )
        };
        Ok(json!({
            "totals": {
                "conversations": total("conversations"),
                "leads": total("leads"),
                "purchases": total("purchases"),
                "revenue": total("revenue"),
            },
            "perAd": per_ad,
        }))
    })
    .await
}

/// `getCtwaFunnelTimeseries`.
pub async fn funnel_timeseries(db: &Db, input: DateRangeInput) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let conversation_rows = conversations(conn, &input, true)?;
        let event_rows = repo::count_conversion_events_by_day_and_ad(conn, &input)?;
        let mut rows: Vec<(String, Map<String, Value>)> = Vec::new();
        let init = |date: &Option<String>, ad_id: &Option<String>| {
            let (date, ad_id) = (date.clone(), ad_id.clone());
            move || {
                let mut row = Map::new();
                row.insert("date".into(), json!(date.unwrap_or_default()));
                row.insert("adId".into(), json!(ad_id));
                for field in ["conversations", "leads", "purchases"] {
                    row.insert(field.into(), json!(0));
                }
                row
            }
        };
        let key = |date: &Option<String>, ad_id: &Option<String>| {
            format!(
                "{}:{}",
                date.as_deref().unwrap_or_default(),
                ad_id.as_deref().unwrap_or("__unattributed__")
            )
        };
        for c in &conversation_rows {
            let row = row_for(&mut rows, key(&c.date, &c.ad_id), init(&c.date, &c.ad_id));
            add(row, "conversations", c.conversations as f64);
        }
        for e in &event_rows {
            let row = row_for(&mut rows, key(&e.date, &e.ad_id), init(&e.date, &e.ad_id));
            let field = if e.event_type == "lead" {
                "leads"
            } else {
                "purchases"
            };
            add(row, field, e.count as f64);
        }
        Ok(rows
            .into_iter()
            .map(|(_, row)| Value::Object(row))
            .collect())
    })
    .await
}

/// `getCapiDeliverySummary`.
pub async fn capi_delivery(db: &Db, input: DateRangeInput) -> Result<Value> {
    tx::read(db, move |conn| {
        let mut summary = json!({ "sent": 0, "pending": 0, "failed": 0, "skippedNoScope": 0, "skippedRegion": 0 });
        for row in repo::count_by_capi_status(conn, &input)? {
            let key = match row.capi_status.as_str() {
                "skipped_no_scope" => "skippedNoScope",
                "skipped_region" => "skippedRegion",
                other => other,
            };
            summary[key] = json!(row.count);
        }
        Ok(summary)
    })
    .await
}

/// `listExportRows` (an unscoped request defaults to WhatsApp) or, with
/// `allChannels`, `listAllChannelExportRows`; `{ data, nextAfterId }`.
pub async fn export_rows(db: &Db, mut input: ExportSegmentInput) -> Result<Value> {
    let r = &mut input.range;
    if !r.all_channels
        && r.channel.is_none()
        && r.integration_whatsapp_id.is_none()
        && r.integration_messenger_id.is_none()
        && r.integration_instagram_id.is_none()
    {
        r.channel = Some(DEFAULT_ADS_CONVERSION_CHANNEL.to_owned());
    }
    let all = input.range.all_channels;
    tx::read(db, move |conn| {
        let page = repo::list_export_segment_rows(conn, &input)?;
        let data: Vec<Value> = page
            .rows
            .iter()
            .map(|row| {
                let mut out = json!({
                    "id": row.id.to_string(),
                    "contactId": row.contact_id.to_string(),
                    "contactName": row.contact_name,
                    "phoneNumber": row.phone_number,
                    "email": row.email,
                    "adId": row.ad_id,
                    "occurredAt": iso(row.occurred_at),
                });
                if all {
                    out["channel"] = json!(row.channel);
                }
                out
            })
            .collect();
        let next = page
            .has_more
            .then(|| page.rows.last().map(|r| r.id.to_string()))
            .flatten();
        Ok(json!({ "data": data, "nextAfterId": next }))
    })
    .await
}

fn id_text(id: Option<i64>) -> Value {
    id.map_or(Value::Null, |id| json!(id.to_string()))
}

/// `findWorkspaceEventOrFail` as `adsConversionEventPublicResource`
/// (no `workspaceId` / `referralEventId`).
pub async fn find_event(db: &Db, workspace_id: i64, id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let e = repo::find_workspace_event(conn, id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Ads conversion event not found"))?;
        Ok(json!({
            "id": e.id.to_string(),
            "createdAt": iso(e.created_at),
            "updatedAt": iso(e.updated_at),
            "channel": e.channel,
            "integrationWhatsappId": id_text(e.integration_whatsapp_id),
            "integrationMessengerId": id_text(e.integration_messenger_id),
            "integrationInstagramId": id_text(e.integration_instagram_id),
            "wabaId": e.waba_id,
            "source": e.source,
            "eventType": e.event_type,
            "ctwaClid": e.ctwa_clid,
            "adId": e.ad_id,
            "contactInboxId": id_text(e.contact_inbox_id),
            "currency": e.currency,
            "value": e.value,
            "orderId": e.order_id,
            "contents": e.contents.map(|c| c.0),
            "occurredAt": iso(e.occurred_at),
            "sourceEventId": e.source_event_id,
            "capiStatus": e.capi_status,
            "capiSentAt": e.capi_sent_at.map(iso),
        }))
    })
    .await
}
