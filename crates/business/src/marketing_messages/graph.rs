//! The Graph calls of Marketing Messages (`integration-facebook-ads`
//! `apis/{ad-accounts,message-campaigns,marketing-messages-send}.ts`,
//! `integration-messenger` `debugToken`) and `classifyMarketingMessageError`.
//! ponytail: no GET retry (ky retries 408/429/5xx three times); the callers
//! already treat a transport error as retryable.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use crate::errors::{BusinessError, sanitize_public_text};

const GRAPH: &str = "https://graph.facebook.com";
/// `DEFAULT_API_VERSION`.
pub const VERSION: &str = "v23.0";
/// `GRAPH_ERROR_CODE_INVALID_TOKEN`.
pub const INVALID_TOKEN: i64 = 190;
const PAGE_LIMIT: &str = "499";
const MAX_PAGES: usize = 20;
/// `CHANNEL_ERROR_FALLBACK` of `toKnownOrpcError`.
const FALLBACK: &str = "The provider rejected the request.";

/// `FacebookAdsException`: a failed Graph call.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphError {
    /// HTTP status of the answer (400 when there was none).
    pub status: u16,
    pub code: Option<i64>,
    pub subcode: Option<String>,
    pub message: String,
    /// No answer arrived (timeout / network).
    pub network: bool,
}

impl GraphError {
    /// `isGraphTransportError`.
    pub fn is_transport(&self) -> bool {
        self.network || self.status >= 500
    }

    fn invalid(message: &str) -> GraphError {
        GraphError {
            status: 400,
            code: None,
            subcode: None,
            message: message.to_owned(),
            network: false,
        }
    }
}

/// `toKnownOrpcError` of an `SdkException`: `BAD_REQUEST` with its status.
impl From<GraphError> for BusinessError {
    fn from(error: GraphError) -> BusinessError {
        let message = sanitize_public_text(&error.message);
        let message = if message.is_empty() {
            FALLBACK.to_owned()
        } else {
            message
        };
        BusinessError::exception(message, "BAD_REQUEST", error.status)
    }
}

static TOKEN_QUERY_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(access_token|appsecret_proof)=[^&\s"'<>)]*"#).expect("valid regex")
});

/// `redactTokens`.
pub fn redact_tokens(message: &str) -> String {
    TOKEN_QUERY_PARAM
        .replace_all(message, "$1=[REDACTED]")
        .into_owned()
}

fn url(path: &str, query: &[(&str, &str)]) -> String {
    let mut out = format!("{GRAPH}/{path}");
    if !query.is_empty() {
        out.push('?');
        out.push_str(
            &url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query)
                .finish(),
        );
    }
    out
}

/// One percent-encoded path segment (an id from a row or the request).
pub fn seg(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}

/// The raw answer of one call: `Ok((status, body))` or a network failure.
async fn send(
    method: &'static str,
    url: String,
    body: Option<&Value>,
    bearer: Option<&str>,
) -> std::result::Result<(u16, String), String> {
    let headers = bearer
        .map(|t| vec![("authorization".to_owned(), format!("Bearer {t}"))])
        .unwrap_or_default();
    HttpUpstream
        .send(UpstreamRequest {
            method,
            url,
            headers,
            body: body.map(Value::to_string),
        })
        .await
}

/// `rescue`: a non-2xx answer or no answer is a [`GraphError`].
async fn call(
    method: &'static str,
    url: String,
    body: Option<&Value>,
    bearer: Option<&str>,
) -> std::result::Result<Value, GraphError> {
    match send(method, url, body, bearer).await {
        Ok((status, text)) if (200..300).contains(&status) => {
            serde_json::from_str(&text).map_err(|_| GraphError::invalid("Invalid Graph response"))
        }
        Ok((status, text)) => {
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let error = body.get("error");
            Err(GraphError {
                status,
                code: error.and_then(|e| e["code"].as_i64()),
                subcode: error.and_then(|e| match &e["error_subcode"] {
                    Value::Number(n) => Some(n.to_string()),
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                }),
                message: redact_tokens(
                    &chatbotx_core::utils::graph_error::format_graph_error(error)
                        .unwrap_or_else(|| "Unknown error".to_owned()),
                ),
                network: false,
            })
        }
        Err(message) => Err(GraphError {
            status: 400,
            code: None,
            subcode: None,
            message: redact_tokens(&message),
            network: true,
        }),
    }
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    match &value[key] {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        _ => None,
    }
}

