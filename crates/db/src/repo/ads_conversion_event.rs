//! Port of `repositories/ads-conversion-event/repository.ts`.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::AdsConversionEvent;
use super::queries::ad_referral::{
    ad_conversation_predicate, any_channel_ad_conversation_predicate,
};
use super::queries::ctwa_retarget::{CtwaSegmentPredicateInput, build_ctwa_segment_predicate};
use super::queries::date_bucket::zoned_date_key;
use super::sql::Sql;
use super::{Error, Json, Model, Result, new_id, now_ms};

/// `DEFAULT_ADS_CONVERSION_CHANNEL`.
pub const DEFAULT_ADS_CONVERSION_CHANNEL: &str = "whatsapp";

/// Insert values of `insertIgnoreDuplicate` (`AdsConversionEventCreateValues`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AdsConversionEventCreate {
    pub workspace_id: i64,
    /// `None` = the DB default (`whatsapp`).
    pub channel: Option<String>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub waba_id: Option<String>,
    pub source: String,
    pub event_type: String,
    pub ctwa_clid: Option<String>,
    pub ad_id: Option<String>,
    pub contact_inbox_id: Option<i64>,
    pub referral_event_id: Option<i64>,
    pub currency: Option<String>,
    pub value: Option<String>,
    pub order_id: Option<String>,
    pub contents: Option<Json>,
    pub occurred_at: i64,
    pub source_event_id: String,
    /// `None` = the DB default (`pending`).
    pub capi_status: Option<String>,
    pub capi_sent_at: Option<i64>,
}

/// `AdsConversionEventAttribution` (`ContactInbox.id`, `referral`, `IntegrationWhatsapp.wabaId`).
#[derive(Debug, Clone, PartialEq)]
pub struct Attribution {
    pub id: i64,
    pub referral: Option<Json>,
    pub waba_id: String,
}

/// `AdReferralAttribution` (`ContactInbox.id`, `referral`).
#[derive(Debug, Clone, PartialEq)]
pub struct AdReferralAttribution {
    pub id: i64,
    pub referral: Option<Json>,
}

/// `DateRangeInput` (plus the fields of `AdConversationDateRangeInput`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateRangeInput {
    pub workspace_id: i64,
    pub since: i64,
    pub until: i64,
    pub integration_whatsapp_id: Option<i64>,
    pub channel: Option<String>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub all_channels: bool,
    pub timezone: Option<String>,
}

/// One row of the conversation counts (`...ConversationCountBy...`); `date` is set by the
/// day-bucketed variants, `channel` by the all-channel variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationCount {
    pub date: Option<String>,
    pub ad_id: Option<String>,
    pub channel: Option<String>,
    pub conversations: i64,
}

/// One row of `countConversionEventsByAd` / `...ByDayAndAd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCount {
    pub date: Option<String>,
    pub ad_id: Option<String>,
    pub event_type: String,
    /// Only with `allChannels`.
    pub channel: Option<String>,
    pub count: i64,
    /// `SUM(value)` of the purchase events as decimal text (`ByAd` only).
    pub purchase_value: Option<String>,
}

/// `AdsConversionCapiStatusCount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapiStatusCount {
    pub capi_status: String,
    pub count: i64,
}

/// `ExportSegmentInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportSegmentInput {
    pub range: DateRangeInput,
    /// `conversations`, `leads` or `purchases`.
    pub segment: String,
    pub ad_id: Option<String>,
    pub after_id: Option<i64>,
    pub limit: i64,
}

/// `AdsConversionExportRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub id: i64,
    pub contact_id: i64,
    pub contact_name: Option<String>,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub ad_id: Option<String>,
    pub occurred_at: i64,
    /// Only with `allChannels`.
    pub channel: Option<String>,
}

/// `ExportSegmentRowsResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSegmentRows {
    pub rows: Vec<ExportRow>,
    pub has_more: bool,
}

/// `FindWorkspaceEventInput` + `UpdateCapiStatusInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCapiStatusInput {
    pub id: i64,
    pub workspace_id: i64,
    pub from: String,
    pub to: String,
    pub capi_sent_at: Option<i64>,
}

