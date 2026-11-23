//! The Meta conversion send path: `metaConversionsService.enqueueEvent` /
//! `buildSourceKey` / `ensureDatasetId` / `refreshCapiScopeCache`, the
//! worker handlers `send-meta-capi-event.ts` (`MetaCapiEvent` rows) and
//! `ads-conversion/send-conversion-event.ts` (`AdsConversionEvent` rows),
//! `hash-user-data.ts`, and the payload builders of
//! `integrations/meta-conversions/src/apis/events.ts` and
//! `integrations/whatsapp/src/api/conversions.ts`.

use std::sync::OnceLock;

use chatbotx_core::js;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::{self as ads_events, UpdateCapiStatusInput};
use chatbotx_db::repo::models::{AdsConversionEvent, MetaCapiEvent};
use chatbotx_db::repo::{iso, meta_capi_event, now_ms, whatsapp_business_account};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::Enqueuer;
use chatbotx_jobs::events::emit::EventBuses;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::email_marketing::open_auth;
use crate::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use crate::errors::{BusinessError, Result};
use crate::meta_capi::{CapiChannel, decrypt_token};
use crate::meta_catalog::process_encryption;
use crate::tx;

const GRAPH: &str = "https://graph.facebook.com";
/// `integrations/meta-conversions` `DEFAULT_API_VERSION`.
const META_API_VERSION: &str = "v24.0";
/// `integrations/whatsapp` / messenger `DEFAULT_API_VERSION`.
const CHANNEL_API_VERSION: &str = "v23.0";
const META_PARTNER_AGENT: &str = "ChatConnectX";
const WHATSAPP_PARTNER_AGENT: &str = "ChatbotX";
const DATASET_NAME_SUFFIX: &str = "Event Data";
/// `META_CAPI_SCOPE_CACHE_TTL_MS`.
const SCOPE_CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

// ---- hash-user-data.ts ------------------------------------------------------

/// `HashableContact`.
#[derive(Debug, Clone, Default)]
pub struct HashableContact {
    pub id: String,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // capi-param-builder `EMAIL_RE` (`\w` is ASCII in JS).
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^[A-Za-z0-9_!#$%&'*+/=?^`{|}~-]+(:?\.[A-Za-z0-9_!#$%&'*+/=?^`{|}~-]+)*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$",
        )
        .expect("email re")
    })
}

fn strip_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r##"[!"#$%&'()*+,\-./:;<=>?@ \[\\\]^_`{|}~\s]+"##).expect("strip re")
    })
}

fn normalized_email(email: &str) -> Option<String> {
    let normalized = js::to_lower(js::trim(email));
    email_re().is_match(&normalized).then_some(normalized)
}

fn normalized_name(name: &str) -> Option<String> {
    let normalized = js::to_lower(&strip_re().replace_all(name, ""));
    (!normalized.is_empty()).then_some(normalized)
}

fn e164_digits_if_valid(phone: &str) -> Option<String> {
    let parsed = phonenumber::parse(None, phone).ok()?;
    if !parsed.is_valid() {
        return None;
    }
    let e164 = parsed.format().mode(phonenumber::Mode::E164).to_string();
    Some(e164.trim_start_matches('+').to_owned())
}

/// `hashContactUserData`: `em` / `fn` / `ln` / `ph` when they normalize,
/// `external_id` always. Never logs the inputs.
pub fn hash_contact_user_data(contact: &HashableContact) -> Value {
    let mut out = Map::new();
    let mut put = |key: &str, value: Option<String>| {
        if let Some(value) = value {
            out.insert(key.into(), json!([sha256_hex(&value)]));
        }
    };
    put("em", some_text(&contact.email).and_then(normalized_email));
    put(
        "fn",
        some_text(&contact.first_name).and_then(normalized_name),
    );
    put(
        "ln",
        some_text(&contact.last_name).and_then(normalized_name),
    );
    put(
        "ph",
        some_text(&contact.phone_number).and_then(e164_digits_if_valid),
    );
    put("external_id", Some(contact.id.clone()));
    Value::Object(out)
}

// ---- events.ts / conversions.ts payloads ---------------------------------------

/// The identity of a Meta CAPI event (`BusinessMessagingIdentity` |
/// `NonMessagingIdentity`).
#[derive(Debug, Clone, PartialEq)]
pub enum Identity {
    Messenger {
        page_id: String,
        psid: String,
    },
    Instagram {
        account_id: String,
        igsid: String,
    },
    Whatsapp {
        waba_id: String,
        ctwa_clid: String,
    },
    /// Any action source but `business_messaging`.
    NonMessaging {
        action_source: String,
    },
}

/// `MetaConversionEventInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetaEvent {
    pub event_name: String,
    pub occurred_at: i64,
    pub event_id: String,
    pub identity: Option<Identity>,
    pub user_data: Option<Value>,
    pub currency: Option<String>,
    pub value: Option<String>,
    pub content_category: Option<String>,
    pub content_name: Option<String>,
    pub content_type: Option<String>,
    pub content_ids: Option<Vec<String>>,
    pub limited_data_use: bool,
    pub order_id: Option<String>,
    /// `PurchaseContentItem[]` (`{ id, quantity, itemPrice }`).
    pub contents: Option<Vec<Value>>,
}