/// `debugToken` (app token as bearer): the `data` object.
pub async fn debug_token(
    input_token: &str,
    app_access_token: &str,
    version: &str,
) -> std::result::Result<Value, GraphError> {
    let body = call(
        "GET",
        url(
            &format!("{version}/debug_token"),
            &[("input_token", input_token)],
        ),
        None,
        Some(app_access_token),
    )
    .await?;
    Ok(body["data"].clone())
}

/// `hasMarketingMessages(scopes)`.
pub fn has_marketing_messages(data: &Value) -> bool {
    data["scopes"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == "marketing_messages_messenger"))
}

/// `MarketingMessagesAdAccount`.
#[derive(Debug, Clone, PartialEq)]
pub struct AdAccount {
    pub id: String,
    pub account_id: String,
    pub name: Option<String>,
    pub currency: String,
    pub timezone_name: Option<String>,
    pub tos_accepted: Map<String, Value>,
}

/// `fetchAllPages`: follows `paging.cursors.after` while `paging.next` is set.
async fn all_pages(
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Vec<Value>, GraphError> {
    let mut rows = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut q = query.to_vec();
        if let Some(after) = &after {
            q.push(("after", after.as_str()));
        }
        let page = call("GET", url(path, &q), None, None).await?;
        rows.extend(page["data"].as_array().cloned().unwrap_or_default());
        after = page["paging"]["next"]
            .as_str()
            .and(page["paging"]["cursors"]["after"].as_str())
            .map(str::to_owned);
        if after.is_none() {
            break;
        }
    }
    Ok(rows)
}

/// `getMarketingMessagesAdAccounts`.
pub async fn mm_ad_accounts(
    token: &str,
    version: &str,
) -> std::result::Result<Vec<AdAccount>, GraphError> {
    let rows = all_pages(
        &format!("{version}/me/adaccounts"),
        &[
            (
                "fields",
                "name,account_id,currency,timezone_name,tos_accepted",
            ),
            ("limit", PAGE_LIMIT),
            ("access_token", token),
        ],
    )
    .await?;
    Ok(rows
        .iter()
        .map(|row| AdAccount {
            id: row["id"].as_str().unwrap_or_default().to_owned(),
            account_id: row["account_id"].as_str().unwrap_or_default().to_owned(),
            name: row["name"].as_str().map(str::to_owned),
            currency: row["currency"].as_str().unwrap_or_default().to_owned(),
            timezone_name: row["timezone_name"].as_str().map(str::to_owned),
            tos_accepted: row["tos_accepted"].as_object().cloned().unwrap_or_default(),
        })
        .collect())
}

/// `isCustomAudienceTosAccepted`.
pub fn tos_accepted(tos: &Map<String, Value>) -> bool {
    ["custom_audience_tos", "web_custom_audience_tos"]
        .iter()
        .any(|k| tos.get(*k).and_then(Value::as_f64).unwrap_or(0.0) > 0.0)
}

/// `customAudienceTosUrl`.
pub fn tos_url(account_id: &str) -> String {
    format!(
        "https://business.facebook.com/ads/manage/customaudiences/tos/?act={}",
        account_id.strip_prefix("act_").unwrap_or(account_id)
    )
}

/// `toAdAccountOption`.
pub fn ad_account_option(account: &AdAccount) -> Value {
    json!({
        "id": account.id,
        "accountId": account.account_id,
        "name": account.name.clone().unwrap_or_else(|| account.id.clone()),
        "currency": account.currency,
        "tosAccepted": tos_accepted(&account.tos_accepted),
        "tosUrl": tos_url(&account.account_id),
    })
}

fn budget_field(budget_type: &str, minor: i64) -> (&'static str, Value) {
    if budget_type == "daily" {
        ("daily_budget", json!(minor))
    } else {
        ("lifetime_budget", json!(minor))
    }
}