/// `findBySourceEventId` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindBySourceEventInput {
    pub workspace_id: i64,
    pub integration_whatsapp_id: Option<i64>,
    pub channel: Option<String>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub source: String,
    pub source_event_id: String,
}

/// `findAttributionByAdReferral` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdReferralInput {
    pub workspace_id: i64,
    /// `messenger` or `instagram`.
    pub channel: String,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
    pub contact_inbox_id: i64,
}

/// The integration FK column of the per-channel partial unique index (`getConflictTarget`).
fn conflict_column(channel: &str) -> Option<&'static str> {
    match channel {
        "whatsapp" => Some("integrationWhatsappId"),
        "messenger" => Some("integrationMessengerId"),
        "instagram" => Some("integrationInstagramId"),
        _ => None,
    }
}

/// `insertIgnoreDuplicate`: inserts the event, or returns `None` when the per-channel
/// partial unique index `(workspaceId, integration<Channel>Id, source, sourceEventId) WHERE
/// channel = '<channel>'` already has it. A channel without such an index (`facebook`)
/// fails with `no dedup unique index`.
pub fn insert_ignore_duplicate(
    conn: &Connection,
    values: &AdsConversionEventCreate,
) -> Result<Option<AdsConversionEvent>> {
    let channel = values
        .channel
        .clone()
        .unwrap_or_else(|| DEFAULT_ADS_CONVERSION_CHANNEL.to_owned());
    let Some(column) = conflict_column(&channel) else {
        return Err(Error::msg(format!(
            "insertIgnoreDuplicate: no dedup unique index for channel \"{channel}\""
        )));
    };
    let now = now_ms();
    let sql = format!(
        "INSERT INTO \"AdsConversionEvent\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"channel\", \
         \"integrationWhatsappId\", \"integrationMessengerId\", \"integrationInstagramId\", \"wabaId\", \"source\", \
         \"eventType\", \"ctwaClid\", \"adId\", \"contactInboxId\", \"referralEventId\", \"currency\", \"value\", \
         \"orderId\", \"contents\", \"occurredAt\", \"sourceEventId\", \"capiStatus\", \"capiSentAt\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, \
         COALESCE(?21, 'pending'), ?22) \
         ON CONFLICT (\"workspaceId\", \"{column}\", \"source\", \"sourceEventId\") WHERE \"channel\" = '{channel}' DO NOTHING \
         RETURNING {}",
        AdsConversionEvent::columns_sql(None)
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(rusqlite::params![
        new_id(),
        now,
        values.workspace_id,
        channel,
        values.integration_whatsapp_id,
        values.integration_messenger_id,
        values.integration_instagram_id,
        values.waba_id,
        values.source,
        values.event_type,
        values.ctwa_clid,
        values.ad_id,
        values.contact_inbox_id,
        values.referral_event_id,
        values.currency,
        values.value,
        values.order_id,
        values.contents,
        values.occurred_at,
        values.source_event_id,
        values.capi_status,
        values.capi_sent_at,
    ])?;
    match rows.next()? {
        Some(row) => Ok(Some(AdsConversionEvent::from_row(row)?)),
        None => Ok(None),
    }
}

/// `findBySourceEventId`.
pub fn find_by_source_event_id(
    conn: &Connection,
    input: &FindBySourceEventInput,
) -> Result<Option<AdsConversionEvent>> {
    let mut q = Sql::raw(format!("{} WHERE ", AdsConversionEvent::select_sql()));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        Some(Sql::bind("\"source\" = ", input.source.clone())),
        Some(Sql::bind(
            "\"sourceEventId\" = ",
            input.source_event_id.clone(),
        )),
        input
            .integration_whatsapp_id
            .map(|id| Sql::bind("\"integrationWhatsappId\" = ", id)),
        input
            .channel
            .clone()
            .map(|c| Sql::bind("\"channel\" = ", c)),
        input
            .integration_messenger_id
            .map(|id| Sql::bind("\"integrationMessengerId\" = ", id)),
        input
            .integration_instagram_id
            .map(|id| Sql::bind("\"integrationInstagramId\" = ", id)),
    ]));
    q.push(" LIMIT 1");
    q.model(conn)
}

