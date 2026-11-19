//! Read side of Google Ads (`google-ads/stats.ts`, `googleAdsConversionService.listEvents`,
//! `integrationGoogleAdsService.getPublicSetup`, `googleAdsSettingsService.getConsent`)
//! and the builder projections the public routes return (`to-event-resource.ts`,
//! `to-consent-view.ts`, `load-google-ads-integration.ts`). The send / record /
//! housekeeping side is not ported here (see specs/Q.D7.md).

use chatbotx_core::utils::connection::{ACTIVE_CONNECTION_STATUSES, ConnectionStatus};
use chatbotx_db::Db;
use chatbotx_db::repo::google_ads_conversion_event::{
    self as events, GOOGLE_ADS_EVENTS_MAX_PER_PAGE, ListInput, StatsFilters, StatusCounts,
};
use chatbotx_db::repo::models::GoogleAdsConversionEvent;
use chatbotx_db::repo::partials::google_ads::{
    GoogleAdsConsentSource, parse_google_ads_event_options, parse_google_ads_settings_document,
};
use chatbotx_db::repo::{connection, google_ads_settings, integration_google_ads, iso, now_ms};
use serde_json::{Value, json};

use crate::email_marketing::open_auth;
use crate::errors::Result;
use crate::pipeline::analytics::{day_keys, parse_analytics_date_range};
use crate::tx;

/// `googleAdsEventStatusValues`.
pub const EVENT_STATUSES: &[&str] = &[
    "pending",
    "sending",
    "sent",
    "processed",
    "failed",
    "skipped_no_account",
    "skipped_expired",
];
/// `BY_ACTION_LIMIT`.
const BY_ACTION_LIMIT: usize = 50;
const REDACTED: &str = "[redacted]";

fn status_values(c: &StatusCounts) -> [i64; 7] {
    [
        c.pending,
        c.sending,
        c.sent,
        c.processed,
        c.failed,
        c.skipped_no_account,
        c.skipped_expired,
    ]
}

fn add(a: &mut StatusCounts, b: &StatusCounts) {
    a.pending += b.pending;
    a.sending += b.sending;
    a.sent += b.sent;
    a.processed += b.processed;
    a.failed += b.failed;
    a.skipped_no_account += b.skipped_no_account;
    a.skipped_expired += b.skipped_expired;
}

fn sum(c: &StatusCounts) -> i64 {
    status_values(c).iter().sum()
}

