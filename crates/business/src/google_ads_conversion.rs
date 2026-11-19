//! Q.D7: the Google Ads conversion record and delivery
//! (`packages/business/src/google-ads/{record-conversion,delivery,consent,
//! customer-properties,customer-matching,transaction-id,timing,login-account,
//! send-queue}.ts`) and the two upload transports
//! (`integrations/google-ads/src/apis/{data-manager,legacy-upload}.ts`,
//! `lib/upload-errors.ts`).

use chatbotx_core::utils::google_click as click;
use chatbotx_core::utils::meta_capi::{parse_meta_capi_currency, parse_meta_capi_value};
use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::{self, FindLatestAdTouchInput};
use chatbotx_db::repo::google_ads_conversion_event::{
    self as events, ClaimInput, EventRef, FinishSendingInput, FinishSendingProcessedInput,
};
use chatbotx_db::repo::models::GoogleAdsConversionEvent as Event;
use chatbotx_db::repo::{contact_inbox, iso, new_id, now_ms, patch};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::events::emit::EventBuses;
use chatbotx_jobs::queues::integration::{
    IntegrationJob, IntegrationJobAction, enqueue_integration_job,
};
use chatbotx_jobs::{Enqueuer, JobOptions};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::connection::credentials::Upstream;
use crate::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use crate::errors::{BusinessError, Result};
use crate::google_ads_api::{self as api, Creds, Failure, GoogleError};
use crate::google_ads_setup::{self as setup, Setup, TokenFailure};
use crate::tx;

// ---- timing.ts ------------------------------------------------------------------

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;
pub const MIN_CLICK_AGE_MS: i64 = 6 * HOUR_MS;
pub const MAX_CLICK_AGE_DAYS: i64 = 90;
pub const REDRIVE_DELAY_MS: i64 = HOUR_MS;
pub const PROCESSING_REDRIVE_DELAY_MS: i64 = 6 * HOUR_MS;
pub const MAX_REDRIVE_GENERATIONS: i64 = 30;
pub const FIRST_PROCESSING_CHECK_MS: i64 = 3 * HOUR_MS;

/// `computeSendDelayMs`: Google refuses a click younger than 6 hours.
pub fn send_delay_ms(received_at: i64, now: i64) -> i64 {
    (received_at + MIN_CLICK_AGE_MS - now).max(0)
}

/// `isConversionExpired` (advisory; Google stays authoritative).
pub fn is_expired(event: &Event, now: i64) -> bool {
    is_conversion_expired(
        event.occurred_at,
        event.google_click_received_at,
        event.lookback_window_days,
        now,
    )
}

/// `isConversionExpired`: A = the click is over 90 days old at delivery;
/// B = the conversion is later than the lookback (`None` = 90 days) after it.
pub fn is_conversion_expired(
    occurred_at: i64,
    received_at: i64,
    lookback_window_days: Option<i64>,
    now: i64,
) -> bool {
    let lookback = lookback_window_days.unwrap_or(MAX_CLICK_AGE_DAYS) * DAY_MS;
    now - received_at > MAX_CLICK_AGE_DAYS * DAY_MS || occurred_at - received_at > lookback
}

// ---- transaction-id.ts / send-queue.ts --------------------------------------------

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `buildTransactionId`: `gads-v2-{action}-{c|i|e}-{sha256(ws:customer:key)[..32]}`.
pub fn transaction_id(
    workspace_id: i64,
    conversion_customer_id: &str,
    conversion_action_id: &str,
    segment: &str,
    key: &str,
) -> String {
    let hash = sha256_hex(&format!("{workspace_id}:{conversion_customer_id}:{key}"));
    format!("gads-v2-{conversion_action_id}-{segment}-{}", &hash[..32])
}

/// `sendJobId`.
pub fn send_job_id(event_id: i64, attempt: i64) -> String {
    format!("google-ads-send-{event_id}-a{attempt}")
}

/// `enqueueSend`. `created == false` = a job with this id is still kept.
pub(crate) async fn enqueue_send(
    jobs: &dyn Enqueuer,
    event: &Event,
    delay_ms: i64,
) -> Result<bool> {
    let added = enqueue_integration_job(
        jobs,
        IntegrationJob {
            action: IntegrationJobAction::SendGoogleAdsConversion,
            data: json!({
                "googleAdsConversionEventId": event.id.to_string(),
                "workspaceId": event.workspace_id.to_string(),
                "attempt": event.attempt,
            }),
        },
        Some(JobOptions {
            job_id: Some(send_job_id(event.id, event.attempt)),
            delay_ms: Some(u64::try_from(delay_ms.max(0)).unwrap_or_default()),
            ..JobOptions::default()
        }),
    )
    .await
    .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    Ok(added.created)
}

// ---- consent.ts / customer-properties.ts / customer-matching.ts -------------------

const UNRESOLVED: &str = "{{";

/// `googleAdsConsentInputSchema.parse`: only the snapshot shape is kept.
fn parse_consent(consent: &Value) -> Option<Value> {
    let entry = |key: &str| {
        let e = &consent[key];
        let status = match &e["status"] {
            Value::Null => Value::Null,
            Value::String(s) if s == "granted" || s == "denied" => json!(s),
            _ => return None,
        };
        let source = e["source"].as_str()?;
        ["notProvided", "fixed", "variable"]
            .contains(&source)
            .then(|| json!({ "status": status, "source": source }))
    };
    Some(
        json!({ "adUserData": entry("adUserData")?, "adPersonalization": entry("adPersonalization")? }),
    )
}

/// `toConsentInput`: the workspace consent setting (`notProvided` /
/// `granted` / `denied` / `variable`) with the resolved variable text.
/// `Err(setting)` = a variable resolved to something else.
pub fn to_consent_input(
    settings: &Value,
    resolved: &serde_json::Map<String, Value>,
) -> std::result::Result<Value, &'static str> {
    let mut out = serde_json::Map::new();
    for setting in ["adUserData", "adPersonalization"] {
        let entry = match settings[setting]["type"].as_str() {
            Some("notProvided") => json!({ "status": null, "source": "notProvided" }),
            Some("granted") => json!({ "status": "granted", "source": "fixed" }),
            Some("denied") => json!({ "status": "denied", "source": "fixed" }),
            Some("variable") => {
                let text = resolved
                    .get(setting)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase();
                if text.is_empty() || text.contains(UNRESOLVED) {
                    json!({ "status": null, "source": "variable" })
                } else if text == "granted" || text == "denied" {
                    json!({ "status": text, "source": "variable" })
                } else {
                    return Err(setting);
                }
            }
            _ => return Err(setting),
        };
        out.insert(setting.into(), entry);
    }
    Ok(Value::Object(out))
}

/// `consentForTransport(..).sent`: legacy uploads carry no `adPersonalization`.
fn consent_sent(consent: &Value, method: &str) -> (Option<String>, Option<String>) {
    let status = |key: &str| consent[key]["status"].as_str().map(str::to_string);
    let personalization = if method == "dataManager" {
        status("adPersonalization")
    } else {
        None
    };
    (status("adUserData"), personalization)
}

fn normalise_property(raw: Option<&str>) -> Option<String> {
    let text = raw.unwrap_or_default().trim();
    (!text.is_empty() && !text.contains(UNRESOLVED)).then(|| text.to_uppercase())
}