fn attribution_by_whatsapp(
    conn: &Connection,
    workspace_id: i64,
    integration_whatsapp_id: i64,
    filter: Sql,
) -> Result<Option<Attribution>> {
    let mut q = Sql::new(
        "SELECT ci.\"id\", ci.\"referral\", iw.\"wabaId\" FROM \"ContactInbox\" AS ci \
         INNER JOIN \"IntegrationWhatsapp\" AS iw ON ci.\"inboxId\" = iw.\"inboxId\" AND iw.\"id\" = ? AND iw.\"workspaceId\" = ? \
         WHERE ",
        vec![
            Value::Integer(integration_whatsapp_id),
            Value::Integer(workspace_id),
        ],
    );
    q.append(filter);
    q.push(" LIMIT 1");
    Ok(q.query_map(conn, |row| {
        Ok(Attribution {
            id: row.get(0)?,
            referral: row.get(1)?,
            waba_id: row.get(2)?,
        })
    })?
    .into_iter()
    .next())
}

/// `findAttributionByCtwaClid`.
pub fn find_attribution_by_ctwa_clid(
    conn: &Connection,
    workspace_id: i64,
    integration_whatsapp_id: i64,
    ctwa_clid: &str,
) -> Result<Option<Attribution>> {
    attribution_by_whatsapp(
        conn,
        workspace_id,
        integration_whatsapp_id,
        Sql::bind("ci.\"referral\"->>'ctwaClid' = ", ctwa_clid.to_owned()),
    )
}

/// `findAttributionByContactInbox`.
pub fn find_attribution_by_contact_inbox(
    conn: &Connection,
    workspace_id: i64,
    integration_whatsapp_id: i64,
    contact_inbox_id: i64,
) -> Result<Option<Attribution>> {
    attribution_by_whatsapp(
        conn,
        workspace_id,
        integration_whatsapp_id,
        Sql::bind("ci.\"id\" = ", contact_inbox_id)
            .closed(" AND ci.\"referral\"->>'ctwaClid' IS NOT NULL"),
    )
}

fn ad_referral_integration_id(
    channel: &str,
    messenger: Option<i64>,
    instagram: Option<i64>,
) -> Option<i64> {
    if channel == "messenger" {
        messenger
    } else {
        instagram
    }
}

fn ad_referral_table(channel: &str) -> &'static str {
    if channel == "messenger" {
        "IntegrationMessenger"
    } else {
        "IntegrationInstagram"
    }
}

/// `findAttributionByAdReferral`: `None` without the channel's integration id (no query).
pub fn find_attribution_by_ad_referral(
    conn: &Connection,
    input: &AdReferralInput,
) -> Result<Option<AdReferralAttribution>> {
    let Some(integration_id) = ad_referral_integration_id(
        &input.channel,
        input.integration_messenger_id,
        input.integration_instagram_id,
    ) else {
        return Ok(None);
    };
    let table = ad_referral_table(&input.channel);
    let mut q = Sql::new(
        format!(
            "SELECT ci.\"id\", ci.\"referral\" FROM \"ContactInbox\" AS ci \
             INNER JOIN \"{table}\" AS i ON ci.\"inboxId\" = i.\"inboxId\" AND i.\"id\" = ? AND i.\"workspaceId\" = ? WHERE "
        ),
        vec![
            Value::Integer(integration_id),
            Value::Integer(input.workspace_id),
        ],
    );
    q.append(Sql::and([
        Some(Sql::bind("ci.\"id\" = ", input.contact_inbox_id)),
        Some(Sql::bind("ci.\"channel\" = ", input.channel.clone())),
        Some(ad_conversation_predicate("ci", &input.channel)),
    ]));
    q.push(" LIMIT 1");
    Ok(q.query_map(conn, |row| {
        Ok(AdReferralAttribution {
            id: row.get(0)?,
            referral: row.get(1)?,
        })
    })?
    .into_iter()
    .next())
}

/// `listPendingCapi`: `capiStatus = 'pending' AND capiSentAt IS NULL` (of one workspace
/// when given), oldest `occurredAt` first.
pub fn list_pending_capi(
    conn: &Connection,
    workspace_id: Option<i64>,
) -> Result<Vec<AdsConversionEvent>> {
    let mut q = Sql::raw(format!(
        "{} WHERE \"capiStatus\" = 'pending' AND \"capiSentAt\" IS NULL",
        AdsConversionEvent::select_sql()
    ));
    if let Some(ws) = workspace_id {
        q.push(" AND \"workspaceId\" = ");
        q.push_bind(ws);
    }
    q.push(" ORDER BY \"occurredAt\" ASC");
    q.models(conn)
}