fn counts_json(c: &StatusCounts) -> serde_json::Map<String, Value> {
    EVENT_STATUSES
        .iter()
        .zip(status_values(c))
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

/// `getGoogleAdsStats` input (validated by the route).
#[derive(Debug, Clone, Default)]
pub struct StatsInput {
    pub workspace_id: i64,
    pub from: String,
    pub to: String,
    pub tz: String,
    pub channel: Option<String>,
    pub conversion_action_id: Option<String>,
}

/// `getGoogleAdsStats`: the `googleAdsStatsResponse` document.
pub async fn get_stats(db: &Db, input: StatsInput) -> Result<Value> {
    let range = parse_analytics_date_range(&input.from, &input.to, &input.tz, now_ms());
    let filters = StatsFilters {
        workspace_id: input.workspace_id,
        channel: input.channel,
        conversion_action_id: input.conversion_action_id,
        since: range.since,
        until: range.until,
    };
    let timezone = range.timezone.clone();
    let (day_rows, action_rows, value_rows) = tx::read(db, move |conn| {
        Ok((
            events::stats_by_day_and_channel(conn, &filters, &timezone)?,
            events::stats_by_action(conn, &filters, BY_ACTION_LIMIT as i64)?,
            events::confirmed_value_by_currency(conn, &filters)?,
        ))
    })
    .await?;

    let dates = day_keys(&range.from, &range.to);
    let mut by_date: Vec<(String, StatusCounts)> = dates
        .into_iter()
        .map(|d| (d, StatusCounts::default()))
        .collect();
    let mut by_channel: Vec<(String, StatusCounts)> = Vec::new();
    let mut totals = StatusCounts::default();
    let mut failures = [0i64; 4];
    for row in &day_rows {
        // A day outside the enumerated keys is dropped from every block (as the original).
        let Some(day) = by_date.iter_mut().find(|(d, _)| *d == row.date) else {
            continue;
        };
        add(&mut day.1, &row.counts);
        add(&mut totals, &row.counts);
        match by_channel.iter_mut().find(|(c, _)| *c == row.channel) {
            Some(entry) => add(&mut entry.1, &row.counts),
            None => by_channel.push((row.channel.clone(), row.counts)),
        }
        failures[0] += row.failed_delivery;
        failures[1] += row.failed_processing;
        failures[2] += row.failed_timeout;
        failures[3] += row.failed_unknown;
    }
    by_channel.sort_by(|a, b| sum(&b.1).cmp(&sum(&a.1)).then_with(|| a.0.cmp(&b.0)));

    let decided = totals.processed + totals.failed;
    let mut totals_json = counts_json(&totals);
    totals_json.insert("total".into(), json!(sum(&totals)));
    totals_json.insert(
        "deliveryRate".into(),
        if decided == 0 {
            Value::Null
        } else {
            json!(totals.processed as f64 / decided as f64)
        },
    );
    Ok(json!({
        "range": { "from": range.from, "to": range.to, "tz": range.timezone },
        "totals": totals_json,
        "failuresByStage": {
            "delivery": failures[0],
            "processing": failures[1],
            "timeout": failures[2],
            "unknown": failures[3],
        },
        "timeseries": by_date
            .iter()
            .map(|(date, c)| json!({ "date": date, "counts": counts_json(c) }))
            .collect::<Vec<_>>(),
        "byChannel": by_channel
            .iter()
            .map(|(channel, c)| json!({ "channel": channel, "total": sum(c), "counts": counts_json(c) }))
            .collect::<Vec<_>>(),
        "confirmedValue": value_rows
            .iter()
            .filter_map(|r| r.currency.as_ref().map(|currency| {
                json!({ "currency": currency, "value": r.value, "count": r.count })
            }))
            .collect::<Vec<_>>(),
        "byAction": action_rows
            .iter()
            .take(BY_ACTION_LIMIT)
            .map(|r| json!({
                "conversionActionId": r.conversion_action_id,
                "name": r.name,
                "category": r.category,
                "total": sum(&r.counts),
                "counts": counts_json(&r.counts),
            }))
            .collect::<Vec<_>>(),
        "byActionTruncated": action_rows.len() > BY_ACTION_LIMIT,
    }))
}

/// `maskClickId`: `abcd…wxyz`; an id of 8 characters or fewer is masked whole.
pub fn mask_click_id(click_id: &str) -> String {
    let chars: Vec<char> = click_id.chars().collect();
    if chars.len() <= 8 {
        return "…".to_owned();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

fn redact(error: Option<&str>, identifiers: &[Option<&str>]) -> Option<String> {
    let error = error.filter(|e| !e.is_empty())?;
    let mut ids: Vec<&str> = identifiers
        .iter()
        .flatten()
        .copied()
        .filter(|s| !s.is_empty())
        .collect();
    ids.sort_by_key(|s| std::cmp::Reverse(s.encode_utf16().count()));
    Some(
        ids.into_iter()
            .fold(error.to_owned(), |text, id| text.replace(id, REDACTED)),
    )
}

fn delivery(row: &GoogleAdsConversionEvent) -> &'static str {
    match row.status.as_str() {
        "pending" | "sending" => "toSend",
        "sent" | "processed" => "sent",
        "failed" => match row.failure_stage.as_deref() {
            Some("processing" | "timeout") => "sent",
            _ => "unknown",
        },
        _ => "notSent",
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// `toGoogleAdsEventResource`.
pub fn event_resource(row: &GoogleAdsConversionEvent) -> Value {
    // An unreadable snapshot (unknown version, malformed) is treated like none.
    let options = row
        .options
        .as_ref()
        .and_then(|o| parse_google_ads_event_options(&o.0).ok());
    let v2 = options.as_ref().filter(|o| o.version == 2);
    let consent_status = |v: Value| if v.is_null() { json!("notProvided") } else { v };
    json!({
        "id": row.id.to_string(),
        "status": row.status,
        "failureStage": row.failure_stage,
        "processingStatus": row.processing_status,
        "error": redact(row.error.as_deref(), &[
            Some(row.click_id.as_str()),
            Some(row.transaction_id.as_str()),
            row.request_id.as_deref(),
        ]),
        "channel": row.channel,
        "conversionActionId": row.conversion_action_id,
        "conversionActionName": row.conversion_action_name,
        "uploadMethod": row.upload_method,
        "clickIdType": row.click_id_type,
        "maskedClickId": mask_click_id(&row.click_id),
        "occurredAt": iso(row.occurred_at),
        "sentAt": row.sent_at.map(iso),
        "value": row.value,
        "currency": row.currency,
        "identity": options.as_ref().map(|o| json!({
            "mode": to_value(&o.identity.effective_policy),
            "id": o.identity.id,
        })),
        "conversionTimeProvided": options
            .as_ref()
            .is_some_and(|o| to_value(&o.time_source) == json!("provided")),
        "customerMatching": v2.and_then(|o| o.matching.as_ref()).map(|m| {
            let mut fields = Vec::new();
            if m.email.is_some() { fields.push("email"); }
            if m.phone.is_some() { fields.push("phone"); }
            json!({ "status": to_value(&m.status), "fields": fields })
        }),
        "customerProperties": v2
            .and_then(|o| o.customer_properties.as_ref())
            .map(to_value),
        "consentSnapshot": options.as_ref().map(|o| json!({
            "delivery": delivery(row),
            "adUserData": consent_status(to_value(&o.consent.ad_user_data.status)),
            // The legacy upload API has no ad personalization field.
            "adPersonalization": if row.upload_method == "legacy" {
                json!("notSupported")
            } else {
                consent_status(to_value(&o.consent.ad_personalization.status))
            },
        })),
    })
}

/// `listGoogleAdsEventResources` + the public `pageCount` (`perPage` clamped, never rejected).
pub async fn list_events(db: &Db, input: ListInput) -> Result<(Vec<Value>, i64)> {
    let (rows, total, per_page) = list_events_total(db, input).await?;
    let page_count = ((total + per_page - 1) / per_page).max(1);
    Ok((rows, page_count))
}

/// `listGoogleAdsEventResources`: the rows, the total and the clamped page size.
pub async fn list_events_total(db: &Db, mut input: ListInput) -> Result<(Vec<Value>, i64, i64)> {
    input.per_page = input.per_page.min(GOOGLE_ADS_EVENTS_MAX_PER_PAGE);
    let per_page = input.per_page.max(1);
    let result = tx::read(db, move |conn| Ok(events::list_by_workspace(conn, &input)?)).await?;
    Ok((
        result.rows.iter().map(event_resource).collect(),
        result.total,
        per_page,
    ))
}

fn setting_view(source: &GoogleAdsConsentSource) -> Value {
    match source {
        GoogleAdsConsentSource::Variable { template } => {
            json!({ "type": "variable", "template": template })
        }
        other => json!({ "type": to_value(other)["type"], "template": null }),
    }
}

/// `googleAdsSettingsService.getConsent` as the `to_consent_input` settings
/// (`{ adUserData: { type, template }, adPersonalization }`): no row = both
/// `notProvided`; `None` = a row failing the document schema (`invalid`).
pub async fn stored_consent(db: &Db, workspace_id: i64) -> Result<Option<Value>> {
    let row = tx::read(db, move |conn| {
        Ok(google_ads_settings::find_by_workspace_id(
            conn,
            workspace_id,
        )?)
    })
    .await?;
    Ok(match row {
        None => Some(json!({
            "adUserData": { "type": "notProvided", "template": null },
            "adPersonalization": { "type": "notProvided", "template": null },
        })),
        Some(row) => parse_google_ads_settings_document(&row.settings.0)
            .ok()
            .map(|doc| {
                json!({
                    "adUserData": setting_view(&doc.consent.ad_user_data),
                    "adPersonalization": setting_view(&doc.consent.ad_personalization),
                })
            }),
    })
}

/// `getConsent` + `toConsentView`: no row = absent; a row failing the
/// document schema = invalid.
fn consent_view(settings: Option<&chatbotx_db::repo::models::GoogleAdsSettings>) -> Value {
    match settings {
        None => json!({
            "status": "absent",
            "adUserData": { "type": "notProvided", "template": null },
            "adPersonalization": { "type": "notProvided", "template": null },
        }),
        Some(row) => match parse_google_ads_settings_document(&row.settings.0) {
            Ok(doc) => json!({
                "status": "ok",
                "adUserData": setting_view(&doc.consent.ad_user_data),
                "adPersonalization": setting_view(&doc.consent.ad_personalization),
            }),
            Err(_) => json!({ "status": "invalid", "adUserData": null, "adPersonalization": null }),
        },
    }
}

/// `getConsent` for a sender: the stored consent setting (`NOT_PROVIDED`
/// when absent), `None` = an invalid document.
pub async fn settings_page_consent(db: &Db, workspace_id: i64) -> Result<Option<Value>> {
    let settings = tx::read(db, move |conn| {
        Ok(google_ads_settings::find_by_workspace_id(
            conn,
            workspace_id,
        )?)
    })
    .await?;
    Ok(match settings {
        None => Some(json!({
            "adUserData": { "type": "notProvided" },
            "adPersonalization": { "type": "notProvided" },
        })),
        Some(row) => parse_google_ads_settings_document(&row.settings.0)
            .ok()
            .map(|doc| serde_json::to_value(doc.consent).unwrap_or(Value::Null)),
    })
}

/// `loadGoogleAdsSettingsPage` without the session part: `setup`
/// (`toGoogleAdsSettingsView`, no `auth` / tokens) and `consent`.
pub async fn settings_page(db: &Db, workspace_id: i64) -> Result<Value> {
    let settings = tx::read(db, move |conn| {
        Ok(google_ads_settings::find_by_workspace_id(
            conn,
            workspace_id,
        )?)
    })
    .await?;
    let consent = consent_view(settings.as_ref());
    let Some(found) = crate::google_ads_setup::get_setup(db, workspace_id).await? else {
        return Ok(json!({ "setup": null, "consent": consent }));
    };
    let integration = &found.integration;
    let auth = crate::google_ads_setup::auth_of(integration);
    let upload_method = match auth["metadata"]["uploadMethod"].as_str() {
        Some("legacy") => "legacy",
        _ => "dataManager",
    };
    let actions: Vec<Value> = integration
        .conversion_actions
        .as_ref()
        .and_then(|a| a.0.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|a| {
            json!({
                "id": a["id"],
                "name": a["name"],
                "category": a["category"],
                "status": a["status"],
                "countingType": a["countingType"],
                "attributionModel": a.get("attributionModel").cloned().unwrap_or(Value::Null),
                "lookbackDays": a.get("clickThroughLookbackWindowDays").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({
        "setup": {
            "customerId": integration.customer_id,
            "loginCustomerId": integration.login_customer_id,
            "descriptiveName": integration.descriptive_name,
            "currencyCode": integration.currency_code,
            "conversionCustomerId": integration.conversion_customer_id,
            "acceptedCustomerDataTerms": integration.accepted_customer_data_terms,
            "setupError": integration.setup_error,
            "uploadMethod": upload_method,
            "readiness": found.readiness,
            "connectionStatus": found.connection_status.as_deref().unwrap_or("disconnected"),
            "conversionActions": actions,
            "conversionActionsSyncedAt": integration.conversion_actions_synced_at.map(iso),
        },
        "consent": consent,
    }))
}

/// `googleAdsSettingsService.updateConsent`: the consent schema (first issue
/// as the validation error), then the version-1 document.
pub async fn update_consent(db: &Db, workspace_id: i64, consent: &Value) -> Result<Value> {
    let parsed = chatbotx_db::repo::partials::google_ads::parse_google_ads_consent(consent)
        .map_err(|error| {
            let issue = error.issues.first();
            crate::BusinessError::validation(
                issue
                    .map(|i| i.path.as_str())
                    .filter(|p| !p.is_empty())
                    .unwrap_or("consent"),
                issue.map_or("Invalid Google Ads consent".to_string(), |i| {
                    i.message.clone()
                }),
            )
        })?;
    let value = serde_json::to_value(&parsed).unwrap_or(Value::Null);
    let document = chatbotx_db::repo::Json(json!({ "version": 1, "consent": value }));
    tx::write(db, move |conn| {
        google_ads_settings::upsert_settings(conn, workspace_id, &document)?;
        Ok(())
    })
    .await?;
    Ok(value)
}

/// `loadGoogleAdsIntegration`: the credential-free setup plus the stored consent.
pub async fn get_connection(db: &Db, workspace_id: i64) -> Result<Value> {
    let (integration, conn_row, settings) = tx::read(db, move |conn| {
        let integration = integration_google_ads::find_by_workspace_id(conn, workspace_id)?;
        let conn_row = match &integration {
            Some(i) => connection::find_by_integration_id(conn, i.integration_id)?,
            None => None,
        };
        let settings = google_ads_settings::find_by_workspace_id(conn, workspace_id)?;
        Ok((integration, conn_row, settings))
    })
    .await?;

    let consent = consent_view(settings.as_ref());

    let Some(integration) = integration else {
        return Ok(json!({
            "connected": false,
            "readiness": null,
            "customerId": null,
            "descriptiveName": null,
            "currencyCode": null,
            "uploadMethod": null,
            "acceptedCustomerDataTerms": null,
            "setupError": null,
            "conversionActions": [],
            "conversionActionsSyncedAt": null,
            "consent": consent,
        }));
    };
    let active = conn_row.as_ref().is_some_and(|c| {
        ConnectionStatus::parse(&c.status).is_some_and(|s| ACTIVE_CONNECTION_STATUSES.contains(&s))
    });
    let readiness = if !active {
        "needs_reauth"
    } else if integration.conversion_customer_id.is_some()
        && integration.conversion_actions_synced_at.is_some()
    {
        "ready"
    } else {
        "setup_incomplete"
    };
    // `uploadMethodOf`: pinned in `auth.metadata` at connect time; absent = Data Manager.
    let auth = serde_json::to_string(&integration.auth.0)
        .ok()
        .and_then(|raw| open_auth(&raw))
        .unwrap_or(Value::Null);
    let upload_method = match auth["metadata"]["uploadMethod"].as_str() {
        Some("legacy") => "legacy",
        _ => "dataManager",
    };
    let actions: Vec<Value> = integration
        .conversion_actions
        .as_ref()
        .and_then(|a| a.0.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|a| {
            json!({
                "id": a["id"],
                "name": a["name"],
                "category": a["category"],
                "status": a["status"],
                "countingType": a["countingType"],
                "attributionModel": a.get("attributionModel").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({
        "connected": true,
        "readiness": readiness,
        "customerId": integration.customer_id,
        "descriptiveName": integration.descriptive_name,
        "currencyCode": integration.currency_code,
        "uploadMethod": upload_method,
        "acceptedCustomerDataTerms": integration.accepted_customer_data_terms,
        "setupError": integration.setup_error,
        "conversionActions": actions,
        "conversionActionsSyncedAt": integration.conversion_actions_synced_at.map(iso),
        "consent": consent,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_redacts() {
        assert_eq!(mask_click_id("abcdefgh12"), "abcd…gh12");
        assert_eq!(mask_click_id("abcdefgh"), "…");
        assert_eq!(
            redact(Some("x tx-1 tx-12 y"), &[Some("tx-1"), Some("tx-12"), None]).as_deref(),
            Some("x [redacted] [redacted] y")
        );
        assert_eq!(redact(None, &[]), None);
    }
}