/// `Number(text)` as JSON (an integral value stays an integer).
fn js_number(text: &str) -> Value {
    let n = js::to_number(text);
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        json!(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn some_text(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

/// `buildContentsData` (`num_items` = the sum of the quantities).
fn contents_data(contents: &[Value], out: &mut Map<String, Value>) {
    let total: f64 = contents
        .iter()
        .map(|item| item["quantity"].as_f64().unwrap_or(0.0))
        .sum();
    out.insert("num_items".into(), js_number(&js::number_to_string(total)));
    out.insert(
        "contents".into(),
        Value::Array(
            contents
                .iter()
                .map(|item| {
                    json!({ "id": item["id"], "quantity": item["quantity"], "item_price": item["itemPrice"] })
                })
                .collect(),
        ),
    );
}

fn ldu(event_limited: bool, out: &mut Map<String, Value>) {
    if event_limited {
        out.insert("data_processing_options".into(), json!(["LDU"]));
        out.insert("data_processing_options_country".into(), json!(0));
        out.insert("data_processing_options_state".into(), json!(0));
    }
}

fn merge(identity: Value, user_data: Option<&Value>) -> Value {
    let mut map = identity.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(extra)) = user_data {
        map.extend(extra.clone());
    }
    Value::Object(map)
}

/// `buildConversionEventPayload` (Meta Conversions API).
pub fn meta_event_payload(event: &MetaEvent) -> Value {
    let mut out = Map::new();
    out.insert("event_name".into(), json!(event.event_name));
    out.insert(
        "event_time".into(),
        json!(event.occurred_at.div_euclid(1000)),
    );
    out.insert("event_id".into(), json!(event.event_id));
    let messaging = |channel: &str, ids: Value, out: &mut Map<String, Value>| {
        out.insert("action_source".into(), json!("business_messaging"));
        out.insert("messaging_channel".into(), json!(channel));
        out.insert("user_data".into(), merge(ids, event.user_data.as_ref()));
    };
    match &event.identity {
        Some(Identity::Messenger { page_id, psid }) => messaging(
            "messenger",
            json!({ "page_id": page_id, "page_scoped_user_id": psid }),
            &mut out,
        ),
        Some(Identity::Instagram { account_id, igsid }) => messaging(
            "instagram",
            json!({ "ig_account_id": account_id, "instagram_business_account_id": account_id, "ig_sid": igsid }),
            &mut out,
        ),
        Some(Identity::Whatsapp { waba_id, ctwa_clid }) => messaging(
            "whatsapp",
            json!({ "whatsapp_business_account_id": waba_id, "ctwa_clid": ctwa_clid }),
            &mut out,
        ),
        Some(Identity::NonMessaging { action_source }) => {
            out.insert("action_source".into(), json!(action_source));
            out.insert(
                "user_data".into(),
                event.user_data.clone().unwrap_or(Value::Null),
            );
        }
        None => {}
    }
    let contents = event.contents.as_deref().filter(|c| !c.is_empty());
    let content_ids = event.content_ids.as_deref().filter(|c| !c.is_empty());
    let content_type = event
        .content_type
        .clone()
        .or_else(|| contents.map(|_| "product".to_owned()));
    let mut custom = Map::new();
    if let Some(currency) = some_text(&event.currency) {
        custom.insert("currency".into(), json!(currency));
    }
    if let Some(value) = &event.value {
        custom.insert("value".into(), js_number(value));
    }
    if let Some(v) = some_text(&event.content_category) {
        custom.insert("content_category".into(), json!(v));
    }
    if let Some(v) = some_text(&event.content_name) {
        custom.insert("content_name".into(), json!(v));
    }
    if let Some(v) = some_text(&event.order_id) {
        custom.insert("order_id".into(), json!(v));
    }
    if let Some(v) = content_type.filter(|s| !s.is_empty()) {
        custom.insert("content_type".into(), json!(v));
    }
    if let Some(ids) = content_ids {
        custom.insert("content_ids".into(), json!(ids));
    }
    if let Some(contents) = contents {
        contents_data(contents, &mut custom);
    }
    if !custom.is_empty() {
        out.insert("custom_data".into(), Value::Object(custom));
    }
    ldu(event.limited_data_use, &mut out);
    Value::Object(out)
}

/// The request body of `sendConversionEvent` (Meta Conversions API).
pub fn meta_events_body(event: &MetaEvent, test_event_code: Option<&str>) -> Value {
    let mut body =
        json!({ "data": [meta_event_payload(event)], "partner_agent": META_PARTNER_AGENT });
    if let Some(code) = test_event_code.filter(|c| !c.is_empty()) {
        body["test_event_code"] = json!(code);
    }
    body
}

/// `ConversionEventInput` (WhatsApp Conversions API).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WhatsappEvent {
    /// `lead` | `purchase`.
    pub event_type: String,
    pub occurred_at: i64,
    pub source_event_id: String,
    pub ctwa_clid: String,
    pub waba_id: String,
    pub currency: Option<String>,
    pub value: Option<String>,
    pub automatic: bool,
    pub user_data: Option<Value>,
    pub limited_data_use: bool,
    pub order_id: Option<String>,
    pub contents: Option<Vec<Value>>,
}

/// `buildConversionEventPayload` (WhatsApp Conversions API).
pub fn whatsapp_event_payload(event: &WhatsappEvent) -> Value {
    let name = if event.event_type == "purchase" {
        "Purchase"
    } else {
        "LeadSubmitted"
    };
    let mut out = Map::new();
    out.insert("event_name".into(), json!(name));
    out.insert(
        "event_time".into(),
        json!(event.occurred_at.div_euclid(1000)),
    );
    out.insert("event_id".into(), json!(event.source_event_id));
    out.insert("action_source".into(), json!("business_messaging"));
    out.insert("messaging_channel".into(), json!("whatsapp"));
    if event.automatic {
        out.insert(
            "messaging_outcome_data".into(),
            json!({ "outcome_type": "automatic_events" }),
        );
    }
    out.insert(
        "user_data".into(),
        merge(
            json!({ "whatsapp_business_account_id": event.waba_id, "ctwa_clid": event.ctwa_clid }),
            event.user_data.as_ref(),
        ),
    );
    let contents = event.contents.as_deref().filter(|c| !c.is_empty());
    let mut custom = Map::new();
    if let Some(currency) = some_text(&event.currency) {
        custom.insert("currency".into(), json!(currency));
    }
    if let Some(value) = &event.value {
        custom.insert("value".into(), js_number(value));
    }
    if let Some(v) = some_text(&event.order_id) {
        custom.insert("order_id".into(), json!(v));
    }
    if let Some(contents) = contents {
        custom.insert("content_type".into(), json!("product"));
        contents_data(contents, &mut custom);
    }
    if !custom.is_empty() {
        out.insert("custom_data".into(), Value::Object(custom));
    }
    ldu(event.limited_data_use, &mut out);
    Value::Object(out)
}

/// The request body of the WhatsApp `sendConversionEvent`.
pub fn whatsapp_events_body(event: &WhatsappEvent) -> Value {
    json!({ "data": [whatsapp_event_payload(event)], "partner_agent": WHATSAPP_PARTNER_AGENT })
}

/// `buildDatasetName`.
pub fn dataset_name(resource_name: &str) -> String {
    let trimmed = resource_name.trim();
    if trimmed.is_empty() {
        DATASET_NAME_SUFFIX.to_owned()
    } else {
        format!("{trimmed} {DATASET_NAME_SUFFIX}")
    }
}

// ---- Graph ------------------------------------------------------------------

/// A failed Meta call (`MetaConversionsException` / `WhatsappConversionsApiException`).
#[derive(Debug, Clone, PartialEq)]
pub struct CapiError {
    pub message: String,
    pub status: u16,
    pub code: Option<i64>,
    pub retryable: bool,
}

impl CapiError {
    /// A terminal local failure (a thrown `Error`).
    pub fn terminal(message: impl Into<String>) -> Self {
        CapiError {
            message: message.into(),
            status: 400,
            code: None,
            retryable: false,
        }
    }
}

impl From<BusinessError> for CapiError {
    fn from(error: BusinessError) -> Self {
        CapiError::terminal(error.message())
    }
}

/// `isRetryableMetaConversionsStatus` / `isRetryableHttpStatus`.
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500
}

/// `isMetaAuthorizationError`.
pub fn is_meta_authorization_error(error: &CapiError) -> bool {
    matches!(error.code, Some(10 | 100 | 190 | 200)) || matches!(error.status, 401 | 403)
}

#[derive(Clone, Copy, PartialEq)]
enum Api {
    Meta,
    Whatsapp,
}