/// `parseCustomerProperties`: `Err` = a set value outside the allowed ones.
fn parse_customer_properties(
    customer_type: Option<&str>,
    bucket: Option<&str>,
) -> std::result::Result<(Option<String>, Option<String>), ()> {
    let kind = normalise_property(customer_type);
    let bucket = normalise_property(bucket);
    if kind
        .as_deref()
        .is_some_and(|k| !["NEW", "RETURNING"].contains(&k))
        || bucket
            .as_deref()
            .is_some_and(|b| !["LOW", "MEDIUM", "HIGH"].contains(&b))
    {
        return Err(());
    }
    Ok((kind, bucket))
}

/// The snapshot status of an enrichment (`buildMatchingSnapshot` /
/// `buildCustomerPropertiesSnapshot`).
fn enrichment_status(method: &str, ad_user_data: Option<&str>) -> &'static str {
    if method != "dataManager" {
        "unsupportedTransport"
    } else if ad_user_data == Some("granted") {
        "enabled"
    } else {
        "withheldConsent"
    }
}

/// `resolveLoginAccountId`.
pub fn login_account_id(
    customer_id: &str,
    login_customer_id: Option<&str>,
    conversion_customer_id: &str,
) -> String {
    let fallback = if conversion_customer_id == customer_id {
        customer_id
    } else {
        conversion_customer_id
    };
    login_customer_id.unwrap_or(fallback).to_string()
}

// ---- record-conversion.ts ---------------------------------------------------------

/// `RecordConversionInput` (the producer resolved every template).
#[derive(Debug, Clone, Default)]
pub struct RecordInput {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    /// `flowStep` | `trigger`.
    pub source: String,
    pub scope_id: String,
    pub conversion_action_id: String,
    pub value: Option<String>,
    pub currency: Option<String>,
    pub recorded_at: i64,
    /// `click` | `id` | `event`.
    pub dedup_mode: String,
    pub dedup_id: Option<String>,
    pub occurrence_key: Option<String>,
    pub conversion_time: Option<String>,
    /// `toConsentInput` output.
    pub consent: Value,
    /// The step's `{{variable}}`s, unresolved (configuration only).
    pub match_email: Option<String>,
    pub match_phone: Option<String>,
    pub customer_type: Option<String>,
    pub customer_value_bucket: Option<String>,
}

/// `RecordConversionResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    Queued(Box<Event>),
    /// `noClick`, `unsupportedChannel`, `noAccount`, ... (no row is written).
    Refused(&'static str),
    /// `invalidConversionTime` with `malformed` | `future`.
    InvalidConversionTime(&'static str),
}

fn blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

struct Fields {
    value: Option<String>,
    currency: Option<String>,
    /// (`click` | `id` | `event`, the segment key, the stored id).
    identity: (&'static str, Option<String>),
    customer_properties: (Option<String>, Option<String>),
    conversion_time: Option<i64>,
    consent: Value,
}

fn validate(input: &RecordInput, now: i64) -> std::result::Result<Fields, Recorded> {
    let refuse = Recorded::Refused;
    let (value, currency) = match (
        blank(input.value.as_deref()),
        blank(input.currency.as_deref()),
    ) {
        (None, None) => (None, None),
        (Some(v), Some(c)) => (
            Some(parse_meta_capi_value(v).map_err(|_| refuse("invalidValue"))?),
            Some(parse_meta_capi_currency(c).map_err(|_| refuse("invalidValue"))?),
        ),
        _ => return Err(refuse("invalidValue")),
    };
    let identity = match input.dedup_mode.as_str() {
        "click" => ("click", None),
        "event" => {
            let key = input.occurrence_key.as_deref().unwrap_or_default().trim();
            if key.is_empty() {
                return Err(refuse("missingOccurrenceKey"));
            }
            ("event", Some(key.to_string()))
        }
        _ => {
            let id = input.dedup_id.as_deref().unwrap_or_default().trim();
            if id.is_empty() || id.contains(UNRESOLVED) {
                return Err(refuse("missingDedupId"));
            }
            if id.chars().count() > 64 {
                return Err(refuse("invalidDedupId"));
            }
            ("id", Some(id.to_string()))
        }
    };
    let customer_properties = parse_customer_properties(
        input.customer_type.as_deref(),
        input.customer_value_bucket.as_deref(),
    )
    .map_err(|()| refuse("invalidCustomerProperty"))?;
    let conversion_time = match blank(input.conversion_time.as_deref()) {
        None => None,
        Some(text) => {
            let time = click::parse_conversion_time(text.trim())
                .ok_or(Recorded::InvalidConversionTime("malformed"))?;
            if time > now {
                return Err(Recorded::InvalidConversionTime("future"));
            }
            Some(time)
        }
    };
    Ok(Fields {
        value,
        currency,
        identity,
        customer_properties,
        conversion_time,
        consent: parse_consent(&input.consent).unwrap_or(Value::Null),
    })
}

/// The inbox's Google click: (`gclid` | `gbraid`, id, receipt ms, channel).
async fn attribution(
    db: &Db,
    input: &RecordInput,
) -> Result<std::result::Result<(&'static str, String, i64, String), Recorded>> {
    let (ws, cib) = (input.workspace_id, input.contact_inbox_id);
    let Some(row) = tx::read(db, move |conn| {
        Ok(contact_inbox::find_google_click_attribution(conn, ws, cib)?)
    })
    .await?
    else {
        return Ok(Err(Recorded::Refused("noClick")));
    };
    if !click::GOOGLE_ADS_CHANNEL_VALUES.contains(&row.channel.as_str()) {
        return Ok(Err(Recorded::Refused("unsupportedChannel")));
    }
    let referral = row.referral.map(|r| r.0).unwrap_or(Value::Null);
    let received = referral["googleClickReceivedAt"]
        .as_str()
        .and_then(click::parse_conversion_time);
    let found = click::read_google_click(Some(&referral));
    let (Some((kind, id)), Some(received)) = (found, received) else {
        return Ok(Err(Recorded::Refused("noClick")));
    };
    let kind = match kind {
        click::GoogleClickIdType::Gclid => "gclid",
        click::GoogleClickIdType::Gbraid => "gbraid",
    };
    Ok(Ok((kind, id, received, row.channel)))
}

fn action_text<'a>(action: &'a Value, key: &str) -> &'a str {
    action[key].as_str().unwrap_or_default()
}

