//! Port of `repositories/contact-referral-event/repository.ts`
//! (`contactReferralEventRepository`, P3.2b).

use std::collections::BTreeMap;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::ContactReferralEvent;
use super::patch::{Patch, insert_clause};
use super::sql::Sql;
use super::utils::like_contains;
use super::{Error, Json, Model, Result, id_list, new_id, now_ms};

/// `adTouchScope` over the unaliased table (`with_ad_id`: a non-empty ad id).
fn ad_touch_scope(workspace_id: i64, with_ad_id: bool) -> Sql {
    let mut q = Sql::bind("\"workspaceId\" = ", workspace_id);
    q.push(" AND \"type\" = 'ad'");
    if with_ad_id {
        q.push(" AND \"adId\" IS NOT NULL AND \"adId\" <> ''");
    }
    q
}

/// `filterableAdId`: a positive bigint that fits `i64`.
const FILTERABLE_AD_ID: &str = "(\"adId\" REGEXP '^[1-9][0-9]*$' AND (length(\"adId\") < 19 \
     OR (length(\"adId\") = 19 AND \"adId\" <= '9223372036854775807')))";

fn text_list(values: &[String]) -> String {
    serde_json::Value::from(values.to_vec()).to_string()
}

/// A decimal text as `(mantissa, scale)`.
fn parse_decimal(text: &str) -> Result<(i128, u32)> {
    let bad = || Error::msg(format!("invalid decimal value {text}"));
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return Err(bad());
    }
    let scale = u32::try_from(frac.len()).map_err(|_| bad())?;
    let mantissa: i128 = format!("{int}{frac}").parse().map_err(|_| bad())?;
    Ok((if negative { -mantissa } else { mantissa }, scale))
}

/// The exact sum of decimal texts (Postgres `numeric` SUM: the largest input scale).
fn decimal_sum<'a>(values: impl Iterator<Item = &'a str>) -> Result<String> {
    let mut parts = Vec::new();
    for v in values {
        parts.push(parse_decimal(v)?);
    }
    let scale = parts.iter().map(|p| p.1).max().unwrap_or(0);
    let total: i128 = parts.iter().map(|(m, s)| m * 10_i128.pow(scale - s)).sum();
    if scale == 0 {
        return Ok(total.to_string());
    }
    let unit = 10_i128.pow(scale);
    let sign = if total < 0 { "-" } else { "" };
    let abs = total.abs();
    Ok(format!(
        "{sign}{}.{:0width$}",
        abs / unit,
        abs % unit,
        width = scale as usize
    ))
}

/// `RECENT_AD_TOUCH_SCAN_LIMIT`: how many of the newest ad touches `listRecentAds` scans.
pub const RECENT_AD_TOUCH_SCAN_LIMIT: i64 = 2000;

/// `RecentAd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentAd {
    pub ad_id: String,
    pub title: Option<String>,
}

/// `ContactAdTouch`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactAdTouch {
    pub id: i64,
    pub channel: String,
    pub provider: Option<String>,
    pub occurred_at: i64,
    pub ad_id: Option<String>,
    pub campaign_id: Option<String>,
    pub ad_group_id: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub metadata: Option<Json>,
}

/// `ContactAdTouchCursor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactAdTouchCursor {
    pub occurred_at: i64,
    pub id: i64,
}

/// One row of `aggregateAdTouchStats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdTouchStatsRow {
    pub ad_id: String,
    pub title: Option<String>,
    pub any_touch_contacts: i64,
    pub first_touch_contacts: i64,
}

/// One row of `aggregateMetaLinkedConversions` (`amount` = exact decimal text of the sum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedConversionRow {
    pub ad_id: String,
    pub event_type: String,
    pub currency: Option<String>,
    pub count: i64,
    pub amount: String,
}

/// `AdStatsRange` (`since` / `until` inclusive, unix ms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdStatsRange {
    pub workspace_id: i64,
    pub since: i64,
    pub until: i64,
    pub channel: Option<String>,
}

/// Input of `findLatestAdTouchId`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindLatestAdTouchInput {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    pub provider: String,
    pub click_id: Option<String>,
    pub ad_id: Option<String>,
}