fn segment(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// One Graph call; an answer outside 2xx is a [`CapiError`].
async fn graph(
    up: &impl Upstream,
    api: Api,
    method: &'static str,
    url: String,
    bearer: &str,
    body: Option<&Value>,
) -> std::result::Result<Value, CapiError> {
    let mut headers = vec![("authorization".to_owned(), format!("Bearer {bearer}"))];
    if body.is_some() {
        headers.push(("content-type".into(), "application/json".into()));
    }
    let reply = up
        .send(UpstreamRequest {
            method,
            url,
            headers,
            body: body.map(Value::to_string),
        })
        .await;
    let (status, text) = match reply {
        Ok(reply) => reply,
        // A transport failure has no HTTP status: the 400 fallback, terminal.
        Err(message) => return Err(CapiError::terminal(message)),
    };
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(parsed);
    }
    let error = &parsed["error"];
    let status = error["status"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or(status);
    let message = match api {
        Api::Meta => crate::meta_capi::graph_message(&parsed),
        Api::Whatsapp => error["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| "WhatsApp Conversions API call failed".to_owned()),
    };
    Err(CapiError {
        message,
        status,
        code: error["code"].as_i64(),
        retryable: is_retryable_status(status),
    })
}

/// `ensureDataset` (`{resource}/dataset`, `dataset_name` when given).
async fn create_dataset(
    up: &impl Upstream,
    api: Api,
    version: &str,
    resource_id: &str,
    token: &str,
    name: &str,
) -> std::result::Result<String, CapiError> {
    let name = name.trim();
    let body = (!name.is_empty()).then(|| json!({ "dataset_name": name }));
    let reply = graph(
        up,
        api,
        "POST",
        format!("{GRAPH}/{version}/{}/dataset", segment(resource_id)),
        token,
        body.as_ref(),
    )
    .await?;
    [
        &reply["id"],
        &reply["dataset_id"],
        &reply["data"]["id"],
        &reply["dataset"]["id"],
    ]
    .into_iter()
    .find_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
    .map(str::to_owned)
    .ok_or_else(|| CapiError::terminal("Invalid dataset response"))
}

/// `createDatasetWithFallback`: one retry with `fallback` only for a Meta
/// authorization error.
async fn create_dataset_with_fallback(
    up: &impl Upstream,
    api: Api,
    version: &str,
    resource_id: &str,
    tokens: &DatasetTokens,
    name: &str,
) -> std::result::Result<String, CapiError> {
    match create_dataset(up, api, version, resource_id, &tokens.primary, name).await {
        Ok(id) => Ok(id),
        Err(error) => match &tokens.fallback {
            Some(fallback)
                if *fallback != tokens.primary && is_meta_authorization_error(&error) =>
            {
                create_dataset(up, api, version, resource_id, fallback, name).await
            }
            _ => Err(error),
        },
    }
}

/// `integrations/meta-conversions` `sendConversionEvent`.
pub async fn send_meta_event(
    up: &impl Upstream,
    dataset_id: &str,
    token: &str,
    event: &MetaEvent,
    test_event_code: Option<&str>,
) -> std::result::Result<(), CapiError> {
    graph(
        up,
        Api::Meta,
        "POST",
        format!("{GRAPH}/{META_API_VERSION}/{}/events", segment(dataset_id)),
        token,
        Some(&meta_events_body(event, test_event_code)),
    )
    .await
    .map(|_| ())
}

/// `integrations/whatsapp` `sendConversionEvent`.
pub async fn send_whatsapp_event(
    up: &impl Upstream,
    version: &str,
    dataset_id: &str,
    token: &str,
    event: &WhatsappEvent,
) -> std::result::Result<(), CapiError> {
    graph(
        up,
        Api::Whatsapp,
        "POST",
        format!("{GRAPH}/{version}/{}/events", segment(dataset_id)),
        token,
        Some(&whatsapp_events_body(event)),
    )
    .await
    .map(|_| ())
}

/// `integrations/whatsapp` `ensureDataset` (`dataset_name` when given).
pub async fn ensure_whatsapp_dataset(
    up: &impl Upstream,
    version: &str,
    waba_id: &str,
    token: &str,
    name: Option<&str>,
) -> std::result::Result<String, CapiError> {
    create_dataset(
        up,
        Api::Whatsapp,
        version,
        waba_id,
        token,
        name.unwrap_or(""),
    )
    .await
}

/// `{ primaryToken, fallbackToken }` of a dataset create.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetTokens {
    pub primary: String,
    pub fallback: Option<String>,
}

/// `resolveDatasetCreationTokens`: an embedded-signup WhatsApp connection
/// creates with the agency System User token and falls back to the connect
/// token; a manual connection or an owner without one uses the connect token.
pub async fn whatsapp_dataset_tokens(
    db: &Db,
    workspace_id: i64,
    auth: &Value,
    connect_token: &str,
) -> DatasetTokens {
    let only = DatasetTokens {
        primary: connect_token.to_owned(),
        fallback: None,
    };
    if auth["metadata"]["isManual"] == true {
        return only;
    }
    let Ok(owner) =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await
    else {
        return only;
    };
    let system = tx::read(db, move |conn| {
        crate::platform_credential::resolve_whatsapp_system_user_token(
            conn,
            process_encryption(),
            owner,
        )
    })
    .await
    .ok()
    .flatten()
    .filter(|t| !t.is_empty());
    match system {
        Some(primary) => DatasetTokens {
            primary,
            fallback: Some(connect_token.to_owned()),
        },
        None => only,
    }
}

// ---- the integration row ----------------------------------------------------

/// Where the send token came from (`ResolvedCapiAccessToken.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Manual,
    Waba,
    Oauth,
}

/// The CAPI columns of one channel integration (`MetaConversionsIntegration`).
#[derive(Debug, Clone)]
pub struct SendIntegration {
    pub channel: CapiChannel,
    pub id: i64,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub name: String,
    /// `pageId` / `igId` / `wabaId`.
    pub resource_id: String,
    /// Instagram `type` (`facebook` | `instagram`).
    pub kind: Option<String>,
    pub dataset_id: Option<String>,
    pub test_event_code: Option<String>,
    pub disconnected: bool,
    pub has_capi_scope: bool,
    pub scope_checked_at: Option<i64>,
    manual: Option<Value>,
    /// The decrypted `auth`.
    pub auth: Value,
}

const SELECT: &str = "\"id\", \"workspaceId\", \"inboxId\", \"name\", \"datasetId\", \"capiTestEventCode\", \
    \"capiDisconnectedAt\", \"hasCapiScope\", \"capiScopeCheckedAt\", \"capiAccessToken\", \"auth\"";

fn resource_column(channel: CapiChannel) -> &'static str {
    match channel {
        CapiChannel::Messenger => "pageId",
        CapiChannel::Instagram => "igId",
        CapiChannel::Whatsapp => "wabaId",
    }
}

fn table(channel: CapiChannel) -> &'static str {
    match channel {
        CapiChannel::Messenger => "IntegrationMessenger",
        CapiChannel::Instagram => "IntegrationInstagram",
        CapiChannel::Whatsapp => "IntegrationWhatsapp",
    }
}

fn load_where(
    conn: &Connection,
    channel: CapiChannel,
    filter: &str,
    a: i64,
    b: i64,
) -> chatbotx_db::rusqlite::Result<Option<SendIntegration>> {
    let kind = if channel == CapiChannel::Instagram {
        "\"type\""
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT {SELECT}, \"{}\", {kind} FROM \"{}\" WHERE {filter}",
        resource_column(channel),
        table(channel)
    );
    conn.query_row(&sql, params![a, b], |r| {
        let manual: Option<String> = r.get(9)?;
        let auth: String = r.get(10)?;
        Ok(SendIntegration {
            channel,
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            inbox_id: r.get(2)?,
            name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            dataset_id: r.get(4)?,
            test_event_code: r.get(5)?,
            disconnected: r.get::<_, Option<i64>>(6)?.is_some(),
            has_capi_scope: r.get(7)?,
            scope_checked_at: r.get(8)?,
            manual: manual
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .filter(|v| !v.is_null()),
            auth: open_auth(&auth).unwrap_or(Value::Null),
            resource_id: r.get(11)?,
            kind: r.get(12)?,
        })
    })
    .optional()
}

/// `findByIdForWorkspace`.
pub fn find_integration(
    conn: &Connection,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
) -> chatbotx_db::rusqlite::Result<Option<SendIntegration>> {
    load_where(
        conn,
        channel,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        id,
        workspace_id,
    )
}

/// `findByInboxIdForWorkspace`.
pub fn find_integration_by_inbox(
    conn: &Connection,
    channel: CapiChannel,
    workspace_id: i64,
    inbox_id: i64,
) -> chatbotx_db::rusqlite::Result<Option<SendIntegration>> {
    load_where(
        conn,
        channel,
        "\"inboxId\" = ?1 AND \"workspaceId\" = ?2",
        inbox_id,
        workspace_id,
    )
}

impl SendIntegration {
    /// `assertSupported`: Instagram Business Login rows have no Meta CAPI.
    pub fn assert_supported(&self) -> std::result::Result<(), CapiError> {
        if self.channel == CapiChannel::Instagram && self.kind.as_deref() == Some("instagram") {
            return Err(CapiError::terminal(
                "Instagram Business Login integrations do not support Meta CAPI",
            ));
        }
        Ok(())
    }