/// `findWorkspaceEvent`.
pub fn find_workspace_event(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<AdsConversionEvent>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `updateCapiStatus`: compare-and-set from `from` to `to` (`None` when the row is not in
/// `from` any more).
pub fn update_capi_status(
    conn: &Connection,
    input: &UpdateCapiStatusInput,
) -> Result<Option<AdsConversionEvent>> {
    let rows = super::returning::<AdsConversionEvent>(
        conn,
        "UPDATE \"AdsConversionEvent\" SET \"capiStatus\" = ?1, \"capiSentAt\" = ?2, \"updatedAt\" = ?6 \
         WHERE \"id\" = ?3 AND \"workspaceId\" = ?4 AND \"capiStatus\" = ?5",
        &[
            &input.to,
            &input.capi_sent_at,
            &input.id,
            &input.workspace_id,
            &input.from,
            &now_ms(),
        ],
    )?;
    Ok(rows.into_iter().next())
}

fn ctwa_conversation_filters(input: &DateRangeInput) -> Sql {
    Sql::and([
        Some(Sql::bind("c.\"workspaceId\" = ", input.workspace_id)),
        Some(ad_conversation_predicate("ci", "whatsapp")),
        Some(Sql::bind("ci.\"firstInteractionAt\" >= ", input.since)),
        Some(Sql::bind("ci.\"firstInteractionAt\" <= ", input.until)),
    ])
}

fn whatsapp_join(input: &DateRangeInput) -> Sql {
    match input.integration_whatsapp_id {
        Some(id) => Sql::new(
            " INNER JOIN \"IntegrationWhatsapp\" AS iw ON ci.\"inboxId\" = iw.\"inboxId\" AND iw.\"id\" = ? AND iw.\"workspaceId\" = ?",
            vec![Value::Integer(id), Value::Integer(input.workspace_id)],
        ),
        None => Sql::default(),
    }
}

fn read_conversation_counts(
    conn: &Connection,
    q: &Sql,
    with_date: bool,
    with_channel: bool,
) -> Result<Vec<ConversationCount>> {
    q.query_map(conn, |row| {
        let mut i = 0;
        let date = if with_date {
            i += 1;
            Some(row.get::<_, String>(0)?)
        } else {
            None
        };
        let ad_id: Option<String> = row.get(i)?;
        i += 1;
        let channel = if with_channel {
            i += 1;
            Some(row.get::<_, String>(i - 1)?)
        } else {
            None
        };
        Ok(ConversationCount {
            date,
            ad_id,
            channel,
            conversations: row.get(i)?,
        })
    })
}

fn conversation_count_query(
    date: Option<&str>,
    with_channel: bool,
    joins: Sql,
    filters: Sql,
) -> Sql {
    let mut q = Sql::raw("SELECT ");
    if let Some(tz) = date {
        q.append(zoned_date_key("ci.\"firstInteractionAt\"", tz));
        q.push(", ");
    }
    q.push("ci.\"referral\"->>'adId'");
    if with_channel {
        q.push(", ci.\"channel\"");
    }
    q.push(", count(ci.\"id\") FROM \"ContactInbox\" AS ci");
    q.append(joins);
    q.push(" INNER JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" WHERE ");
    q.append(filters);
    q.push(" GROUP BY ");
    if date.is_some() {
        q.push("1, ");
    }
    q.push("ci.\"referral\"->>'adId'");
    if with_channel {
        q.push(", ci.\"channel\"");
    }
    q
}

/// `countCtwaConversationsByAd`.
pub fn count_ctwa_conversations_by_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let q = conversation_count_query(
        None,
        false,
        whatsapp_join(input),
        ctwa_conversation_filters(input),
    );
    read_conversation_counts(conn, &q, false, false)
}

/// `countCtwaConversationsByDayAndAd` (days in `timezone`, default `UTC`).
pub fn count_ctwa_conversations_by_day_and_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let tz = input.timezone.as_deref().unwrap_or("UTC");
    let q = conversation_count_query(
        Some(tz),
        false,
        whatsapp_join(input),
        ctwa_conversation_filters(input),
    );
    read_conversation_counts(conn, &q, true, false)
}

fn ad_conversation_filters(input: &DateRangeInput) -> Sql {
    let channel = input
        .channel
        .clone()
        .unwrap_or_else(|| "messenger".to_owned());
    let scope = ad_referral_integration_id(
        &channel,
        input.integration_messenger_id,
        input.integration_instagram_id,
    )
    .map(|id| {
        let table = ad_referral_table(&channel);
        Sql::new(
            format!(
                "EXISTS (SELECT 1 FROM \"{table}\" AS i WHERE i.\"id\" = ? AND i.\"workspaceId\" = ? AND i.\"inboxId\" = ci.\"inboxId\")"
            ),
            vec![Value::Integer(id), Value::Integer(input.workspace_id)],
        )
    });
    Sql::and([
        Some(Sql::bind("c.\"workspaceId\" = ", input.workspace_id)),
        Some(ad_conversation_predicate("ci", &channel)),
        Some(Sql::bind("ci.\"firstInteractionAt\" >= ", input.since)),
        Some(Sql::bind("ci.\"firstInteractionAt\" <= ", input.until)),
        Some(Sql::bind("ci.\"channel\" = ", channel.clone())),
        scope,
    ])
}

/// `countAdConversationsByAd` (messenger / instagram ad-referral conversations; `channel`
/// is required).
pub fn count_ad_conversations_by_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let q = conversation_count_query(None, false, Sql::default(), ad_conversation_filters(input));
    read_conversation_counts(conn, &q, false, false)
}