/// Input of `listRecentAds`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListRecentAdsInput {
    pub workspace_id: i64,
    pub limit: i64,
    pub search: Option<String>,
    pub ad_ids: Option<Vec<String>>,
}

/// `insertIgnoreDuplicate`: `None` when `(workspaceId, contactInboxId, dedupKey)` exists.
pub fn insert_ignore_duplicate(
    conn: &Connection,
    values: &Patch,
) -> Result<Option<ContactReferralEvent>> {
    let now = now_ms();
    let mut all: Patch = Vec::new();
    for (col, v) in [
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
    ] {
        if !values.iter().any(|(c, _)| *c == col) {
            all.push((col, v));
        }
    }
    all.extend(values.iter().cloned());
    let mut q = Sql::raw("INSERT INTO \"ContactReferralEvent\" ");
    q.append(insert_clause(ContactReferralEvent::COLUMNS, &all)?);
    q.push(&format!(
        " ON CONFLICT (\"workspaceId\", \"contactInboxId\", \"dedupKey\") DO NOTHING RETURNING {}",
        ContactReferralEvent::columns_sql(None)
    ));
    q.model(conn)
}

/// `findLatestAdTouchId`: the newest ad touch of the provider on the contact inbox,
/// pinned to the click id (else to the ad id) when given.
pub fn find_latest_ad_touch_id(
    conn: &Connection,
    input: &FindLatestAdTouchInput,
) -> Result<Option<i64>> {
    let mut q = Sql::raw("SELECT \"id\" FROM \"ContactReferralEvent\" WHERE ");
    q.append(ad_touch_scope(input.workspace_id, false));
    q.push(" AND \"contactInboxId\" = ");
    q.push_bind(input.contact_inbox_id);
    q.push(" AND \"provider\" = ");
    q.push_bind(input.provider.clone());
    // `pinnedTouchFilter`: the click id wins over the ad id.
    if let Some(click_id) = input.click_id.as_deref().filter(|c| !c.is_empty()) {
        q.push(" AND \"clickId\" = ");
        q.push_bind(click_id.to_owned());
    } else if let Some(ad_id) = input.ad_id.as_deref().filter(|a| !a.is_empty()) {
        q.push(" AND \"adId\" = ");
        q.push_bind(ad_id.to_owned());
    }
    q.push(" ORDER BY \"occurredAt\" DESC, \"id\" DESC LIMIT 1");
    Ok(q.query_map(conn, |r| r.get(0))?.into_iter().next())
}

/// `listRecentAds`: the ads of the workspace's newest ad touches (bounded scan).
pub fn list_recent_ads(conn: &Connection, input: &ListRecentAdsInput) -> Result<Vec<RecentAd>> {
    let mut q = Sql::raw(
        "SELECT \"adId\", \"title\" FROM (SELECT \"adId\", max(\"occurredAt\") AS \"last\", \
         (SELECT r2.\"title\" FROM recent AS r2 WHERE r2.\"adId\" = recent.\"adId\" AND r2.\"title\" IS NOT NULL \
         ORDER BY r2.\"occurredAt\" DESC, r2.\"id\" DESC LIMIT 1) AS \"title\" FROM recent GROUP BY \"adId\") AS g WHERE 1 = 1",
    );
    if let Some(ids) = &input.ad_ids {
        q.push(" AND g.\"adId\" IN (SELECT value FROM json_each(");
        q.push_bind(text_list(ids));
        q.push("))");
    }
    if let Some(search) = input.search.as_deref().filter(|s| !s.is_empty()) {
        let pattern = like_contains(search);
        q.push(" AND (ilike(g.\"adId\", ");
        q.push_bind(pattern.clone());
        q.push(") OR ilike(g.\"title\", ");
        q.push_bind(pattern);
        q.push("))");
    }
    q.push(" ORDER BY g.\"last\" DESC, g.\"adId\" LIMIT ");
    q.push_bind(input.limit);
    let mut full = Sql::raw(
        "WITH recent AS (SELECT \"id\", \"adId\", \"metadata\" ->> '$.adTitle' AS \"title\", \"occurredAt\" \
         FROM \"ContactReferralEvent\" WHERE ",
    );
    full.append(ad_touch_scope(input.workspace_id, true));
    full.push(&format!(
        " AND {FILTERABLE_AD_ID} ORDER BY \"occurredAt\" DESC, \"id\" DESC LIMIT {RECENT_AD_TOUCH_SCAN_LIMIT}) "
    ));
    full.append(q);
    let rows: Vec<(Option<String>, Option<String>)> =
        full.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows
        .into_iter()
        .filter_map(|(ad_id, title)| ad_id.map(|ad_id| RecentAd { ad_id, title }))
        .collect())
}