/// `recordGoogleAdsConversion`: one conversion for delivery. Every refusal
/// is typed and writes nothing; a repeat of the same identity returns the
/// existing event (re-enqueued when its job is gone).
pub async fn record(db: &Db, jobs: &dyn Enqueuer, input: RecordInput) -> Result<Recorded> {
    let fields = match validate(&input, now_ms()) {
        Ok(fields) => fields,
        Err(refusal) => return Ok(refusal),
    };
    if fields.consent.is_null() {
        return Err(BusinessError::exception(
            "Google Ads consent snapshot is malformed",
            "internalServerError",
            500,
        ));
    }
    let (click_type, click_id, received_at, channel) = match attribution(db, &input).await? {
        Ok(found) => found,
        Err(refusal) => return Ok(refusal),
    };
    let Some(Setup {
        integration,
        readiness: "ready",
        ..
    }) = setup::get_setup(db, input.workspace_id).await?
    else {
        return Ok(Recorded::Refused("noAccount"));
    };
    let Some(conversion_customer) = integration.conversion_customer_id.clone() else {
        return Ok(Recorded::Refused("noAccount"));
    };
    let actions = integration
        .conversion_actions
        .as_ref()
        .map(|a| a.0.clone())
        .unwrap_or(Value::Null);
    let Some(action) = actions
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"] == input.conversion_action_id.as_str())
        .cloned()
    else {
        return Ok(Recorded::Refused("unknownConversionAction"));
    };
    if !click::is_enabled_conversion_action(action_text(&action, "status")) {
        return Ok(Recorded::Refused("actionDisabled"));
    }
    if click::is_external_attribution_action(action["attributionModel"].as_str()) {
        return Ok(Recorded::Refused("unsupportedAction"));
    }
    if click_type == "gbraid"
        && click::is_one_per_click_action(action_text(&action, "countingType"))
    {
        return Ok(Recorded::Refused("incompatibleAction"));
    }

    let method = api::upload_method_of(&setup::auth_of(&integration));
    let (mode, key) = fields.identity.clone();
    let segment = match mode {
        "id" => "i",
        "event" => "e",
        _ => "c",
    };
    let transaction = transaction_id(
        input.workspace_id,
        &conversion_customer,
        &input.conversion_action_id,
        segment,
        key.as_deref().unwrap_or(&click_id),
    );
    let options = options_snapshot(&input, &fields, method);
    let mut values: patch::Patch = vec![
        ("workspaceId", Sql::Integer(input.workspace_id)),
        ("integrationGoogleAdsId", Sql::Integer(integration.id)),
        ("contactInboxId", Sql::Integer(input.contact_inbox_id)),
        ("customerId", Sql::Text(integration.customer_id.clone())),
        (
            "loginCustomerId",
            integration
                .login_customer_id
                .clone()
                .map_or(Sql::Null, Sql::Text),
        ),
        ("conversionCustomerId", Sql::Text(conversion_customer)),
        (
            "conversionActionId",
            Sql::Text(input.conversion_action_id.clone()),
        ),
        (
            "conversionActionName",
            action["name"]
                .as_str()
                .map_or(Sql::Null, |n| Sql::Text(n.into())),
        ),
        (
            "conversionActionCategory",
            Sql::Text(action["category"].as_str().unwrap_or("DEFAULT").into()),
        ),
        (
            "lookbackWindowDays",
            action["clickThroughLookbackWindowDays"]
                .as_f64()
                .map_or(Sql::Null, |d| Sql::Integer(d as i64)),
        ),
        ("channel", Sql::Text(channel)),
        ("source", Sql::Text(input.source.clone())),
        ("scopeId", Sql::Text(input.scope_id.clone())),
        ("clickIdType", Sql::Text(click_type.into())),
        ("clickId", Sql::Text(click_id.clone())),
        ("googleClickReceivedAt", Sql::Integer(received_at)),
        (
            "occurredAt",
            Sql::Integer(fields.conversion_time.unwrap_or(input.recorded_at)),
        ),
        ("value", fields.value.clone().map_or(Sql::Null, Sql::Text)),
        (
            "currency",
            fields.currency.clone().map_or(Sql::Null, Sql::Text),
        ),
        ("transactionId", Sql::Text(transaction.clone())),
        ("options", patch::json(&options)),
        ("uploadMethod", Sql::Text(method.into())),
        ("status", Sql::Text("pending".into())),
        ("attempt", Sql::Integer(0)),
        ("processingAttempts", Sql::Integer(0)),
    ];
    let (ws, cib, touch_click) = (input.workspace_id, input.contact_inbox_id, click_id.clone());
    let now = now_ms();
    let outcome = tx::write(db, move |conn| {
        let touch = contact_referral_event::find_latest_ad_touch_id(
            conn,
            &FindLatestAdTouchInput {
                workspace_id: ws,
                contact_inbox_id: cib,
                provider: "google".into(),
                click_id: Some(touch_click),
                ad_id: None,
            },
        )?;
        values.push(("referralEventId", touch.map_or(Sql::Null, Sql::Integer)));
        if let Some(inserted) = events::insert_ignore_duplicate(conn, &values)? {
            return Ok((true, Some(inserted)));
        }
        Ok((
            false,
            events::find_by_transaction_id(conn, ws, &transaction)?,
        ))
    })
    .await
    .map_err(|e| sanitized(&e, &click_id))?;
    match outcome {
        (true, Some(event)) => {
            enqueue_send(
                jobs,
                &event,
                send_delay_ms(event.google_click_received_at, now),
            )
            .await
            .map_err(|e| sanitized(&e, &click_id))?;
            Ok(Recorded::Queued(Box::new(event)))
        }
        (_, Some(event)) => {
            // `recoverUnscheduledEvent`.
            // ponytail: no job-state lookup in the Rust queue: the same job id is
            // added again (a no-op while that job is kept, whatever its state).
            if event.status == "pending"
                && event.claim_token.is_none()
                && event.attempt < MAX_REDRIVE_GENERATIONS
            {
                enqueue_send(
                    jobs,
                    &event,
                    send_delay_ms(event.google_click_received_at, now),
                )
                .await
                .map_err(|e| sanitized(&e, &click_id))?;
            }
            Ok(Recorded::Queued(Box::new(event)))
        }
        _ => Ok(Recorded::Refused("noClick")),
    }
}

/// A driver error can echo the bound click id: only its sanitized text leaves.
fn sanitized(error: &BusinessError, click_id: &str) -> BusinessError {
    BusinessError::exception(
        api::sanitize(&error.to_string(), &[click_id]),
        "internalServerError",
        500,
    )
}

/// `buildOptions`: v2 only when an enrichment is configured.
fn options_snapshot(input: &RecordInput, fields: &Fields, method: &str) -> Value {
    let (mode, key) = &fields.identity;
    let ad_user_data = fields.consent["adUserData"]["status"].as_str();
    let mut options = json!({
        "version": 1,
        "identity": {
            "version": 1,
            "configuredPolicy": mode,
            "effectivePolicy": mode,
            "keySource": match *mode { "id" => "explicit", "event" => "occurrence", _ => "click" },
            "id": if *mode == "id" { json!(key) } else { Value::Null },
        },
        "timeSource": if fields.conversion_time.is_some() { "provided" } else { "recorded" },
        "consent": fields.consent,
    });
    let template = |t: &Option<String>| {
        t.as_deref()
            .map(str::trim)
            .filter(|t| click::is_google_ads_match_template(t))
            .map(str::to_string)
    };
    let (email, phone) = (template(&input.match_email), template(&input.match_phone));
    if email.is_some() || phone.is_some() {
        options["version"] = json!(2);
        options["matching"] = json!({
            "status": enrichment_status(method, ad_user_data),
            "email": email,
            "phone": phone,
        });
    }
    let (kind, bucket) = &fields.customer_properties;
    if kind.is_some() || bucket.is_some() {
        options["version"] = json!(2);
        options["customerProperties"] = json!({
            "status": enrichment_status(method, ad_user_data),
            "customerType": kind,
            "customerValueBucket": bucket,
        });
    }
    options
}

// ---- the upload transports ------------------------------------------------------