/// `createMessageCampaign`: the id of the created ad.
pub async fn create_message_campaign(
    token: &str,
    version: &str,
    ad_account_id: &str,
    name: &str,
    page_id: &str,
    budget_type: &str,
    budget_minor_units: i64,
) -> std::result::Result<String, GraphError> {
    let (key, value) = budget_field(budget_type, budget_minor_units);
    let mut body = json!({ "access_token": token, "name": name, "page_id": page_id });
    body[key] = value;
    let response = call(
        "POST",
        url(
            &format!("{version}/{}/message_campaign", seg(ad_account_id)),
            &[],
        ),
        Some(&body),
        None,
    )
    .await?;
    str_field(&response, "id").ok_or_else(|| GraphError::invalid("Invalid Graph response"))
}

/// `updateMessageCampaign`: the budget on the ad set first, then the ad's name.
pub async fn update_message_campaign(
    token: &str,
    version: &str,
    campaign_id: &str,
    name: &str,
    budget_type: &str,
    budget_minor_units: i64,
) -> std::result::Result<(), GraphError> {
    let node = call(
        "GET",
        url(
            &format!("{version}/{}", seg(campaign_id)),
            &[("access_token", token), ("fields", "adset{id}")],
        ),
        None,
        None,
    )
    .await?;
    let ad_set_id = str_field(&node["adset"], "id")
        .ok_or_else(|| GraphError::invalid("Invalid Graph response"))?;
    let (key, value) = budget_field(budget_type, budget_minor_units);
    let mut budget = json!({ "access_token": token });
    budget[key] = value;
    call(
        "POST",
        url(&format!("{version}/{}", seg(&ad_set_id)), &[]),
        Some(&budget),
        None,
    )
    .await?;
    call(
        "POST",
        url(&format!("{version}/{}", seg(campaign_id)), &[]),
        Some(&json!({ "access_token": token, "name": name })),
        None,
    )
    .await?;
    Ok(())
}

/// `deleteMessageCampaign`.
pub async fn delete_message_campaign(
    token: &str,
    campaign_id: &str,
) -> std::result::Result<(), GraphError> {
    call(
        "DELETE",
        url(
            &format!("{VERSION}/{}", seg(campaign_id)),
            &[("access_token", token)],
        ),
        None,
        None,
    )
    .await
    .map(drop)
}

/// `getMessageCampaignSchedule`: the ad set id and Meta's end time (ms).
pub async fn campaign_schedule(
    token: &str,
    campaign_id: &str,
) -> std::result::Result<(String, Option<i64>), GraphError> {
    let node = call(
        "GET",
        url(
            &format!("{VERSION}/{}", seg(campaign_id)),
            &[("access_token", token), ("fields", "adset{id,end_time}")],
        ),
        None,
        None,
    )
    .await?;
    let ad_set_id = str_field(&node["adset"], "id")
        .ok_or_else(|| GraphError::invalid("Invalid Graph response"))?;
    let end = node["adset"]["end_time"]
        .as_str()
        .filter(|s| !s.is_empty())
        .and_then(chatbotx_core::js::parse_date);
    Ok((ad_set_id, end))
}

/// `getAdAccountDetails`: the ad account's time zone.
pub async fn ad_account_timezone(
    token: &str,
    ad_account_id: &str,
) -> std::result::Result<String, GraphError> {
    let body = call(
        "GET",
        url(
            &format!("{VERSION}/{}", seg(ad_account_id)),
            &[
                (
                    "fields",
                    "id,name,currency,timezone_name,account_status,min_daily_budget",
                ),
                ("access_token", token),
            ],
        ),
        None,
        None,
    )
    .await?;
    let valid = body["id"].is_string()
        && body["currency"].is_string()
        && body["account_status"].is_number();
    match body["timezone_name"].as_str() {
        Some(tz) if valid => Ok(tz.to_owned()),
        _ => Err(GraphError::invalid("Invalid Graph response")),
    }
}

fn act(ad_account_id: &str) -> String {
    if ad_account_id.starts_with("act_") {
        ad_account_id.to_owned()
    } else {
        format!("act_{ad_account_id}")
    }
}