/// `listAdTouchesByContact`: newest first, `limit + 1` rows past `cursor`.
pub fn list_ad_touches_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    limit: i64,
    cursor: Option<ContactAdTouchCursor>,
) -> Result<Vec<ContactAdTouch>> {
    let mut q = Sql::raw(
        "SELECT \"id\", \"channel\", \"provider\", \"occurredAt\", \"adId\", \"campaignId\", \"adGroupId\", \
         \"source\", \"sourceUrl\", \"metadata\" FROM \"ContactReferralEvent\" WHERE ",
    );
    q.append(ad_touch_scope(workspace_id, false));
    q.push(" AND \"contactId\" = ");
    q.push_bind(contact_id);
    if let Some(cursor) = cursor {
        q.push(" AND (\"occurredAt\" < ");
        q.push_bind(cursor.occurred_at);
        q.push(" OR (\"occurredAt\" = ");
        q.push_bind(cursor.occurred_at);
        q.push(" AND \"id\" < ");
        q.push_bind(cursor.id);
        q.push("))");
    }
    q.push(" ORDER BY \"occurredAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(limit + 1);
    q.query_map(conn, |r| {
        Ok(ContactAdTouch {
            id: r.get(0)?,
            channel: r.get(1)?,
            provider: r.get(2)?,
            occurred_at: r.get(3)?,
            ad_id: r.get(4)?,
            campaign_id: r.get(5)?,
            ad_group_id: r.get(6)?,
            source: r.get(7)?,
            source_url: r.get(8)?,
            metadata: r.get(9)?,
        })
    })
}

/// `aggregateAdTouchStats`: contacts per ad (any touch / first touch) in the range.
pub fn aggregate_ad_touch_stats(
    conn: &Connection,
    range: &AdStatsRange,
    limit: i64,
) -> Result<Vec<AdTouchStatsRow>> {
    let mut q = Sql::raw(
        "WITH touches AS (SELECT \"contactId\", \"adId\", \"occurredAt\", \"id\", \"metadata\" ->> '$.adTitle' AS \"title\" \
         FROM \"ContactReferralEvent\" WHERE ",
    );
    q.append(ad_touch_scope(range.workspace_id, true));
    q.push(" AND \"occurredAt\" >= ");
    q.push_bind(range.since);
    q.push(" AND \"occurredAt\" <= ");
    q.push_bind(range.until);
    if let Some(channel) = &range.channel {
        q.push(" AND \"channel\" = ");
        q.push_bind(channel.clone());
    }
    q.push(
        "), firsts AS (SELECT \"contactId\", \"adId\" FROM (SELECT \"contactId\", \"adId\", row_number() OVER \
         (PARTITION BY \"contactId\" ORDER BY \"occurredAt\", \"id\") AS rn FROM touches) WHERE rn = 1) \
         SELECT a.\"adId\", (SELECT t2.\"title\" FROM touches AS t2 WHERE t2.\"adId\" = a.\"adId\" AND t2.\"title\" IS NOT NULL \
         ORDER BY t2.\"occurredAt\" DESC, t2.\"id\" DESC LIMIT 1), a.\"contacts\", \
         coalesce((SELECT count(*) FROM firsts AS f WHERE f.\"adId\" = a.\"adId\"), 0) \
         FROM (SELECT \"adId\", count(DISTINCT \"contactId\") AS \"contacts\" FROM touches GROUP BY \"adId\") AS a \
         ORDER BY a.\"contacts\" DESC, a.\"adId\" LIMIT ",
    );
    q.push_bind(limit);
    q.query_map(conn, |r| {
        Ok(AdTouchStatsRow {
            ad_id: r.get(0)?,
            title: r.get(1)?,
            any_touch_contacts: r.get(2)?,
            first_touch_contacts: r.get(3)?,
        })
    })
}

/// `aggregateMetaLinkedConversions`: conversions linked to a touch of one of `ad_ids`.
pub fn aggregate_meta_linked_conversions(
    conn: &Connection,
    range: &AdStatsRange,
    ad_ids: &[String],
) -> Result<Vec<LinkedConversionRow>> {
    if ad_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = Sql::bind(
        "SELECT r.\"adId\", e.\"eventType\", e.\"currency\", e.\"value\" FROM \"AdsConversionEvent\" AS e \
         JOIN \"ContactReferralEvent\" AS r ON r.\"id\" = e.\"referralEventId\" WHERE e.\"workspaceId\" = ",
        range.workspace_id,
    );
    q.push(" AND e.\"occurredAt\" >= ");
    q.push_bind(range.since);
    q.push(" AND e.\"occurredAt\" <= ");
    q.push_bind(range.until);
    q.push(" AND r.\"adId\" IN (SELECT value FROM json_each(");
    q.push_bind(text_list(ad_ids));
    q.push("))");
    if let Some(channel) = &range.channel {
        q.push(" AND e.\"channel\" = ");
        q.push_bind(channel.clone());
    }
    q.push(" ORDER BY r.\"adId\", e.\"eventType\", e.\"currency\", e.\"id\"");
    type Raw = (String, String, Option<String>, Option<String>);
    let rows: Vec<Raw> = q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let mut out: Vec<(LinkedConversionRow, Vec<String>)> = Vec::new();
    for (ad_id, event_type, currency, value) in rows {
        let same = out.last().is_some_and(|(row, _)| {
            row.ad_id == ad_id && row.event_type == event_type && row.currency == currency
        });
        if !same {
            out.push((
                LinkedConversionRow {
                    ad_id,
                    event_type,
                    currency,
                    count: 0,
                    amount: String::new(),
                },
                Vec::new(),
            ));
        }
        if let Some((row, values)) = out.last_mut() {
            row.count += 1;
            values.extend(value);
        }
    }
    out.into_iter()
        .map(|(mut row, values)| {
            row.amount = decimal_sum(values.iter().map(String::as_str))?;
            Ok(row)
        })
        .collect()
}

/// `listAdIdsByContact`.
pub fn list_ad_ids_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    limit: i64,
) -> Result<Vec<String>> {
    Ok(
        list_ad_ids_by_contacts(conn, workspace_id, &[contact_id], limit)?
            .remove(&contact_id)
            .unwrap_or_default(),
    )
}