/// `GoogleAdsIngestOutcome`.
#[derive(Debug, Clone, PartialEq)]
enum Ingest {
    /// Data Manager: accepted, Google processes it later.
    Accepted(Option<String>),
    /// Legacy: recorded synchronously (`legacy:{jobId}`).
    Completed(String),
    Duplicate,
    /// `tooRecent` | `transient`.
    Retry(&'static str),
}

struct Conversion<'a> {
    event: &'a Event,
    method: &'a str,
    login_account: String,
    consent: (Option<String>, Option<String>),
    customer_properties: Option<Value>,
    /// `{ emailAddress?, phoneNumber? }` SHA-256 hex digests.
    identifiers: Option<Value>,
}

fn invalid_id(label: &str) -> Failure {
    Failure::Google(GoogleError::terminal(
        400,
        "invalidIdentifier",
        &format!("Google Ads {label} is malformed"),
    ))
}

fn checked_action_id(id: &str) -> std::result::Result<&str, Failure> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(invalid_id("conversion action id"))
    }
}

fn malformed(what: &str) -> Failure {
    Failure::Google(GoogleError::terminal(
        502,
        "malformedResponse",
        &format!("{what} could not be read"),
    ))
}

/// `Number(event.value)`.
fn value_number(value: &str) -> Value {
    value
        .parse::<i64>()
        .map(Value::from)
        .or_else(|_| value.parse::<f64>().map(Value::from))
        .unwrap_or(Value::Null)
}

fn grant(status: &str, granted: &str, denied: &str) -> String {
    if status == "granted" { granted } else { denied }.to_string()
}

/// `dataManagerIngestEvent` (`events:ingest`).
async fn ingest_data_manager(
    up: &impl Upstream,
    access_token: &str,
    c: &Conversion<'_>,
) -> std::result::Result<Ingest, Failure> {
    let e = c.event;
    let account = |id: &str, label: &str| -> std::result::Result<Value, Failure> {
        api::checked_id(id, label).map_err(|_| invalid_id(label))?;
        Ok(json!({ "accountType": "GOOGLE_ADS", "accountId": id }))
    };
    let mut event = json!({
        "transactionId": e.transaction_id,
        "eventTimestamp": iso(e.occurred_at),
        "adIdentifiers": { e.click_id_type.clone(): e.click_id },
        "eventSource": "MESSAGE",
    });
    if let Some(value) = &e.value {
        event["conversionValue"] = value_number(value);
    }
    if let Some(currency) = &e.currency {
        event["currency"] = json!(currency);
    }
    let mut consent = serde_json::Map::new();
    if let Some(s) = &c.consent.0 {
        consent.insert(
            "adUserData".into(),
            json!(grant(s, "CONSENT_GRANTED", "CONSENT_DENIED")),
        );
    }
    if let Some(s) = &c.consent.1 {
        consent.insert(
            "adPersonalization".into(),
            json!(grant(s, "CONSENT_GRANTED", "CONSENT_DENIED")),
        );
    }
    if !consent.is_empty() {
        event["consent"] = Value::Object(consent);
    }
    if let Some(identifiers) = &c.identifiers {
        let list: Vec<Value> = ["emailAddress", "phoneNumber"]
            .iter()
            .filter_map(|key| identifiers.get(*key).map(|v| json!({ *key: v })))
            .collect();
        if !list.is_empty() {
            event["userData"] = json!({ "userIdentifiers": list });
        }
    }
    if let Some(properties) = &c.customer_properties {
        event["userProperties"] = properties.clone();
    }
    let user_data = event.get("userData").is_some();
    let mut body = json!({
        "destinations": [{
            "loginAccount": account(&c.login_account, "login customer id")?,
            "operatingAccount": account(&e.conversion_customer_id, "operating customer id")?,
            "productDestinationId": checked_action_id(&e.conversion_action_id)?,
        }],
        "events": [event],
    });
    // Required by Google whenever userData is sent; the digests are hex.
    if user_data {
        body["encoding"] = json!("HEX");
    }
    let reply = up
        .send(crate::connection::credentials::UpstreamRequest {
            method: "POST",
            url: format!("{}events:ingest", api::DATA_MANAGER_API),
            headers: vec![("authorization".into(), format!("Bearer {access_token}"))],
            body: Some(body.to_string()),
        })
        .await;
    let answer = api::rescue(reply)?;
    if !answer.is_object() {
        return Err(malformed("Data Manager ingest response"));
    }
    log_field_warnings(e, &answer["fieldWarnings"]);
    Ok(Ingest::Accepted(
        answer["requestId"].as_str().map(str::to_string),
    ))
}

/// Data Manager `FieldWarning`: only the field path and reason code are
/// logged (`description` can echo the value sent).
fn log_field_warnings(event: &Event, warnings: &Value) {
    let Some(list) = warnings.as_array().filter(|l| !l.is_empty()) else {
        return;
    };
    let secrets = [event.click_id.as_str()];
    let summary: Vec<Value> = list
        .iter()
        .take(10)
        .map(|w| {
            json!({
                "field": w["field"].as_str().map(|f| api::sanitize(f, &secrets)),
                "reason": w["reason"].as_str().map(|r| api::sanitize(r, &secrets)),
            })
        })
        .collect();
    let warnings = serde_json::Value::from(summary).to_string();
    tracing::warn!(
        event_id = event.id,
        workspace_id = event.workspace_id,
        warning_count = list.len(),
        warnings,
        "google ads: Data Manager returned field warnings"
    );
}

const DUPLICATE_REASONS: &[&str] = &["CLICK_CONVERSION_ALREADY_EXISTS", "ORDER_ID_ALREADY_IN_USE"];
const TOO_RECENT_REASONS: &[&str] = &["TOO_RECENT_EVENT", "TOO_RECENT_CONVERSION_ACTION"];
const TRANSIENT_REASONS: &[&str] = &[
    "RESOURCE_EXHAUSTED",
    "RESOURCE_TEMPORARILY_EXHAUSTED",
    "INTERNAL_ERROR",
    "TRANSIENT_ERROR",
    "CONCURRENT_DATA_MODIFICATION",
    "DEADLINE_EXCEEDED",
];

/// `(category, reason)` of every `details[].errors[].errorCode` entry.
fn upload_errors(error: &Value) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for detail in error["details"].as_array().into_iter().flatten() {
        for item in detail["errors"].as_array().into_iter().flatten() {
            for (category, value) in item["errorCode"].as_object().into_iter().flatten() {
                if let Some(reason) = value.as_str() {
                    found.push((category.clone(), reason.to_string()));
                }
            }
        }
    }
    found
}