    fn oauth_token(&self) -> Option<String> {
        self.auth["tokens"]["accessToken"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    }

    /// `resolveCapiAccessTokenForChannel`: the manual token, else (WhatsApp)
    /// the WABA credential, else `auth.tokens.accessToken`.
    pub fn access_token(
        &self,
        conn: &Connection,
    ) -> std::result::Result<(String, TokenSource), CapiError> {
        if let Some(raw) = &self.manual {
            return decrypt_token(raw, None)
                .map(|t| (t, TokenSource::Manual))
                .ok_or_else(|| CapiError::terminal("Meta CAPI manual access token is invalid"));
        }
        if self.channel == CapiChannel::Whatsapp {
            let waba =
                whatsapp_business_account::find_by_waba(conn, self.workspace_id, &self.resource_id)
                    .map_err(|e| CapiError::terminal(e.to_string()))?;
            let aad = format!("whatsapp-waba:{}:{}", self.workspace_id, self.resource_id);
            if let Some(token) = waba.and_then(|w| decrypt_token(&w.credential.0, Some(&aad))) {
                return Ok((token, TokenSource::Waba));
            }
        }
        self.oauth_token()
            .map(|t| (t, TokenSource::Oauth))
            .ok_or_else(|| CapiError::terminal("Meta CAPI access token is missing"))
    }

    /// The `/debug_token`-able channel token (`buildScopeCheckInput`).
    fn scope_token(&self) -> std::result::Result<String, CapiError> {
        self.oauth_token()
            .ok_or_else(|| CapiError::terminal("Meta CAPI access token is missing"))
    }
}

/// `isCapiScopeRequired`: WhatsApp is exempt.
pub fn is_capi_scope_required(channel: CapiChannel) -> bool {
    channel != CapiChannel::Whatsapp
}

/// `capiEventRequiresCtwaClid` / `capiEventDedupsPerUtcDay` (the same rule).
pub fn keyed_to_ad_click(channel: CapiChannel, action_source: &str) -> bool {
    channel == CapiChannel::Whatsapp && action_source == "business_messaging"
}

/// The Meta channel of a `MetaCapiEvent.channel` / contact-inbox channel.
pub fn capi_channel(channel: &str) -> Option<CapiChannel> {
    match channel {
        "messenger" => Some(CapiChannel::Messenger),
        "instagram" => Some(CapiChannel::Instagram),
        "whatsapp" => Some(CapiChannel::Whatsapp),
        _ => None,
    }
}

// ---- refreshCapiScopeCache --------------------------------------------------

/// `refreshCapiScopeCache` + `checkMetaCapiScope` (Messenger / Instagram):
/// a stored check younger than 24 h is kept; else the refresh is claimed
/// (CAS on `capiScopeCheckedAt`), the channel token is debugged with the
/// owner's app credential, and the result stored. A failed check gives the
/// claim back and is retryable.
pub async fn refresh_scope_cache(
    db: &Db,
    up: &impl Upstream,
    integration: SendIntegration,
) -> std::result::Result<SendIntegration, CapiError> {
    let now = now_ms();
    if integration
        .scope_checked_at
        .is_some_and(|at| now - at < SCOPE_CACHE_TTL_MS)
    {
        return Ok(integration);
    }
    let (channel, id, ws) = (
        integration.channel,
        integration.id,
        integration.workspace_id,
    );
    let expected = integration.scope_checked_at;
    let claimed = tx::write(db, move |tx| {
        Ok(tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"capiScopeCheckedAt\" = ?1, \"updatedAt\" = ?1 \
                 WHERE \"id\" = ?2 AND \"workspaceId\" = ?3 AND \"capiScopeCheckedAt\" IS ?4",
                table(channel)
            ),
            params![now, id, ws, expected],
        )? > 0)
    })
    .await?;
    if !claimed {
        return Ok(reread(db, channel, ws, id).await?.unwrap_or(integration));
    }
    let has_capi_scope = match check_scope(db, up, &integration).await {
        Ok(has) => has,
        Err(error) => {
            tracing::warn!(
                id,
                workspace_id = ws,
                message = error.message,
                "meta-conversions: CAPI scope refresh failed"
            );
            if let Err(e) = restore_claim(
                db,
                channel,
                ws,
                id,
                integration.has_capi_scope,
                expected,
                now,
            )
            .await
            {
                tracing::warn!(id, err = %e.message(), "meta-conversions: failed to restore CAPI scope refresh claim");
            }
            return Err(CapiError {
                message: "Meta CAPI scope refresh failed".into(),
                status: 500,
                code: None,
                retryable: true,
            });
        }
    };
    tx::write(db, move |tx| {
        tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"hasCapiScope\" = ?1, \"updatedAt\" = ?2 \
                 WHERE \"id\" = ?3 AND \"workspaceId\" = ?4 AND \"capiScopeCheckedAt\" IS ?5",
                table(channel)
            ),
            params![has_capi_scope, now_ms(), id, ws, now],
        )?;
        Ok(())
    })
    .await?;
    Ok(reread(db, channel, ws, id).await?.unwrap_or(integration))
}

async fn reread(
    db: &Db,
    channel: CapiChannel,
    ws: i64,
    id: i64,
) -> Result<Option<SendIntegration>> {
    tx::read(db, move |conn| Ok(find_integration(conn, channel, ws, id)?)).await
}

/// Gives a failed scope refresh claim back (`capiScopeCheckedAt` = `expected`).
async fn restore_claim(
    db: &Db,
    channel: CapiChannel,
    ws: i64,
    id: i64,
    has: bool,
    expected: Option<i64>,
    now: i64,
) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"hasCapiScope\" = ?1, \"capiScopeCheckedAt\" = ?2, \"updatedAt\" = ?3 \
                 WHERE \"id\" = ?4 AND \"workspaceId\" = ?5 AND \"capiScopeCheckedAt\" IS ?6",
                table(channel)
            ),
            params![has, expected, now_ms(), id, ws, now],
        )?;
        Ok(())
    })
    .await
}