/// `countAdConversationsByDayAndAd`.
pub fn count_ad_conversations_by_day_and_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let tz = input.timezone.as_deref().unwrap_or("UTC");
    let q = conversation_count_query(
        Some(tz),
        false,
        Sql::default(),
        ad_conversation_filters(input),
    );
    read_conversation_counts(conn, &q, true, false)
}

fn all_channel_filters(input: &DateRangeInput) -> Sql {
    Sql::and([
        Some(Sql::bind("c.\"workspaceId\" = ", input.workspace_id)),
        Some(any_channel_ad_conversation_predicate("ci")),
        Some(Sql::bind("ci.\"firstInteractionAt\" >= ", input.since)),
        Some(Sql::bind("ci.\"firstInteractionAt\" <= ", input.until)),
    ])
}

/// `countAllChannelConversationsByAd` (group by `(adId, channel)`).
pub fn count_all_channel_conversations_by_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let q = conversation_count_query(None, true, Sql::default(), all_channel_filters(input));
    read_conversation_counts(conn, &q, false, true)
}

/// `countAllChannelConversationsByDayAndAd` (group by `(date, adId, channel)`).
pub fn count_all_channel_conversations_by_day_and_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<ConversationCount>> {
    let tz = input.timezone.as_deref().unwrap_or("UTC");
    let q = conversation_count_query(Some(tz), true, Sql::default(), all_channel_filters(input));
    read_conversation_counts(conn, &q, true, true)
}

fn date_range_event_filters(input: &DateRangeInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        Some(Sql::bind("\"occurredAt\" >= ", input.since)),
        Some(Sql::bind("\"occurredAt\" <= ", input.until)),
        input
            .integration_whatsapp_id
            .map(|id| Sql::bind("\"integrationWhatsappId\" = ", id)),
        if input.all_channels {
            None
        } else {
            Some(Sql::bind(
                "\"channel\" = ",
                input
                    .channel
                    .clone()
                    .unwrap_or_else(|| DEFAULT_ADS_CONVERSION_CHANNEL.to_owned()),
            ))
        },
        input
            .integration_messenger_id
            .map(|id| Sql::bind("\"integrationMessengerId\" = ", id)),
        input
            .integration_instagram_id
            .map(|id| Sql::bind("\"integrationInstagramId\" = ", id)),
    ])
}