/// `classifyUploadErrors` + `fromFailure`.
fn from_upload_failure(error: &Value, status: u16) -> std::result::Result<Ingest, Failure> {
    let found = upload_errors(error);
    if found.is_empty() {
        let transient = error["code"]
            .as_i64()
            .is_some_and(|c| [4, 8, 13, 14].contains(&c))
            || error["status"].as_str().is_some_and(|s| {
                [
                    "DEADLINE_EXCEEDED",
                    "RESOURCE_EXHAUSTED",
                    "INTERNAL",
                    "UNAVAILABLE",
                ]
                .contains(&s)
            });
        return if transient {
            Ok(Ingest::Retry("transient"))
        } else {
            Err(malformed("Google Ads upload response"))
        };
    }
    if found.iter().any(|(_, r)| r == api::NOT_ALLOWLISTED) {
        return Err(Failure::Google(GoogleError::terminal(
            403,
            api::NOT_ALLOWLISTED,
            "Google does not allow the legacy upload API for this customer",
        )));
    }
    let permanent: Vec<&(String, String)> = found
        .iter()
        .filter(|(category, reason)| {
            !(DUPLICATE_REASONS.contains(&reason.as_str())
                || TOO_RECENT_REASONS.contains(&reason.as_str())
                || TRANSIENT_REASONS.contains(&reason.as_str())
                || category == "quotaError"
                || category == "internalError")
        })
        .collect();
    if let Some((category, _)) = permanent.first() {
        let reasons: Vec<String> = permanent
            .iter()
            .take(5)
            .map(|(_, r)| {
                let bounded = r.len() <= 80
                    && r.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
                if bounded { r.clone() } else { "UNKNOWN".into() }
            })
            .collect();
        return Err(Failure::Google(GoogleError {
            status,
            reason: reasons.first().cloned(),
            category: Some(category.clone()),
            message: format!("Google rejected the conversion: {}", reasons.join(", ")),
            ..GoogleError::default()
        }));
    }
    if found
        .iter()
        .all(|(_, r)| DUPLICATE_REASONS.contains(&r.as_str()))
    {
        return Ok(Ingest::Duplicate);
    }
    if found
        .iter()
        .any(|(_, r)| TOO_RECENT_REASONS.contains(&r.as_str()))
    {
        return Ok(Ingest::Retry("tooRecent"));
    }
    Ok(Ingest::Retry("transient"))
}

/// `legacyOrderId`.
pub fn legacy_order_id(transaction_id: &str) -> String {
    format!("v1-{}", &sha256_hex(transaction_id)[..40])
}

/// `legacyUploadClickConversion` (`customers/{id}:uploadClickConversions`).
async fn upload_legacy(
    up: &impl Upstream,
    access_token: &str,
    developer_token: Option<&str>,
    c: &Conversion<'_>,
) -> std::result::Result<Ingest, Failure> {
    let e = c.event;
    let operating = api::checked_id(&e.conversion_customer_id, "operating customer id")
        .map_err(|_| invalid_id("operating customer id"))?;
    let login = api::checked_id(&c.login_account, "login customer id")
        .map_err(|_| invalid_id("login customer id"))?;
    let action = checked_action_id(&e.conversion_action_id)?;
    let conversion_action = format!("customers/{operating}/conversionActions/{action}");
    let mut conversion = json!({
        e.click_id_type.clone(): e.click_id,
        "conversionAction": conversion_action,
        "conversionDateTime": format!("{}+00:00", iso(e.occurred_at)[..19].replace('T', " ")),
        "orderId": legacy_order_id(&e.transaction_id),
    });
    if let Some(value) = &e.value {
        conversion["conversionValue"] = value_number(value);
    }
    if let Some(currency) = &e.currency {
        conversion["currencyCode"] = json!(currency);
    }
    if let Some(s) = &c.consent.0 {
        conversion["consent"] = json!({ "adUserData": grant(s, "GRANTED", "DENIED") });
    }
    let reply = up
        .send(crate::connection::credentials::UpstreamRequest {
            method: "POST",
            url: format!(
                "{}customers/{operating}:uploadClickConversions",
                api::ADS_API
            ),
            headers: api::ads_headers(Creds {
                access_token,
                developer_token,
                login_customer_id: (login != operating).then_some(login),
            }),
            body: Some(json!({ "conversions": [conversion], "partialFailure": true }).to_string()),
        })
        .await;
    // `topLevelFailure`: a non-2xx (not 401) whose `error` lists upload errors.
    if let Ok((status, body)) = &reply
        && !(200..300).contains(status)
        && *status != 401
    {
        let envelope: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if !upload_errors(&envelope["error"]).is_empty() {
            return from_upload_failure(&envelope["error"], *status);
        }
    }
    let answer = api::rescue(reply)?;
    if !answer["partialFailureError"].is_null() {
        return from_upload_failure(&answer["partialFailureError"], 400);
    }
    let job_id = api::id_text(&answer["jobId"]).filter(|id| {
        id == "0"
            || (id.len() <= 19 && !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
    });
    let results = answer["results"].as_array();
    let recorded = results.filter(|r| r.len() == 1).is_some_and(|r| {
        r[0][e.click_id_type.as_str()] == e.click_id.as_str()
            || r[0]["conversionAction"] == conversion_action.as_str()
    });
    match job_id {
        Some(id) if recorded => Ok(Ingest::Completed(format!("legacy:{id}"))),
        _ => Err(malformed("Google Ads upload response")),
    }
}

// ---- delivery.ts --------------------------------------------------------------

/// `DeliverInput`.
#[derive(Debug, Clone, Copy)]
pub struct DeliverInput {
    pub event_id: i64,
    pub workspace_id: i64,
    /// The redrive generation the job was enqueued for.
    pub attempt: i64,
    /// The queue will not retry this job after this run.
    pub is_last_in_job_attempt: bool,
}

/// What the delivery needs from the process.
pub struct DeliverDeps<'a, U: Upstream> {
    pub db: &'a Db,
    pub up: &'a U,
    pub jobs: &'a dyn Enqueuer,
    pub events: Option<&'a EventBuses>,
    /// For the platform credential (the optional developer token).
    pub app_url: &'a str,
    /// `resolveTemplates` of customer matching; `None` = click only.
    pub matching: Option<&'a dyn MatchingResolver>,
}

/// The resolved `(email, phone)`; `Ok(None)` = the inbox is gone. An `Err`
/// fails the delivery for a retry (the original's resolver throws).
pub type MatchingValues = std::result::Result<Option<(Option<String>, Option<String>)>, String>;

/// `MatchingTemplateResolver`: the step's e-mail / phone `{{variable}}`s for
/// the event's contact inbox.
pub trait MatchingResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        workspace_id: i64,
        contact_inbox_id: i64,
        email: Option<String>,
        phone: Option<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = MatchingValues> + Send + 'a>>;
}

/// `normalizeEmail`: lower-case, no whitespace; Gmail local parts lose dots
/// and `+tags`.
pub fn normalize_email(raw: &str) -> Option<String> {
    let lowered: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let mut parts = lowered.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let local = if domain == "gmail.com" || domain == "googlemail.com" {
        local
            .replace('.', "")
            .split('+')
            .next()
            .unwrap_or_default()
            .to_string()
    } else {
        local.to_string()
    };
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    // `PLAIN_ADDRESS`: `^[^\s@]+@[^\s@]+\.[^\s@]+$`.
    let dotted = domain
        .char_indices()
        .any(|(i, c)| c == '.' && i > 0 && i + 1 < domain.len());
    dotted.then(|| format!("{local}@{domain}"))
}

fn parse_e164(text: &str) -> Option<String> {
    // `{ extract: false }`: the whole text is the number, never one found in
    // prose. An extension (`ext.`, `x`) is refused anyway.
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_whitespace() || "+()-./".contains(c))
    {
        return None;
    }
    let parsed = phonenumber::parse(None, text).ok()?;
    (parsed.is_valid() && parsed.extension().is_none())
        .then(|| parsed.format().mode(phonenumber::Mode::E164).to_string())
}