async fn check_scope(
    db: &Db,
    up: &impl Upstream,
    integration: &SendIntegration,
) -> std::result::Result<bool, CapiError> {
    let (kind, scope) = match integration.channel {
        CapiChannel::Instagram => ("instagramFacebook", "instagram_manage_events"),
        _ => ("messenger", "page_events"),
    };
    let owner = crate::tenant::resolve_credential_owner_id_for_workspace(
        db,
        integration.workspace_id,
        None,
    )
    .await?;
    let credential = tx::read(db, move |conn| {
        crate::platform_credential::resolve_for_owner(
            conn,
            process_encryption(),
            owner,
            kind,
            false,
        )
    })
    .await?;
    let Some(credential) = credential else {
        return Ok(integration.has_capi_scope);
    };
    let app_token = format!(
        "{}|{}",
        credential.text("clientId").unwrap_or_default(),
        credential.text("clientSecret").unwrap_or_default()
    );
    let version = credential
        .text("version")
        .filter(|v| !v.is_empty())
        .unwrap_or(CHANNEL_API_VERSION);
    let mut url = url::Url::parse(&format!("{GRAPH}/{version}/debug_token"))
        .map_err(|e| CapiError::terminal(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("input_token", &integration.scope_token()?);
    let reply = graph(up, Api::Meta, "GET", url.to_string(), &app_token, None).await?;
    Ok(reply["data"]["scopes"]
        .as_array()
        .is_some_and(|s| s.iter().any(|v| v == scope)))
}

// ---- ensureDatasetId --------------------------------------------------------

/// `metaConversionsService.ensureDatasetId` / `integrationWhatsappService.
/// ensureDatasetId`: the stored id, else the one Meta links (created with
/// the channel's tokens) written only while still null.
pub async fn ensure_dataset_id(
    db: &Db,
    up: &impl Upstream,
    integration: &SendIntegration,
    connect_token: &str,
) -> std::result::Result<String, CapiError> {
    if let Some(id) = &integration.dataset_id {
        return Ok(id.clone());
    }
    let created = provision_dataset(db, up, integration, connect_token).await?;
    let (channel, id, ws) = (
        integration.channel,
        integration.id,
        integration.workspace_id,
    );
    let stored = tx::write(db, move |tx| {
        tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"datasetId\" = ?1, \"updatedAt\" = ?2 \
                 WHERE \"id\" = ?3 AND \"workspaceId\" = ?4 AND \"datasetId\" IS NULL",
                table(channel)
            ),
            params![created, now_ms(), id, ws],
        )?;
        Ok(tx
            .query_row(
                &format!(
                    "SELECT \"datasetId\" FROM \"{}\" WHERE \"id\" = ?1",
                    table(channel)
                ),
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    })
    .await?;
    stored.ok_or_else(|| CapiError::terminal("Meta CAPI dataset id was not stored"))
}

/// `provisionDatasetViaMeta`: the WhatsApp dataset is created with the
/// agency System User token first (connect-token fallback); the other
/// channels use the connect token only.
pub async fn provision_dataset(
    db: &Db,
    up: &impl Upstream,
    integration: &SendIntegration,
    connect_token: &str,
) -> std::result::Result<String, CapiError> {
    let tokens = match integration.channel {
        CapiChannel::Whatsapp => {
            whatsapp_dataset_tokens(
                db,
                integration.workspace_id,
                &integration.auth,
                connect_token,
            )
            .await
        }
        _ => DatasetTokens {
            primary: connect_token.to_owned(),
            fallback: None,
        },
    };
    let (api, version) = match integration.channel {
        CapiChannel::Whatsapp => (
            Api::Whatsapp,
            integration.auth["version"]
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(CHANNEL_API_VERSION)
                .to_owned(),
        ),
        _ => (Api::Meta, META_API_VERSION.to_owned()),
    };
    create_dataset_with_fallback(
        up,
        api,
        &version,
        &integration.resource_id,
        &tokens,
        &dataset_name(&integration.name),
    )
    .await
}

// ---- the send handlers --------------------------------------------------------

/// What the send handlers need.
pub struct SendDeps<'a, U: Upstream> {
    pub db: &'a Db,
    pub up: &'a U,
    pub events: Option<&'a EventBuses>,
}

/// A send handler's failure that the job retries.
pub type Retry = String;

struct Inbox {
    contact: HashableContact,
    source_id: String,
    inbox_id: i64,
    ctwa_clid: Option<String>,
}

/// The inbox, contact id, source id and inbox id of a contact inbox.
type LoadedInbox = (Option<Inbox>, i64, String, i64);

/// `contactInboxService.findByUncached` + the workspace-scoped contact.
fn load_inbox(
    conn: &Connection,
    workspace_id: i64,
    contact_inbox_id: i64,
) -> chatbotx_db::rusqlite::Result<Option<LoadedInbox>> {
    let row: Option<(i64, String, i64, Option<String>)> = conn
        .query_row(
            "SELECT \"contactId\", \"sourceId\", \"inboxId\", \"referral\" FROM \"ContactInbox\" WHERE \"id\" = ?1",
            params![contact_inbox_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((contact_id, source_id, inbox_id, referral)) = row else {
        return Ok(None);
    };
    let contact = conn
        .query_row(
            "SELECT \"email\", \"phoneNumber\", \"firstName\", \"lastName\" FROM \"Contact\" \
             WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
            params![contact_id, workspace_id],
            |r| {
                Ok(HashableContact {
                    id: contact_id.to_string(),
                    email: r.get(0)?,
                    phone_number: r.get(1)?,
                    first_name: r.get(2)?,
                    last_name: r.get(3)?,
                })
            },
        )
        .optional()?;
    let ctwa_clid = referral
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .and_then(|r| r["ctwaClid"].as_str().map(str::to_owned))
        .filter(|c| !c.is_empty());
    Ok(Some((
        contact.map(|contact| Inbox {
            contact,
            source_id: source_id.clone(),
            inbox_id,
            ctwa_clid,
        }),
        contact_id,
        source_id,
        inbox_id,
    )))
}

fn workspace_ldu(conn: &Connection, workspace_id: i64) -> chatbotx_db::rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT \"capiLimitedDataUse\" FROM \"Workspace\" WHERE \"id\" = ?1",
            params![workspace_id],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

async fn log_failure(
    events: Option<&EventBuses>,
    provider: &str,
    workspace_id: i64,
    contact: Option<(String, String)>,
    message: &str,
) {
    let mut input = ProviderErrorInput::new(
        provider,
        &workspace_id.to_string(),
        Thrown::Error {
            message: message.to_owned(),
            stack: None,
            status: None,
        },
    );
    input.http_code = Some(Some("400".into()));
    if let Some((contact_id, source_id)) = contact {
        input.contact_id = Some(contact_id);
        input.source_id = Some(source_id);
    }
    log_provider_error(events, input).await;
}

/// `metaConversionsService.updateCapiStatus` (pending → `to`).
async fn set_meta_status(
    db: &Db,
    event: &MetaCapiEvent,
    to: &'static str,
    sent_at: Option<i64>,
    error: Option<String>,
) -> std::result::Result<(), Retry> {
    let (id, ws) = (event.id, event.workspace_id);
    tx::write(db, move |tx| {
        meta_capi_event::update_capi_status(
            tx,
            id,
            ws,
            "pending",
            to,
            sent_at,
            error.as_deref().map(Some),
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.message())
}

/// `handleSendMetaCapiEvent` (inside `withBlockedOwnerGuard`, which the
/// caller applies). `Err` = the job is retried.
pub async fn send_meta_capi_event<U: Upstream>(
    deps: &SendDeps<'_, U>,
    event_id: i64,
    workspace_id: i64,
) -> std::result::Result<(), Retry> {
    let db = deps.db;
    let event = tx::read(db, move |conn| {
        Ok(meta_capi_event::find_workspace_event(
            conn,
            event_id,
            workspace_id,
        )?)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(event) = event.filter(|e| e.capi_status == "pending") else {
        return Ok(());
    };
    let Some(channel) = capi_channel(&event.channel) else {
        return set_meta_status(
            db,
            &event,
            "failed",
            None,
            Some("integrationNotFound".into()),
        )
        .await;
    };
    let (integration_id, ws) = (event.integration_id, event.workspace_id);
    let integration = tx::read(db, move |conn| {
        Ok(find_integration(conn, channel, ws, integration_id)?)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(integration) = integration else {
        tracing::warn!(
            meta_capi_event_id = event.id,
            workspace_id = ws,
            "Meta CAPI event integration not found; marked failed"
        );
        return set_meta_status(
            db,
            &event,
            "failed",
            None,
            Some("integrationNotFound".into()),
        )
        .await;
    };
    if integration.disconnected {
        return set_meta_status(db, &event, "skipped_disconnected", None, None).await;
    }
    // Read outside the try: a failure retries instead of a wrong LDU state.
    let ldu = tx::read(db, move |conn| Ok(workspace_ldu(conn, ws)?))
        .await
        .map_err(|e| e.message())?;
    let mut contact_ref: Option<(String, String)> = None;
    let outcome =
        send_meta_capi_try(deps, &event, channel, integration, ldu, &mut contact_ref).await;
    match outcome {
        Ok(Some((to, error))) => set_meta_status(db, &event, to, None, error).await,
        Ok(None) => set_meta_status(db, &event, "sent", Some(now_ms()), None).await,
        Err(error) if error.retryable => Err(error.message),
        Err(error) => {
            set_meta_status(db, &event, "failed", None, Some(error.message.clone())).await?;
            log_failure(
                deps.events,
                "meta-conversions",
                ws,
                contact_ref,
                &error.message,
            )
            .await;
            tracing::warn!(
                meta_capi_event_id = event.id,
                workspace_id = ws,
                error_message = error.message,
                "Meta CAPI event marked failed"
            );
            Ok(())
        }
    }
}

/// The `try` block: `Ok(Some(status))` = an early terminal status,
/// `Ok(None)` = sent.
async fn send_meta_capi_try<U: Upstream>(
    deps: &SendDeps<'_, U>,
    event: &MetaCapiEvent,
    channel: CapiChannel,
    integration: SendIntegration,
    ldu: bool,
    contact_ref: &mut Option<(String, String)>,
) -> std::result::Result<Option<(&'static str, Option<String>)>, CapiError> {
    let db = deps.db;
    let (ws, cib) = (event.workspace_id, event.contact_inbox_id);
    let loaded = tx::read(db, move |conn| Ok(load_inbox(conn, ws, cib)?)).await?;
    let Some((inbox, _, _, _)) = loaded else {
        tracing::warn!(
            meta_capi_event_id = event.id,
            "Meta CAPI event contact inbox not found; marked failed"
        );
        return Ok(Some(("failed", Some("contactInboxNotFound".into()))));
    };
    let Some(inbox) = inbox.filter(|i| i.inbox_id == integration.inbox_id) else {
        tracing::error!(
            meta_capi_event_id = event.id,
            "Meta CAPI event contact inbox workspace/inbox mismatch; marked failed"
        );
        return Ok(Some((
            "failed",
            Some("contactInboxWorkspaceMismatch".into()),
        )));
    };
    *contact_ref = Some((inbox.contact.id.clone(), inbox.source_id.clone()));
    if keyed_to_ad_click(channel, &event.action_source) && inbox.ctwa_clid.is_none() {
        return Ok(Some(("skipped_no_identity", None)));
    }
    let probe = integration.clone();
    let (token, source) = tx::read(db, move |conn| Ok(probe.access_token(conn))).await??;
    let scope_required = is_capi_scope_required(channel);
    let integration = if source == TokenSource::Manual || !scope_required {
        integration
    } else {
        integration.assert_supported()?;
        refresh_scope_cache(db, deps.up, integration).await?
    };
    if source == TokenSource::Manual && integration.dataset_id.is_none() {
        return Ok(Some(("skipped_no_scope", None)));
    }
    if source != TokenSource::Manual && scope_required && !integration.has_capi_scope {
        return Ok(Some(("skipped_no_scope", None)));
    }
    // `resolveTestEventCode`: a manual test re-reads the row (a clear wins).
    let test_event_code = if event.source == "manualTest" {
        let (id, ws) = (integration.id, integration.workspace_id);
        tx::read(db, move |conn| Ok(find_integration(conn, channel, ws, id)?))
            .await?
            .and_then(|i| i.test_event_code)
    } else {
        integration.test_event_code.clone()
    };
    if event.source == "manualTest" && test_event_code.is_none() {
        return Ok(Some(("failed", Some("testEventCodeMissing".into()))));
    }
    integration.assert_supported()?;
    let dataset_id = match (&integration.dataset_id, source) {
        (Some(id), TokenSource::Manual) => id.clone(),
        _ => ensure_dataset_id(db, deps.up, &integration, &token).await?,
    };
    let user_data = hash_contact_user_data(&inbox.contact);
    let identity = if event.action_source == "business_messaging" {
        Some(match channel {
            CapiChannel::Messenger => Identity::Messenger {
                page_id: integration.resource_id.clone(),
                psid: inbox.source_id.clone(),
            },
            CapiChannel::Instagram => Identity::Instagram {
                account_id: integration.resource_id.clone(),
                igsid: inbox.source_id.clone(),
            },
            CapiChannel::Whatsapp => Identity::Whatsapp {
                waba_id: integration.resource_id.clone(),
                ctwa_clid: inbox.ctwa_clid.clone().ok_or_else(|| {
                    CapiError::terminal("Missing ctwa_clid for WhatsApp Meta CAPI event")
                })?,
            },
        })
    } else {
        Some(Identity::NonMessaging {
            action_source: event.action_source.clone(),
        })
    };
    let content_ids = event.content_ids.as_ref().and_then(|ids| {
        ids.0.as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
    });
    let payload = MetaEvent {
        event_name: event.event_name.clone(),
        occurred_at: event.occurred_at,
        event_id: event.source_key.clone(),
        identity,
        user_data: Some(user_data),
        currency: event.currency.clone(),
        value: event.value.clone(),
        content_category: event.content_category.clone(),
        content_name: event.content_name.clone(),
        content_type: event.content_type.clone(),
        content_ids,
        limited_data_use: ldu,
        order_id: None,
        contents: None,
    };
    send_meta_event(
        deps.up,
        &dataset_id,
        &token,
        &payload,
        test_event_code.as_deref(),
    )
    .await?;
    Ok(None)
}

async fn set_ads_status(
    db: &Db,
    event: &AdsConversionEvent,
    to: &'static str,
    sent_at: Option<i64>,
) -> std::result::Result<(), Retry> {
    let input = UpdateCapiStatusInput {
        id: event.id,
        workspace_id: event.workspace_id,
        from: "pending".into(),
        to: to.into(),
        capi_sent_at: sent_at,
    };
    tx::write(db, move |tx| {
        ads_events::update_capi_status(tx, &input)?;
        Ok(())
    })
    .await
    .map_err(|e| e.message())
}

/// `handleSendConversionEvent` (an `AdsConversionEvent` row). `Err` = retry.
pub async fn send_conversion_event<U: Upstream>(
    deps: &SendDeps<'_, U>,
    event_id: i64,
    workspace_id: i64,
) -> std::result::Result<(), Retry> {
    let db = deps.db;
    let event = tx::read(db, move |conn| {
        Ok(ads_events::find_workspace_event(
            conn,
            event_id,
            workspace_id,
        )?)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(event) = event.filter(|e| e.capi_status == "pending") else {
        return Ok(());
    };
    match event.channel.as_str() {
        "messenger" => send_meta_channel_conversion(deps, &event, CapiChannel::Messenger).await,
        "instagram" => send_meta_channel_conversion(deps, &event, CapiChannel::Instagram).await,
        _ => send_whatsapp_conversion(deps, &event).await,
    }
}

async fn send_whatsapp_conversion<U: Upstream>(
    deps: &SendDeps<'_, U>,
    event: &AdsConversionEvent,
) -> std::result::Result<(), Retry> {
    let db = deps.db;
    let (Some(integration_id), Some(ctwa_clid), Some(waba_id)) = (
        event.integration_whatsapp_id,
        event.ctwa_clid.clone(),
        event.waba_id.clone(),
    ) else {
        tracing::error!(
            ads_conversion_event_id = event.id,
            "AdsConversionEvent missing WhatsApp attribution fields; marking failed"
        );
        return set_ads_status(db, event, "failed", None).await;
    };
    let ws = event.workspace_id;
    let integration = tx::read(db, move |conn| {
        Ok(find_integration(
            conn,
            CapiChannel::Whatsapp,
            ws,
            integration_id,
        )?)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(integration) = integration else {
        return Ok(());
    };
    if integration.disconnected {
        return set_ads_status(db, event, "failed", None).await;
    }
    // `whatsappAuthForCapiScopeSchema.parse(integration.auth)` (outside the try).
    let Some(connect_token) = integration.oauth_token() else {
        return Err("WhatsApp integration auth is invalid".into());
    };
    let probe = integration.clone();
    let (token, _) = tx::read(db, move |conn| Ok(probe.access_token(conn)))
        .await
        .map_err(|e| e.message())?
        .map_err(|e| e.message)?;
    let ldu = tx::read(db, move |conn| Ok(workspace_ldu(conn, ws)?))
        .await
        .map_err(|e| e.message())?;
    let version = integration.auth["version"]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(CHANNEL_API_VERSION)
        .to_owned();
    let attempt = async {
        let dataset_id = ensure_dataset_id(db, deps.up, &integration, &connect_token).await?;
        let user_data = match event.contact_inbox_id {
            Some(cib) => tx::read(db, move |conn| Ok(load_inbox(conn, ws, cib)?))
                .await?
                .and_then(|(inbox, ..)| inbox)
                .filter(|i| i.inbox_id == integration.inbox_id)
                .map(|i| hash_contact_user_data(&i.contact)),
            None => None,
        };
        let payload = WhatsappEvent {
            event_type: event.event_type.clone(),
            occurred_at: event.occurred_at,
            source_event_id: event.source_event_id.clone(),
            ctwa_clid,
            waba_id,
            currency: event.currency.clone(),
            value: event.value.clone(),
            automatic: event.source == "automatic",
            user_data,
            limited_data_use: ldu,
            order_id: event.order_id.clone(),
            contents: event
                .contents
                .as_ref()
                .and_then(|c| c.0.as_array().cloned()),
        };
        send_whatsapp_event(deps.up, &version, &dataset_id, &token, &payload).await
    };
    match attempt.await {
        Ok(()) => set_ads_status(db, event, "sent", Some(now_ms())).await,
        Err(error) if error.retryable => Err(error.message),
        Err(error) => {
            set_ads_status(db, event, "failed", None).await?;
            log_failure(deps.events, "whatsapp", ws, None, &error.message).await;
            tracing::warn!(
                ads_conversion_event_id = event.id,
                workspace_id = ws,
                message = error.message,
                "Ads conversion event marked failed"
            );
            Ok(())
        }
    }
}

async fn send_meta_channel_conversion<U: Upstream>(
    deps: &SendDeps<'_, U>,
    event: &AdsConversionEvent,
    channel: CapiChannel,
) -> std::result::Result<(), Retry> {
    let db = deps.db;
    let integration_id = match channel {
        CapiChannel::Messenger => event.integration_messenger_id,
        _ => event.integration_instagram_id,
    };
    let Some(integration_id) = integration_id else {
        tracing::error!(
            ads_conversion_event_id = event.id,
            "AdsConversionEvent missing integration id; marking failed"
        );
        return set_ads_status(db, event, "failed", None).await;
    };
    let ws = event.workspace_id;
    let integration = tx::read(db, move |conn| {
        Ok(find_integration(conn, channel, ws, integration_id)?)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(integration) = integration.filter(|i| !i.disconnected) else {
        tracing::warn!(
            ads_conversion_event_id = event.id,
            "AdsConversionEvent integration not found or disconnected; marking failed"
        );
        return set_ads_status(db, event, "failed", None).await;
    };
    let Some(cib) = event.contact_inbox_id else {
        return set_ads_status(db, event, "failed", None).await;
    };
    let loaded = tx::read(db, move |conn| Ok(load_inbox(conn, ws, cib)?))
        .await
        .map_err(|e| e.message())?;
    let Some(inbox) = loaded
        .and_then(|(inbox, ..)| inbox)
        .filter(|i| i.inbox_id == integration.inbox_id)
    else {
        tracing::error!(
            ads_conversion_event_id = event.id,
            "AdsConversionEvent contact inbox missing or workspace/inbox mismatch; marking failed"
        );
        return set_ads_status(db, event, "failed", None).await;
    };
    let ldu = tx::read(db, move |conn| Ok(workspace_ldu(conn, ws)?))
        .await
        .map_err(|e| e.message())?;
    let attempt = async {
        let probe = integration.clone();
        let (token, source) = tx::read(db, move |conn| Ok(probe.access_token(conn))).await??;
        let integration = if source == TokenSource::Manual {
            integration.clone()
        } else {
            integration.assert_supported()?;
            refresh_scope_cache(db, deps.up, integration.clone()).await?
        };
        if (source == TokenSource::Manual && integration.dataset_id.is_none())
            || (source != TokenSource::Manual && !integration.has_capi_scope)
        {
            return Ok(Some("skipped_no_scope"));
        }
        integration.assert_supported()?;
        let dataset_id = match (&integration.dataset_id, source) {
            (Some(id), TokenSource::Manual) => id.clone(),
            _ => ensure_dataset_id(db, deps.up, &integration, &token).await?,
        };
        let identity = match channel {
            CapiChannel::Messenger => Identity::Messenger {
                page_id: integration.resource_id.clone(),
                psid: inbox.source_id.clone(),
            },
            _ => Identity::Instagram {
                account_id: integration.resource_id.clone(),
                igsid: inbox.source_id.clone(),
            },
        };
        let payload = MetaEvent {
            event_name: if event.event_type == "purchase" {
                "Purchase".into()
            } else {
                "LeadSubmitted".into()
            },
            occurred_at: event.occurred_at,
            event_id: event.source_event_id.clone(),
            identity: Some(identity),
            user_data: Some(hash_contact_user_data(&inbox.contact)),
            currency: event.currency.clone(),
            value: event.value.clone(),
            limited_data_use: ldu,
            order_id: event.order_id.clone(),
            contents: event
                .contents
                .as_ref()
                .and_then(|c| c.0.as_array().cloned()),
            ..MetaEvent::default()
        };
        send_meta_event(deps.up, &dataset_id, &token, &payload, None).await?;
        Ok::<_, CapiError>(None)
    };
    match attempt.await {
        Ok(Some(status)) => set_ads_status(db, event, status, None).await,
        Ok(None) => set_ads_status(db, event, "sent", Some(now_ms())).await,
        Err(error) if error.retryable => Err(error.message),
        Err(error) => {
            set_ads_status(db, event, "failed", None).await?;
            log_failure(
                deps.events,
                "meta-conversions",
                ws,
                Some((inbox.contact.id.clone(), inbox.source_id.clone())),
                &error.message,
            )
            .await;
            tracing::warn!(
                ads_conversion_event_id = event.id,
                workspace_id = ws,
                message = error.message,
                "Ads conversion event marked failed"
            );
            Ok(())
        }
    }
}

// ---- enqueueEvent -------------------------------------------------------------

/// `buildSourceKey`: per contact per UTC day for an ad-click keyed event,
/// else unique per fire.
pub fn build_source_key(
    scope: &str,
    scope_id: &str,
    contact_inbox_id: i64,
    channel: CapiChannel,
    action_source: Option<&str>,
) -> String {
    let segment = if keyed_to_ad_click(channel, action_source.unwrap_or("business_messaging")) {
        iso(now_ms())[..10].to_owned()
    } else {
        chatbotx_core::utils::id::create_id()
    };
    format!("{scope}:{scope_id}:{contact_inbox_id}:{segment}")
}

/// `EnqueueEventInput` (the `{{variable}}` fields already resolved).
#[derive(Debug, Clone, Default)]
pub struct EnqueueInput {
    pub workspace_id: i64,
    pub channel: String,
    pub contact_inbox_id: i64,
    pub inbox_id: i64,
    /// `flowStep` | `triggerAction` | `manualTest`.
    pub source: String,
    pub source_key: String,
    pub event_name: Option<String>,
    pub action_source: Option<String>,
    pub content_type: Option<String>,
    /// A comma-separated string (`splitContentIds`).
    pub content_ids: Option<String>,
    pub value: Option<String>,
    pub currency: Option<String>,
    pub content_category: Option<String>,
    pub content_name: Option<String>,
}

/// Why `enqueue_event` refused.
#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueError {
    /// The `enqueueEventInput` issues, as `z.prettifyError` lines.
    Invalid(String),
    Failed(String),
}

impl EnqueueError {
    pub fn message(&self) -> &str {
        match self {
            EnqueueError::Invalid(m) | EnqueueError::Failed(m) => m,
        }
    }
}

/// `splitContentIds`.
pub fn split_content_ids(value: &str) -> Option<Vec<String>> {
    let ids: Vec<String> = value
        .split(',')
        .map(|id| js::trim(id).to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    (!ids.is_empty()).then_some(ids)
}

struct Parsed {
    event_name: String,
    action_source: String,
    content_type: Option<String>,
    content_ids: Option<Vec<String>>,
    value: Option<String>,
    currency: Option<String>,
    content_category: Option<String>,
    content_name: Option<String>,
}

/// The CAPI fields of `enqueueEventInput` (`withMetaCapiEventRefinements`).
fn enqueue_fields_schema() -> chatbotx_core::flow_config::z::Schema {
    use chatbotx_core::flow_config::{deps, z};
    chatbotx_core::flow_config::steps::with_meta_capi_event_refinements(z::object(vec![
        (
            "eventName",
            deps::meta_capi_event_name_schema().default(json!("LeadSubmitted")),
        ),
        (
            "actionSource",
            deps::meta_capi_action_source_schema().default(json!("business_messaging")),
        ),
        (
            "contentType",
            deps::meta_capi_content_type_schema().optional(),
        ),
        ("contentIds", z::array(z::string().min(1)).min(1).optional()),
        ("value", deps::meta_capi_value_schema().optional()),
        ("currency", deps::meta_capi_currency_schema().optional()),
        (
            "contentCategory",
            z::string().trim().min(1).max(200).optional(),
        ),
        ("contentName", z::string().trim().min(1).max(200).optional()),
    ]))
}

/// `z.prettifyError`: issues by path length, `✖ message` + `→ at path`.
pub fn prettify(issues: &[chatbotx_core::flow_config::z::Issue]) -> String {
    let mut sorted: Vec<_> = issues.iter().collect();
    sorted.sort_by_key(|i| i.path.len());
    let mut lines = Vec::new();
    for issue in sorted {
        lines.push(format!("✖ {}", issue.message));
        if !issue.path.is_empty() {
            lines.push(format!("  → at {}", issue.path_string()));
        }
    }
    lines.join("\n")
}

/// `enqueueEventInput.parse` (the CAPI fields).
fn parse_enqueue(input: &EnqueueInput) -> std::result::Result<Parsed, String> {
    let mut fields = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            fields.insert(key.into(), value);
        }
    };
    put("eventName", input.event_name.clone().map(Value::String));
    put(
        "actionSource",
        input.action_source.clone().map(Value::String),
    );
    put("contentType", input.content_type.clone().map(Value::String));
    put(
        "contentIds",
        input
            .content_ids
            .as_deref()
            .and_then(split_content_ids)
            .map(|ids| json!(ids)),
    );
    put("value", input.value.clone().map(Value::String));
    put("currency", input.currency.clone().map(Value::String));
    put(
        "contentCategory",
        input.content_category.clone().map(Value::String),
    );
    put("contentName", input.content_name.clone().map(Value::String));
    let result = enqueue_fields_schema().safe_parse(&Value::Object(fields));
    let Some(data) = result.data.clone() else {
        return Err(prettify(result.issues()));
    };
    let text = |key: &str| data[key].as_str().map(str::to_owned);
    Ok(Parsed {
        event_name: text("eventName").unwrap_or_else(|| "LeadSubmitted".into()),
        action_source: text("actionSource").unwrap_or_else(|| "business_messaging".into()),
        content_type: text("contentType"),
        content_ids: data["contentIds"].as_array().map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        }),
        value: text("value"),
        currency: text("currency"),
        content_category: text("contentCategory"),
        content_name: text("contentName"),
    })
}

/// `enqueueSendMetaCapiEvent`.
async fn enqueue_send(jobs: &dyn Enqueuer, event: &MetaCapiEvent) -> Result<()> {
    chatbotx_jobs::queues::integration::enqueue_integration_job(
        jobs,
        chatbotx_jobs::queues::integration::IntegrationJob {
            action: chatbotx_jobs::queues::integration::IntegrationJobAction::SendMetaCapiEvent,
            data: json!({
                "metaCapiEventId": event.id.to_string(),
                "workspaceId": event.workspace_id.to_string(),
            }),
        },
        Some(chatbotx_jobs::JobOptions {
            job_id: Some(format!("meta-capi-send-{}", event.id)),
            ..chatbotx_jobs::JobOptions::default()
        }),
    )
    .await
    .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    Ok(())
}

/// `metaConversionsService.enqueueEvent`: one `MetaCapiEvent` row (deduped
/// by `sourceKey`) and its send job; a pending duplicate is re-enqueued.
pub async fn enqueue_event(
    db: &Db,
    jobs: &dyn Enqueuer,
    input: EnqueueInput,
) -> std::result::Result<Option<MetaCapiEvent>, EnqueueError> {
    let parsed = parse_enqueue(&input).map_err(EnqueueError::Invalid)?;
    let channel = capi_channel(&input.channel)
        .ok_or_else(|| EnqueueError::Invalid("✖ Invalid option\n  → at channel".into()))?;
    let failed = |e: BusinessError| EnqueueError::Failed(e.message());
    let (ws, inbox) = (input.workspace_id, input.inbox_id);
    let integration = tx::read(db, move |conn| {
        Ok(find_integration_by_inbox(conn, channel, ws, inbox)?)
    })
    .await
    .map_err(failed)?
    .ok_or_else(|| EnqueueError::Failed("Integration not found".into()))?;
    integration
        .assert_supported()
        .map_err(|e| EnqueueError::Failed(e.message))?;
    let now = now_ms();
    let opt = |v: Option<String>| v.map_or(Sql::Null, Sql::Text);
    let values: chatbotx_db::repo::patch::Patch = vec![
        ("workspaceId", Sql::Integer(ws)),
        ("channel", Sql::Text(input.channel.clone())),
        ("integrationId", Sql::Integer(integration.id)),
        ("contactInboxId", Sql::Integer(input.contact_inbox_id)),
        ("eventName", Sql::Text(parsed.event_name)),
        ("actionSource", Sql::Text(parsed.action_source)),
        ("contentType", opt(parsed.content_type)),
        (
            "contentIds",
            parsed
                .content_ids
                .map_or(Sql::Null, |ids| Sql::Text(json!(ids).to_string())),
        ),
        ("currency", opt(parsed.currency)),
        ("contentCategory", opt(parsed.content_category)),
        ("contentName", opt(parsed.content_name)),
        ("value", opt(parsed.value)),
        ("source", Sql::Text(input.source.clone())),
        ("sourceKey", Sql::Text(input.source_key.clone())),
        ("occurredAt", Sql::Integer(now)),
        ("capiStatus", Sql::Text("pending".into())),
    ];
    let (channel_name, key) = (input.channel.clone(), input.source_key.clone());
    let (inserted, existing) = tx::write(db, move |tx| {
        if let Some(row) = meta_capi_event::insert_ignore_duplicate(tx, &values)? {
            return Ok((Some(row), None));
        }
        Ok((
            None,
            meta_capi_event::find_pending_by_source_key(tx, ws, &channel_name, &key)?,
        ))
    })
    .await
    .map_err(failed)?;
    if let Some(row) = &inserted {
        enqueue_send(jobs, row).await.map_err(failed)?;
    } else if let Some(row) = &existing {
        enqueue_send(jobs, row).await.map_err(failed)?;
    }
    Ok(inserted)
}

/// `reportCapiInputFailure` (`provider: "meta-conversions"`, no HTTP code).
pub async fn report_input_failure(
    events: Option<&EventBuses>,
    workspace_id: i64,
    contact_id: i64,
    source_id: &str,
    message: String,
) {
    let mut input = ProviderErrorInput::new(
        "meta-conversions",
        &workspace_id.to_string(),
        Thrown::Error {
            message,
            stack: None,
            status: None,
        },
    );
    input.contact_id = Some(contact_id.to_string());
    input.source_id = Some(source_id.to_owned());
    input.http_code = Some(None);
    log_provider_error(events, input).await;
}

/// `describeCapiInputValidationError`: the issues plus the non-empty
/// resolved values.
pub fn describe_input_failure(issues: &str, resolved: &[(&str, Option<&str>)]) -> String {
    let entries: Vec<String> = resolved
        .iter()
        .filter_map(|(field, value)| {
            value
                .filter(|v| !v.is_empty())
                .map(|v| format!("{field}={}", Value::String(v.to_owned())))
        })
        .collect();
    if entries.is_empty() {
        issues.to_owned()
    } else {
        format!("{issues}\nResolved: {}", entries.join(", "))
    }
}