/// `getMessageDeliveryEstimate`: the upper bound.
pub async fn delivery_estimate_upper(
    token: &str,
    ad_account_id: &str,
    page_id: &str,
    daily_budget_minor_units: i64,
) -> std::result::Result<i64, GraphError> {
    let targeting = json!({
        "messenger_positions": ["marketing_messages"],
        "publisher_platforms": ["messenger"],
    })
    .to_string();
    let promoted = json!({ "page_id": page_id }).to_string();
    let budget = daily_budget_minor_units.to_string();
    let body = call(
        "GET",
        url(
            &format!(
                "{VERSION}/{}/message_delivery_estimate",
                seg(&act(ad_account_id))
            ),
            &[
                ("access_token", token),
                (
                    "fields",
                    "estimate_delivery_lower_bound,estimate_delivery_upper_bound",
                ),
                ("targeting_spec", &targeting),
                ("promoted_object", &promoted),
                ("daily_budget", &budget),
                ("is_direct_send_campaign", "true"),
                ("pacing_type", "no_pacing"),
            ],
        ),
        None,
        None,
    )
    .await?;
    body["data"][0]["estimate_delivery_upper_bound"]
        .as_f64()
        .filter(|_| body["data"][0]["estimate_delivery_lower_bound"].is_number())
        .map(|n| n as i64)
        .ok_or_else(|| GraphError::invalid("Invalid Graph response"))
}

/// `uploadAttachment` (messenger, Page token): the reusable attachment id.
pub async fn upload_attachment(
    page_token: &str,
    version: &str,
    media_url: &str,
    media_type: &str,
) -> std::result::Result<Option<String>, GraphError> {
    let body = json!({ "message": { "attachment": { "type": media_type,
        "payload": { "is_reusable": true, "url": media_url } } } });
    let response = call(
        "POST",
        url(&format!("{version}/me/message_attachments"), &[]),
        Some(&body),
        Some(page_token),
    )
    .await?;
    Ok(id_text(&response["attachment_id"]).filter(|id| !id.is_empty()))
}

/// `NotificationMessageToken`.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationToken {
    pub token: String,
    pub topic_title: Option<String>,
    pub psid: Option<String>,
    pub next_eligible_at: Option<i64>,
}

fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `listNotificationMessageTokens` (Page token), following `paging.next`.
/// Only a `paging.next` on the Graph host is followed (it carries the token).
pub async fn notification_tokens(
    page_token: &str,
    page_id: &str,
    custom_audience_id: Option<&str>,
    version: &str,
) -> std::result::Result<Vec<NotificationToken>, GraphError> {
    let mut query = vec![("limit", "1000"), ("access_token", page_token)];
    if let Some(id) = custom_audience_id {
        query.push(("custom_audience_ids", id));
    }
    let mut next = Some(url(
        &format!("{version}/{}/notification_message_tokens", seg(page_id)),
        &query,
    ));
    let mut tokens = Vec::new();
    let no_rows = Vec::new();
    while let Some(page_url) = next.take() {
        let page = call("GET", page_url, None, None).await?;
        let Some(rows) = page.get("data").map_or(Some(&no_rows), Value::as_array) else {
            return Err(GraphError::invalid("Invalid Graph response"));
        };
        for row in rows {
            let Some(token) = row["notification_messages_token"]
                .as_str()
                .filter(|t| !t.is_empty())
            else {
                return Err(GraphError::invalid("Invalid Graph response"));
            };
            tokens.push(NotificationToken {
                token: token.to_owned(),
                topic_title: row["topic_title"].as_str().map(str::to_owned),
                psid: id_text(&row["recipient_id"]),
                next_eligible_at: row["next_eligible_time"]
                    .as_f64()
                    .map(|s| (s * 1000.0) as i64),
            });
        }
        next = page["paging"]["next"]
            .as_str()
            .filter(|n| n.starts_with(&format!("{GRAPH}/")))
            .map(str::to_owned);
    }
    Ok(tokens)
}

/// `SendMarketingMessageResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum SendResult {
    Ok(String),
    Meta {
        code: Option<i64>,
        status: u16,
        message: String,
    },
    Uncertain(String),
}