/// `normalizePhone`: an E.164 number, also from bare international digits.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let text = raw.trim();
    parse_e164(text).or_else(|| {
        let digits = text.len() >= 8
            && text.len() <= 15
            && !text.starts_with('0')
            && text.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| parse_e164(&format!("+{text}"))).flatten()
    })
}

/// `hashMatchingIdentifiers`: `None` when nothing normalizes.
pub fn hash_matching_identifiers(email: Option<&str>, phone: Option<&str>) -> Option<Value> {
    let mut out = serde_json::Map::new();
    if let Some(email) = email.and_then(normalize_email) {
        out.insert(
            "emailAddress".into(),
            json!(chatbotx_core::utils::crypto::sha256_hex(&email)),
        );
    }
    if let Some(phone) = phone.and_then(normalize_phone) {
        out.insert(
            "phoneNumber".into(),
            json!(chatbotx_core::utils::crypto::sha256_hex(&phone)),
        );
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

/// `loadMatchingIdentifiers`: only an `enabled` snapshot with an inbox.
async fn load_matching_identifiers<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    matching: &Value,
) -> std::result::Result<Option<Value>, String> {
    let (Some(resolver), Some(inbox)) = (deps.matching, event.contact_inbox_id) else {
        return Ok(None);
    };
    if matching["status"] != "enabled" {
        return Ok(None);
    }
    let template = |key: &str| matching[key].as_str().map(str::to_string);
    let values = resolver
        .resolve(
            event.workspace_id,
            inbox,
            template("email"),
            template("phone"),
        )
        .await?;
    Ok(values
        .and_then(|(email, phone)| hash_matching_identifiers(email.as_deref(), phone.as_deref())))
}

pub(crate) struct Lease {
    pub(crate) id: i64,
    pub(crate) workspace_id: i64,
    pub(crate) claim_token: String,
}

fn lost(event: &Event, action: &str) {
    tracing::warn!(
        event_id = event.id,
        workspace_id = event.workspace_id,
        attempt = event.attempt,
        "google ads: lease lost before {action}; another worker owns this generation"
    );
}

fn finish(lease: &Lease, to: &str) -> FinishSendingInput {
    FinishSendingInput {
        id: lease.id,
        workspace_id: lease.workspace_id,
        claim_token: lease.claim_token.clone(),
        to: to.into(),
        ..FinishSendingInput::default()
    }
}

async fn write_finish(db: &Db, input: FinishSendingInput) -> Result<Option<Event>> {
    tx::write(db, move |conn| Ok(events::finish_sending(conn, &input)?)).await
}

/// `failDelivery`.
pub(crate) async fn fail<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    lease: &Lease,
    failure: &Failure,
    log_provider_failure: bool,
) -> Result<()> {
    let message = if failure.reason() == Some(api::NOT_ALLOWLISTED) {
        api::failure_message("legacy_upload_not_allowed").to_string()
    } else {
        failure.public_message(&[&event.click_id])
    };
    let finished = write_finish(
        deps.db,
        FinishSendingInput {
            error: Some(message.clone()),
            failure_stage: Some("delivery".into()),
            ..finish(lease, "failed")
        },
    )
    .await?;
    match finished {
        None => lost(event, "recording the failure"),
        Some(_) if log_provider_failure => {
            let status = match failure {
                Failure::Google(e) => Some(i64::from(e.status)),
                Failure::Auth => Some(401),
                Failure::Network(_) => None,
            };
            log_provider_error(
                deps.events,
                ProviderErrorInput::new(
                    "google-ads",
                    &event.workspace_id.to_string(),
                    Thrown::Error {
                        message,
                        stack: None,
                        status,
                    },
                ),
            )
            .await;
        }
        Some(_) => {}
    }
    Ok(())
}

fn local(message: &str) -> Failure {
    Failure::Google(GoogleError::terminal(500, "delivery", message))
}

/// `deferDelivery`: gives the lease back and schedules the next generation,
/// or fails once the generations run out.
async fn defer<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    lease: &Lease,
    delay_ms: i64,
    reason: &str,
) -> Result<()> {
    if event.attempt >= MAX_REDRIVE_GENERATIONS {
        let failure = local(&format!("Delivery deferred too long: {reason}"));
        return fail(deps, event, lease, &failure, false).await;
    }
    let (r, token, next) = (
        EventRef {
            id: lease.id,
            workspace_id: lease.workspace_id,
        },
        lease.claim_token.clone(),
        event.attempt + 1,
    );
    let released = tx::write(deps.db, move |conn| {
        Ok(events::release_claim(conn, r, &token, next)?)
    })
    .await?;
    match released {
        Some(released) => {
            enqueue_send(deps.jobs, &released, delay_ms).await?;
        }
        None => lost(event, "deferring"),
    }
    Ok(())
}