/// `listAdIdsByContacts`: each contact's distinct ad ids, oldest first touch first, at
/// most `limit_per_contact` (a contact without a touch has no entry).
pub fn list_ad_ids_by_contacts(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
    limit_per_contact: i64,
) -> Result<BTreeMap<i64, Vec<String>>> {
    let mut out = BTreeMap::new();
    if contact_ids.is_empty() {
        return Ok(out);
    }
    let mut q = Sql::raw(
        "SELECT \"contactId\", \"adId\" FROM (SELECT \"contactId\", \"adId\", row_number() OVER \
         (PARTITION BY \"contactId\" ORDER BY min(\"occurredAt\"), \"adId\") AS \"position\" \
         FROM \"ContactReferralEvent\" WHERE ",
    );
    q.append(ad_touch_scope(workspace_id, true));
    q.push(" AND \"contactId\" IN (SELECT value FROM json_each(");
    q.push_bind(id_list(contact_ids));
    q.push(")) GROUP BY \"contactId\", \"adId\") WHERE \"position\" <= ");
    q.push_bind(limit_per_contact);
    q.push(" ORDER BY \"contactId\", \"position\"");
    let rows: Vec<(i64, String)> = q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?;
    for (contact_id, ad_id) in rows {
        out.entry(contact_id).or_insert_with(Vec::new).push(ad_id);
    }
    Ok(out)
}