/// `countConversionEventsByAd`: per `(adId, eventType)` (plus `channel` with
/// `allChannels`): the count and the purchase value sum (decimal text).
pub fn count_conversion_events_by_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<EventCount>> {
    let channel_col = if input.all_channels {
        ", \"channel\""
    } else {
        ""
    };
    let mut q = Sql::raw(format!(
        "SELECT \"adId\", \"eventType\"{channel_col}, count(\"id\"), \
         json_group_array(CASE WHEN \"eventType\" = 'purchase' THEN \"value\" END) \
         FROM \"AdsConversionEvent\" WHERE "
    ));
    q.append(date_range_event_filters(input));
    q.push(&format!(" GROUP BY \"adId\", \"eventType\"{channel_col}"));
    let all = input.all_channels;
    let raw = q.query_map(conn, |row| {
        let mut i = 2;
        let channel = if all {
            i += 1;
            Some(row.get::<_, String>(2)?)
        } else {
            None
        };
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            channel,
            row.get::<_, i64>(i)?,
            row.get::<_, String>(i + 1)?,
        ))
    })?;
    raw.into_iter()
        .map(|(ad_id, event_type, channel, count, values)| {
            let values: Vec<Option<String>> = serde_json::from_str(&values)
                .map_err(|e| Error::msg(format!("purchase values: {e}")))?;
            Ok(EventCount {
                date: None,
                ad_id,
                event_type,
                channel,
                count,
                purchase_value: decimal_sum(values.iter().flatten().map(String::as_str)),
            })
        })
        .collect()
}

/// `countConversionEventsByDayAndAd` (days in `timezone`, default `UTC`).
pub fn count_conversion_events_by_day_and_ad(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<EventCount>> {
    let tz = input.timezone.as_deref().unwrap_or("UTC");
    let channel_col = if input.all_channels {
        ", \"channel\""
    } else {
        ""
    };
    let mut q = Sql::raw("SELECT ");
    q.append(zoned_date_key("\"occurredAt\"", tz));
    q.push(&format!(
        ", \"adId\", \"eventType\"{channel_col}, count(\"id\") FROM \"AdsConversionEvent\" WHERE "
    ));
    q.append(date_range_event_filters(input));
    q.push(&format!(
        " GROUP BY 1, \"adId\", \"eventType\"{channel_col}"
    ));
    let all = input.all_channels;
    q.query_map(conn, |row| {
        let channel = if all { Some(row.get(3)?) } else { None };
        Ok(EventCount {
            date: Some(row.get(0)?),
            ad_id: row.get(1)?,
            event_type: row.get(2)?,
            channel,
            count: row.get(if all { 4 } else { 3 })?,
            purchase_value: None,
        })
    })
}

/// `countByCapiStatus`.
pub fn count_by_capi_status(
    conn: &Connection,
    input: &DateRangeInput,
) -> Result<Vec<CapiStatusCount>> {
    let mut q = Sql::raw("SELECT \"capiStatus\", count(\"id\") FROM \"AdsConversionEvent\" WHERE ");
    q.append(date_range_event_filters(input));
    q.push(" GROUP BY \"capiStatus\"");
    q.query_map(conn, |row| {
        Ok(CapiStatusCount {
            capi_status: row.get(0)?,
            count: row.get(1)?,
        })
    })
}

/// `listExportSegmentRows`: one page (`limit`, keyset after `afterId`, id order) of the
/// segment's rows; `has_more` from a `limit + 1` over-fetch.
pub fn list_export_segment_rows(
    conn: &Connection,
    input: &ExportSegmentInput,
) -> Result<ExportSegmentRows> {
    let fetch_limit = input.limit + 1;
    let r = &input.range;
    let predicate_input = CtwaSegmentPredicateInput {
        segment: input.segment.clone(),
        ad_id: input.ad_id.clone(),
        since: r.since,
        until: r.until,
        workspace_id: Some(r.workspace_id),
        integration_whatsapp_id: r.integration_whatsapp_id,
        channel: r.channel.clone(),
        integration_messenger_id: r.integration_messenger_id,
        integration_instagram_id: r.integration_instagram_id,
    };
    let all = r.all_channels;
    let mut q;
    if input.segment == "conversations" {
        q = Sql::raw(format!(
            "SELECT ci.\"id\", c.\"id\", c.\"fullName\", c.\"phoneNumber\", c.\"email\", ci.\"referral\"->>'adId', \
             ci.\"firstInteractionAt\"{} FROM \"ContactInbox\" AS ci INNER JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" WHERE ",
            if all { ", ci.\"channel\"" } else { "" }
        ));
        q.append(Sql::and([
            Some(Sql::bind("c.\"workspaceId\" = ", r.workspace_id)),
            Some(build_ctwa_segment_predicate(&predicate_input)),
            input.after_id.map(|id| Sql::bind("ci.\"id\" > ", id)),
        ]));
        q.push(" ORDER BY ci.\"id\" ASC LIMIT ");
    } else {
        q = Sql::raw(format!(
            "SELECT ace.\"id\", c.\"id\", c.\"fullName\", c.\"phoneNumber\", c.\"email\", ace.\"adId\", ace.\"occurredAt\"{} \
             FROM \"AdsConversionEvent\" AS ace INNER JOIN \"ContactInbox\" AS ci ON ci.\"id\" = ace.\"contactInboxId\" \
             INNER JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" WHERE ",
            if all { ", ace.\"channel\"" } else { "" }
        ));
        q.append(Sql::and([
            Some(build_ctwa_segment_predicate(&predicate_input)),
            input.after_id.map(|id| Sql::bind("ace.\"id\" > ", id)),
        ]));
        q.push(" ORDER BY ace.\"id\" ASC LIMIT ");
    }
    q.push_bind(fetch_limit);
    let conversations = input.segment == "conversations";
    let mut rows = q.query_map(conn, |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            if all {
                Some(row.get::<_, String>(7)?)
            } else {
                None
            },
        ))
    })?;
    let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > input.limit;
    if has_more {
        rows.truncate(usize::try_from(input.limit).unwrap_or(0));
    }
    let rows = rows
        .into_iter()
        .map(
            |(id, contact_id, contact_name, phone_number, email, ad_id, occurred_at, channel)| {
                let Some(occurred_at) = occurred_at else {
                    return Err(Error::msg(if conversations {
                        "listExportSegmentRows: conversations row is missing firstInteractionAt despite the predicate's date-range bounds"
                    } else {
                        "listExportSegmentRows: event row is missing occurredAt"
                    }));
                };
                Ok(ExportRow {
                    id,
                    contact_id,
                    contact_name,
                    phone_number,
                    email,
                    ad_id,
                    occurred_at,
                    channel,
                })
            },
        )
        .collect::<Result<Vec<_>>>()?;
    Ok(ExportSegmentRows { rows, has_more })
}