/// `evaluateDeliveryGuards`.
enum Guard {
    Skip(&'static str),
    Defer(i64, &'static str),
}

fn guard(event: &Event, setup: Option<&Setup>, now: i64) -> Option<Guard> {
    let Some(setup) = setup.filter(|s| {
        s.integration.customer_id == event.customer_id
            && s.integration.conversion_customer_id.as_deref()
                == Some(event.conversion_customer_id.as_str())
    }) else {
        return Some(Guard::Skip("skipped_no_account"));
    };
    if is_expired(event, now) {
        return Some(Guard::Skip("skipped_expired"));
    }
    let scope = setup::auth_of(&setup.integration)["metadata"]["scope"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let lacks_scopes = scope.is_some_and(|scope| {
        !api::missing_scopes(
            &api::parse_scopes(&scope),
            api::upload_method(&json!(event.upload_method)),
        )
        .is_empty()
    });
    if setup.connection_status.as_deref() == Some("needs_reauth") || lacks_scopes {
        return Some(Guard::Defer(REDRIVE_DELAY_MS, "needs_reauth"));
    }
    if setup.readiness != "ready" {
        return Some(Guard::Skip("skipped_no_account"));
    }
    let wait = send_delay_ms(event.google_click_received_at, now);
    (wait > 0).then_some(Guard::Defer(wait, "click_too_recent"))
}

/// Why [`send`] stopped without a Google answer to act on.
enum SendError {
    /// The grant is unusable (`AuthException`): deferred as `needs_reauth`.
    Reauth,
    Failed(Failure),
}

/// `sendToGoogle`.
async fn send<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    lease: &Lease,
    setup: &Setup,
) -> std::result::Result<Result<()>, SendError> {
    let method = api::upload_method(&json!(event.upload_method));
    let options = event.options.as_ref().map(|o| o.0.clone());
    let (consent, customer_properties) = match &options {
        None => ((None, None), None),
        Some(o) if matches!(o["version"].as_i64(), Some(1 | 2)) => {
            let properties = &o["customerProperties"];
            let user_properties = (properties["status"] == "enabled").then(|| {
                let mut p = serde_json::Map::new();
                for key in ["customerType", "customerValueBucket"] {
                    if let Some(v) = properties[key].as_str() {
                        p.insert(key.into(), json!(v));
                    }
                }
                Value::Object(p)
            });
            (consent_sent(&o["consent"], method), user_properties)
        }
        Some(_) => {
            let failure = local("Unsupported conversion options version");
            return Ok(fail(deps, event, lease, &failure, false).await);
        }
    };
    // Only a confirmed `true` sends user data: Google fails an event with user
    // data when the account has not signed the customer-data terms.
    let terms_accepted = setup.integration.accepted_customer_data_terms == Some(true);
    let matching = options.as_ref().map(|o| &o["matching"]);
    let matching_enabled = matching.is_some_and(|m| m["status"] == "enabled");
    let identifiers = match matching {
        Some(matching) if terms_accepted => {
            match load_matching_identifiers(deps, event, matching).await {
                Ok(identifiers) => identifiers,
                Err(error) => {
                    return Ok(Err(BusinessError::exception(
                        error,
                        "internalServerError",
                        500,
                    )));
                }
            }
        }
        _ => None,
    };
    if matching_enabled && !terms_accepted {
        tracing::info!(
            event_id = event.id,
            workspace_id = event.workspace_id,
            "google ads: customer data terms not accepted, customer matching withheld, sent by click only"
        );
    } else if matching_enabled && identifiers.is_none() {
        tracing::info!(
            event_id = event.id,
            workspace_id = event.workspace_id,
            "google ads: customer matching had no usable identifier, sent by click only"
        );
    }
    let stamped = event
        .processing_detail
        .as_ref()
        .and_then(|d| d.0["sendAttemptedAt"].as_str().map(str::to_string));
    let is_replay = stamped.is_some();
    let mut stamp = stamped;
    if method == "legacy" {
        let claim = ClaimInput {
            id: lease.id,
            workspace_id: lease.workspace_id,
            claim_token: lease.claim_token.clone(),
            attempt: event.attempt,
        };
        let marked = tx::write(deps.db, move |conn| {
            Ok(events::mark_send_attempted(conn, &claim, now_ms())?)
        })
        .await;
        match marked {
            Ok(Some(marked)) => {
                stamp = marked
                    .processing_detail
                    .as_ref()
                    .and_then(|d| d.0["sendAttemptedAt"].as_str().map(str::to_string));
            }
            Ok(None) => {
                lost(event, "starting the upload");
                return Ok(Ok(()));
            }
            Err(e) => return Ok(Err(e)),
        }
    }
    let developer_token = if method == "legacy" {
        setup::credential(
            deps.db,
            crate::meta_catalog::process_encryption(),
            deps.app_url,
            event.workspace_id,
        )
        .await
        .ok()
        .flatten()
        .and_then(|c| setup::developer_token(Some(&c)).flatten())
    } else {
        None
    };
    let conversion = Conversion {
        event,
        method,
        login_account: login_account_id(
            &setup.integration.customer_id,
            setup.integration.login_customer_id.as_deref(),
            &event.conversion_customer_id,
        ),
        consent,
        customer_properties,
        identifiers,
    };
    // `buildActionContext` + one forced refresh after a 401.
    let mut outcome = Err(Failure::Auth);
    for force in [false, true] {
        let auth = match setup::fresh_auth(deps.db, deps.up, &setup.integration, force).await {
            Ok(auth) => auth,
            Err(TokenFailure::Revoked) => return Err(SendError::Reauth),
            Err(TokenFailure::Other(message)) => {
                return Err(SendError::Failed(Failure::Network(message)));
            }
        };
        let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
        outcome = if conversion.method == "legacy" {
            upload_legacy(deps.up, token, developer_token.as_deref(), &conversion).await
        } else {
            ingest_data_manager(deps.up, token, &conversion).await
        };
        if outcome != Err(Failure::Auth) {
            break;
        }
    }
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(Failure::Auth) => return Err(SendError::Reauth),
        Err(failure) => return Err(SendError::Failed(failure)),
    };
    Ok(settle(deps, event, lease, outcome, is_replay, stamp).await)
}

/// `handleLegacyOutcome` and the Data Manager acceptance.
async fn settle<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    lease: &Lease,
    outcome: Ingest,
    is_replay: bool,
    stamp: Option<String>,
) -> Result<()> {
    let complete = |request_id: String, duplicate_recovery: bool| {
        let mut detail = json!({
            "requestStatus": "LEGACY_UPLOAD_COMPLETED",
            "recordCount": 1,
            "errorCounts": [],
            "warningCounts": [],
        });
        if let Some(stamp) = &stamp {
            detail["sendAttemptedAt"] = json!(stamp);
        }
        if duplicate_recovery {
            detail["duplicateRecovery"] = json!(true);
        }
        FinishSendingProcessedInput {
            id: lease.id,
            workspace_id: lease.workspace_id,
            claim_token: lease.claim_token.clone(),
            attempt: event.attempt,
            request_id,
            sent_at: now_ms(),
            processing_detail: detail,
        }
    };
    let input = match outcome {
        Ingest::Completed(request_id) => complete(request_id, false),
        Ingest::Duplicate if is_replay => complete("legacy:recovered".into(), true),
        Ingest::Duplicate => {
            let failure = local("Google already holds this conversion");
            return fail(deps, event, lease, &failure, true).await;
        }
        Ingest::Retry(reason) => {
            let delay = if reason == "tooRecent" {
                PROCESSING_REDRIVE_DELAY_MS
            } else {
                REDRIVE_DELAY_MS
            };
            let reason = format!("google_{reason}");
            return defer(deps, event, lease, delay, &reason).await;
        }
        Ingest::Accepted(None) => {
            let failure = local("Data Manager accepted the request without a request id");
            return fail(deps, event, lease, &failure, true).await;
        }
        Ingest::Accepted(Some(request_id)) => {
            let now = now_ms();
            let sent = write_finish(
                deps.db,
                FinishSendingInput {
                    request_id: Some(request_id),
                    sent_at: Some(now),
                    processing_status: Some("processing".into()),
                    next_processing_check_at: Some(now + FIRST_PROCESSING_CHECK_MS),
                    ..finish(lease, "sent")
                },
            )
            .await?;
            if sent.is_none() {
                lost(event, "recording the send");
            }
            return Ok(());
        }
    };
    let done = tx::write(deps.db, move |conn| {
        Ok(events::finish_sending_processed(conn, &input)?)
    })
    .await?;
    if done.is_none() {
        lost(event, "recording the completed upload");
    }
    Ok(())
}