/// `sendMarketingMessage`: never retried; a 5xx / 408 / no answer may have
/// delivered the message, so it is [`SendResult::Uncertain`].
pub async fn send_marketing_message(
    token: &str,
    ad_account_id: &str,
    campaign_id: &str,
    subscription_token: &str,
    message: &Value,
    version: &str,
) -> SendResult {
    let body = json!({
        "access_token": token,
        "message_id": campaign_id,
        "messenger_delivery_data": { "subscription_token": subscription_token },
        "message": message,
    });
    let target = url(
        &format!("{version}/{}/messages", seg(&act(ad_account_id))),
        &[],
    );
    match send("POST", target, Some(&body), None).await {
        Ok((status, text)) if (200..300).contains(&status) => {
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            match id_text(&parsed["marketing_message_tracking_id"]) {
                Some(id) => SendResult::Ok(id),
                None => SendResult::Uncertain(
                    "Meta accepted the request but returned no tracking id".to_owned(),
                ),
            }
        }
        Ok((status, text)) if status < 500 && status != 408 => {
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let error = &parsed["error"];
            SendResult::Meta {
                code: error["code"].as_i64(),
                status,
                message: redact_tokens(
                    error["message"]
                        .as_str()
                        .map_or(format!("HTTP {status}"), str::to_owned)
                        .as_str(),
                ),
            }
        }
        Ok((status, _)) => SendResult::Uncertain(format!("HTTP {status}: HTTP {status}")),
        Err(message) => SendResult::Uncertain(redact_tokens(&message)),
    }
}

/// `MarketingMessageErrorClass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    DailyBudget,
    LifetimeBudget,
    NotReady,
    Transient,
    RecipientTiming,
    RecipientPermanent,
    CampaignBlocker,
    CampaignInactive,
    ContentInvalid,
    Unknown,
}

/// `classifyMarketingMessageError`.
pub fn classify(code: Option<i64>, http_status: Option<u16>) -> ErrorClass {
    use ErrorClass::*;
    let Some(code) = code else {
        return if http_status == Some(429) {
            Transient
        } else {
            Unknown
        };
    };
    match code {
        2_300_054 => DailyBudget,
        2_300_053 | 2_300_015 => LifetimeBudget,
        2_300_012 | 2_300_041 | 2_300_033 => NotReady,
        2_300_055
        | 2_300_057
        | 2_300_060..=2_300_063
        | 2_300_068
        | 2_300_078
        | 2_300_080..=2_300_083
        | 2_300_008
        | 4
        | 17
        | 32
        | 613 => Transient,
        2_300_035 | 2_300_036 | 2_300_042..=2_300_044 | 2_300_052 => RecipientTiming,
        2_300_004 | 2_300_005 | 2_300_032 | 2_300_034 | 2_300_037 => RecipientPermanent,
        2_300_000 | 2_300_002 | 2_300_003 => CampaignInactive,
        2_300_001
        | 2_300_006
        | 2_300_007
        | 2_300_009
        | 2_300_011
        | 2_300_013
        | 2_300_014
        | 2_300_016
        | 2_300_017
        | 2_300_019
        | 2_300_020
        | 2_300_030
        | 2_300_031
        | 2_300_039
        | 2_300_040
        | 2_300_056
        | 2_300_058
        | 2_300_059
        | 2_300_065..=2_300_067
        | 2_300_069
        | 2_300_070
        | 2_300_076
        | 2_300_091..=2_300_093
        | 190
        | 10
        | 200..=299
        | 368 => CampaignBlocker,
        2_300_010
        | 2_300_018
        | 2_300_021..=2_300_027
        | 2_300_038
        | 2_300_046..=2_300_048
        | 2_300_064
        | 2_300_071..=2_300_075
        | 2_300_077
        | 2_300_090 => ContentInvalid,
        _ => Unknown,
    }
}

/// `MARKETING_MESSAGES_AUDIENCE_SUBTYPE`.
const AUDIENCE_SUBTYPE: i64 = 1010;
/// `AUDIENCE_UPLOAD_BATCH_SIZE`.
const AUDIENCE_UPLOAD_BATCH: usize = 10_000;