/// Exact sum of decimal strings (Postgres `SUM(numeric)`): scale = the largest input
/// scale. `None` for no input (SQL `SUM` of no rows / only NULLs).
pub fn decimal_sum<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let parsed: Vec<(i128, u32)> = values.filter_map(parse_decimal).collect();
    if parsed.is_empty() {
        return None;
    }
    let scale = parsed.iter().map(|(_, s)| *s).max().unwrap_or(0);
    let mut total: i128 = 0;
    for (units, s) in parsed {
        total += units * 10i128.pow(scale - s);
    }
    Some(format_decimal(total, scale))
}

fn parse_decimal(text: &str) -> Option<(i128, u32)> {
    let t = text.trim();
    let (neg, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let digits = format!("{int}{frac}");
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let units: i128 = if digits.is_empty() {
        0
    } else {
        digits.parse().ok()?
    };
    Some((
        if neg { -units } else { units },
        u32::try_from(frac.len()).ok()?,
    ))
}

fn format_decimal(units: i128, scale: u32) -> String {
    let neg = units < 0;
    let digits = units.unsigned_abs().to_string();
    let scale = scale as usize;
    let body = if scale == 0 {
        digits
    } else {
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let (i, f) = padded.split_at(padded.len() - scale);
        format!("{i}.{f}")
    };
    if neg { format!("-{body}") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_sum_keeps_the_largest_scale() {
        assert_eq!(
            decimal_sum(["10.5", "2.25", "1"].into_iter()),
            Some("13.75".to_owned())
        );
        assert_eq!(
            decimal_sum(["10.50", "-11"].into_iter()),
            Some("-0.50".to_owned())
        );
        assert_eq!(decimal_sum(std::iter::empty()), None);
        assert_eq!(decimal_sum(["7"].into_iter()), Some("7".to_owned()));
    }
}