/// `deliverGoogleAdsConversion`: one delivery attempt of one generation.
/// `Err(message)` = a transient failure the queue retries (token-free).
pub async fn deliver<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    input: DeliverInput,
) -> std::result::Result<(), String> {
    let lease = Lease {
        id: input.event_id,
        workspace_id: input.workspace_id,
        claim_token: new_id().to_string(),
    };
    let claim = ClaimInput {
        id: lease.id,
        workspace_id: lease.workspace_id,
        claim_token: lease.claim_token.clone(),
        attempt: input.attempt,
    };
    let event = tx::write(deps.db, move |conn| {
        Ok(events::claim_for_sending(conn, &claim)?)
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(event) = event else {
        return Ok(());
    };
    let error = match run_claimed(deps, &event, &lease).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => Failure::Network(e.to_string()),
        Err(SendError::Reauth) => {
            return defer(deps, &event, &lease, REDRIVE_DELAY_MS, "needs_reauth")
                .await
                .map_err(|e| e.to_string());
        }
        Err(SendError::Failed(failure)) => failure,
    };
    // `deferralOf`.
    let deferral = match &error {
        Failure::Google(e)
            if e.reason
                .as_deref()
                .is_some_and(|r| r.ends_with("CONVERSION_ACTION_TOO_RECENTLY_CREATED")) =>
        {
            Some((PROCESSING_REDRIVE_DELAY_MS, "conversion_action_too_recent"))
        }
        Failure::Google(e) if e.retryable && input.is_last_in_job_attempt => {
            Some((REDRIVE_DELAY_MS, "google_unavailable"))
        }
        _ => None,
    };
    let settled = if let Some((delay, reason)) = deferral {
        defer(deps, &event, &lease, delay, reason).await
    } else if matches!(&error, Failure::Google(e) if !e.retryable) || input.is_last_in_job_attempt {
        fail(deps, &event, &lease, &error, true).await
    } else {
        let (r, token, attempt) = (
            EventRef {
                id: lease.id,
                workspace_id: lease.workspace_id,
            },
            lease.claim_token.clone(),
            event.attempt,
        );
        let released = tx::write(deps.db, move |conn| {
            Ok(events::release_claim(conn, r, &token, attempt)?)
        })
        .await
        .map_err(|e| e.to_string())?;
        if released.is_none() {
            lost(&event, "releasing for retry");
        }
        return Err(error.public_message(&[&event.click_id]));
    };
    settled.map_err(|e| e.to_string())
}

async fn run_claimed<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    lease: &Lease,
) -> std::result::Result<Result<()>, SendError> {
    let setup = match setup::get_setup(deps.db, event.workspace_id).await {
        Ok(setup) => setup,
        Err(e) => return Ok(Err(e)),
    };
    match guard(event, setup.as_ref(), now_ms()) {
        Some(Guard::Skip(status)) => {
            return Ok(match write_finish(deps.db, finish(lease, status)).await {
                Ok(None) => {
                    lost(event, "skipping");
                    Ok(())
                }
                Ok(Some(_)) => Ok(()),
                Err(e) => Err(e),
            });
        }
        Some(Guard::Defer(delay, reason)) => {
            return Ok(defer(deps, event, lease, delay, reason).await);
        }
        None => {}
    }
    let Some(setup) = setup else {
        return Ok(Ok(()));
    };
    send(deps, event, lease, &setup).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_ids_and_order_ids_are_stable() {
        let id = transaction_id(7, "1234567890", "55", "c", "gclid-1");
        assert!(id.starts_with("gads-v2-55-c-"));
        assert_eq!(id.len(), "gads-v2-55-c-".len() + 32);
        assert_eq!(id, transaction_id(7, "1234567890", "55", "c", "gclid-1"));
        assert_ne!(id, transaction_id(8, "1234567890", "55", "c", "gclid-1"));
        assert_eq!(legacy_order_id(&id).len(), 43);
    }

    #[test]
    fn login_account_prefers_the_manager() {
        assert_eq!(login_account_id("1", None, "1"), "1");
        assert_eq!(login_account_id("1", Some("9"), "2"), "9");
        assert_eq!(login_account_id("1", None, "2"), "2");
    }

    #[test]
    fn consent_input_and_transport() {
        let settings = json!({ "adUserData": { "type": "variable", "template": "{{c}}" },
            "adPersonalization": { "type": "granted" } });
        let mut resolved = serde_json::Map::new();
        resolved.insert("adUserData".into(), json!(" Denied "));
        let consent = to_consent_input(&settings, &resolved).expect("valid");
        assert_eq!(
            consent["adUserData"],
            json!({ "status": "denied", "source": "variable" })
        );
        assert_eq!(
            consent_sent(&consent, "legacy"),
            (Some("denied".into()), None)
        );
        assert_eq!(
            consent_sent(&consent, "dataManager"),
            (Some("denied".into()), Some("granted".into()))
        );
        resolved.insert("adUserData".into(), json!("maybe"));
        assert_eq!(to_consent_input(&settings, &resolved), Err("adUserData"));
    }

    #[test]
    fn upload_errors_classify_like_the_original() {
        let error = |reasons: &[&str]| {
            json!({ "details": [{ "errors": reasons.iter()
                .map(|r| json!({ "errorCode": { "conversionUploadError": r } }))
                .collect::<Vec<_>>() }] })
        };
        assert_eq!(
            from_upload_failure(&error(&["CLICK_CONVERSION_ALREADY_EXISTS"]), 400),
            Ok(Ingest::Duplicate)
        );
        assert_eq!(
            from_upload_failure(
                &error(&["TOO_RECENT_EVENT", "ORDER_ID_ALREADY_IN_USE"]),
                400
            ),
            Ok(Ingest::Retry("tooRecent"))
        );
        assert!(matches!(
            from_upload_failure(&error(&["INVALID_GCLID"]), 400),
            Err(Failure::Google(e)) if e.reason.as_deref() == Some("INVALID_GCLID") && !e.retryable
        ));
        assert_eq!(
            from_upload_failure(&json!({ "code": 14 }), 400),
            Ok(Ingest::Retry("transient"))
        );
    }

    #[test]
    fn values_and_properties_validate() {
        let input = RecordInput {
            value: Some("19.99".into()),
            currency: Some("eur".into()),
            dedup_mode: "id".into(),
            dedup_id: Some("{{order}}".into()),
            ..RecordInput::default()
        };
        assert!(matches!(
            validate(&input, 0),
            Err(Recorded::Refused("missingDedupId"))
        ));
        let input = RecordInput {
            dedup_mode: "click".into(),
            value: Some("1".into()),
            ..RecordInput::default()
        };
        assert!(matches!(
            validate(&input, 0),
            Err(Recorded::Refused("invalidValue"))
        ));
        assert_eq!(
            parse_customer_properties(Some(" new "), Some("{{x}}")),
            Ok((Some("NEW".into()), None))
        );
        assert!(parse_customer_properties(Some("vip"), None).is_err());
    }

    /// Port of `google-ads-customer-properties.test.ts`
    /// (`parseCustomerProperties`, `buildCustomerPropertiesSnapshot` status).
    #[test]
    fn customer_properties_parse_and_snapshot_status() {
        assert_eq!(parse_customer_properties(None, None), Ok((None, None)));
        assert_eq!(
            parse_customer_properties(Some("  "), Some("{{contact.tier}}")),
            Ok((None, None))
        );
        assert_eq!(
            parse_customer_properties(Some(" returning "), Some("High")),
            Ok((Some("RETURNING".into()), Some("HIGH".into())))
        );
        for (kind, bucket) in [
            (Some("VIP"), None),
            (Some("REENGAGED"), None),
            (None, Some("GOLD")),
            (Some("NEW"), Some("5")),
        ] {
            assert!(parse_customer_properties(kind, bucket).is_err());
        }
        assert_eq!(enrichment_status("dataManager", Some("granted")), "enabled");
        assert_eq!(
            enrichment_status("dataManager", Some("denied")),
            "withheldConsent"
        );
        assert_eq!(enrichment_status("dataManager", None), "withheldConsent");
        assert_eq!(
            enrichment_status("legacy", Some("granted")),
            "unsupportedTransport"
        );
    }
}