/// `createMarketingMessagesAudience`: the new custom audience id.
pub async fn create_audience(
    token: &str,
    version: &str,
    ad_account_id: &str,
    page_id: &str,
    name: &str,
    description: Option<&str>,
) -> std::result::Result<String, GraphError> {
    let mut body = json!({
        "subtype": "CUSTOM",
        "customer_file_source": "USER_PROVIDED_ONLY",
        "name": name,
    });
    if let Some(d) = description.filter(|d| !d.is_empty()) {
        body["description"] = json!(d);
    }
    body["use_for_products"] = json!(["MARKETING_MESSAGES"]);
    body["subscription_info"] = json!(["MESSENGER"]);
    body["marketing_message_channels"] = json!({ "MESSENGER": page_id });
    let answer = call(
        "POST",
        url(
            &format!("{version}/{}/customaudiences", seg(ad_account_id)),
            &[("access_token", token)],
        ),
        Some(&body),
        None,
    )
    .await?;
    str_field(&answer, "id")
        .or_else(|| str_field(&answer["data"][0], "id"))
        .ok_or_else(|| GraphError::invalid("Invalid Graph response"))
}

/// `uploadAudienceUsersSession`: `(received, invalid)`; a failed batch is
/// `Err((received so far, invalid so far, error))` (`AudienceUploadError`).
pub async fn upload_audience_users(
    token: &str,
    version: &str,
    custom_audience_id: &str,
    schema: &[&str],
    data: &[Vec<String>],
) -> std::result::Result<(i64, i64), (i64, i64, GraphError)> {
    // A positive safe integer (`Math.random() * MAX_SAFE_INTEGER`).
    let session_id = chatbotx_db::repo::new_id().rem_euclid((1i64 << 53) - 1) + 1;
    let endpoint = url(
        &format!("{version}/{}/users", seg(custom_audience_id)),
        &[("access_token", token)],
    );
    let batches: Vec<_> = data.chunks(AUDIENCE_UPLOAD_BATCH).collect();
    let (mut received, mut invalid) = (0, 0);
    for (i, batch) in batches.iter().enumerate() {
        let body = json!({
            "payload": { "schema": schema, "data": batch },
            "session": {
                "session_id": session_id,
                "batch_seq": i + 1,
                "last_batch_flag": i + 1 == batches.len(),
                "estimated_num_total": data.len(),
            },
        });
        let answer = call("POST", endpoint.clone(), Some(&body), None)
            .await
            .map_err(|e| (received, invalid, e))?;
        received += answer["num_received"].as_i64().unwrap_or(0);
        invalid += answer["num_invalid_entries"].as_i64().unwrap_or(0);
    }
    Ok((received, invalid))
}

/// `getMarketingMessagesAudiences`: the raw Graph rows of the account.
pub async fn mm_audiences(
    token: &str,
    version: &str,
    ad_account_id: &str,
) -> std::result::Result<Vec<Value>, GraphError> {
    let filtering =
        json!([{ "field": "data_source.subtype", "operator": "IN", "value": [AUDIENCE_SUBTYPE] }])
            .to_string();
    all_pages(
        &format!("{version}/{}/customaudiences", seg(ad_account_id)),
        &[
            (
                "fields",
                "id,name,subtype,description,delivery_status,operation_status,approximate_count_lower_bound,approximate_count_upper_bound,messenger_marketing_messages_page",
            ),
            ("filtering", &filtering),
            ("limit", PAGE_LIMIT),
            ("access_token", token),
        ],
    )
    .await
}

/// `deleteCustomAudience`: `Ok(false)` when Meta no longer has it (100/33).
pub async fn delete_custom_audience(
    token: &str,
    version: &str,
    custom_audience_id: &str,
) -> std::result::Result<bool, GraphError> {
    let answer = call(
        "DELETE",
        url(
            &format!("{version}/{}", seg(custom_audience_id)),
            &[("access_token", token)],
        ),
        None,
        None,
    )
    .await;
    match answer {
        Ok(_) => Ok(true),
        Err(e) if e.code == Some(100) && e.subcode.as_deref() == Some("33") => Ok(false),
        Err(e) => Err(e),
    }
}

/// `getNotificationMessageTokenAudienceIds` (Page token).
pub async fn token_audience_ids(
    page_token: &str,
    version: &str,
    page_id: &str,
    custom_audience_id: &str,
) -> std::result::Result<Vec<String>, GraphError> {
    let answer = call(
        "GET",
        url(
            &format!("{version}/{}/notification_message_tokens", seg(page_id)),
            &[
                ("custom_audience_ids", custom_audience_id),
                ("limit", "1"),
                ("access_token", page_token),
            ],
        ),
        None,
        None,
    )
    .await?;
    Ok(answer["data"][0]["custom_audience_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect())
}

/// `ALL_AD_EFFECTIVE_STATUSES`.
const AD_EFFECTIVE_STATUSES: [&str; 12] = [
    "ACTIVE",
    "PAUSED",
    "DELETED",
    "PENDING_REVIEW",
    "DISAPPROVED",
    "PREAPPROVED",
    "PENDING_BILLING_INFO",
    "CAMPAIGN_PAUSED",
    "ARCHIVED",
    "ADSET_PAUSED",
    "IN_PROCESS",
    "WITH_ISSUES",
];

/// `METRIC_FIELDS` (output key, Graph field).
pub const METRIC_FIELDS: [(&str, &str); 7] = [
    ("delivered", "marketing_messages_delivered"),
    ("readRate", "marketing_messages_read_rate"),
    ("linkClicks", "marketing_messages_link_btn_click"),
    ("linkClickRate", "marketing_messages_link_btn_click_rate"),
    ("costPerClick", "marketing_messages_cost_per_link_btn_click"),
    ("costPerDelivery", "marketing_messages_cost_per_delivered"),
    ("spend", "marketing_messages_spend"),
];

/// `fetchInsightRows` (`level` = `ad` | `account`): the raw Graph rows.
pub async fn insights(
    token: &str,
    version: &str,
    ad_account_id: &str,
    ad_ids: &[String],
    since: &str,
    until: &str,
    level: &str,
) -> std::result::Result<Vec<Value>, GraphError> {
    let mut fields: Vec<&str> = if level == "ad" { vec!["ad_id"] } else { vec![] };
    fields.push("account_currency");
    fields.extend(METRIC_FIELDS.iter().map(|(_, f)| *f));
    let fields = fields.join(",");
    let time_range = json!({ "since": since, "until": until }).to_string();
    let filtering = json!([
        { "field": "ad.id", "operator": "IN", "value": ad_ids },
        { "field": "ad.effective_status", "operator": "IN", "value": AD_EFFECTIVE_STATUSES },
    ])
    .to_string();
    all_pages(
        &format!("{version}/{}/insights", seg(ad_account_id)),
        &[
            ("level", level),
            ("fields", &fields),
            ("time_range", &time_range),
            ("filtering", &filtering),
            ("limit", PAGE_LIMIT),
            ("access_token", token),
        ],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_codes_and_a_bare_429() {
        assert_eq!(classify(Some(2_300_054), None), ErrorClass::DailyBudget);
        assert_eq!(classify(Some(250), None), ErrorClass::CampaignBlocker);
        assert_eq!(classify(Some(2_300_072), None), ErrorClass::ContentInvalid);
        assert_eq!(classify(Some(613), None), ErrorClass::Transient);
        assert_eq!(classify(None, Some(429)), ErrorClass::Transient);
        assert_eq!(classify(None, Some(400)), ErrorClass::Unknown);
        assert_eq!(classify(Some(1), None), ErrorClass::Unknown);
    }

    #[test]
    fn redacts_tokens_and_reads_tos() {
        assert_eq!(
            redact_tokens("GET https://x/?access_token=abc&b=1 failed"),
            "GET https://x/?access_token=[REDACTED]&b=1 failed"
        );
        let tos = json!({ "web_custom_audience_tos": 1 });
        assert!(tos_accepted(tos.as_object().unwrap()));
        assert!(!tos_accepted(&Map::new()));
        assert_eq!(
            tos_url("act_12"),
            "https://business.facebook.com/ads/manage/customaudiences/tos/?act=12"
        );
    }
}
