//! The `webhook` queue (`webhook/worker.ts`): `evaluateWebhooks` jobs of the
//! `WebhookEventEmitter` run `WebhookMatcherService.findAndExecuteWebhooks`
//! — match the workspace's active webhooks against the event, build one
//! payload (`webhook-payload.builder.ts`) and POST it to every matched
//! endpoint (`WebhookExecutor`).

use std::sync::Arc;
use std::time::Duration;

use chatbotx_business::events::registry;
use chatbotx_business::net::ssrf_guard::assert_public_url;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{BoxFuture, JobCtx, JobError};
use serde_json::{Map, Value, json};

use crate::condition::{self, Condition, EvalContext};
use crate::{Deps, db_fail, fail, str_of};

/// `MAX_RETRIES` of `WebhookExecutor`.
const MAX_RETRIES: u32 = 3;
/// `RETRY_DELAY_MS`.
const RETRY_DELAY: Duration = Duration::from_millis(1000);
/// `AbortSignal.timeout(30_000)`.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Redirect hops re-checked by the SSRF guard (`fetchFollowingSafeRedirects`).
const MAX_REDIRECT_HOPS: u32 = 5;

/// Why one POST failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostError {
    /// `ECONNREFUSED` / `ENOTFOUND` / `ECONNRESET` / ...: retried.
    Connect(String),
    /// The 30 s abort.
    Timeout,
    /// Anything else (SSRF refusal, bad URL, TLS): not retried.
    Other(String),
}

/// The webhook transport: POST a JSON body, answer the final status.
pub trait WebhookHttp: Send + Sync {
    fn post<'a>(&'a self, url: &'a str, body: &'a str) -> BoxFuture<'a, Result<u16, PostError>>;
}

/// [`WebhookHttp`] on reqwest: `assertPublicUrl` before the request and
/// before every redirect hop (redirects are followed by hand; 301 / 302 /
/// 303 continue as a GET without body, 307 / 308 repeat the POST, as fetch).
pub struct LiveWebhookHttp {
    client: reqwest::Client,
}

impl Default for LiveWebhookHttp {
    fn default() -> LiveWebhookHttp {
        LiveWebhookHttp {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }
}

fn post_error(error: &reqwest::Error) -> PostError {
    if error.is_timeout() {
        PostError::Timeout
    } else if error.is_connect() {
        PostError::Connect(error.to_string())
    } else {
        PostError::Other(error.to_string())
    }
}

impl WebhookHttp for LiveWebhookHttp {
    fn post<'a>(&'a self, url: &'a str, body: &'a str) -> BoxFuture<'a, Result<u16, PostError>> {
        Box::pin(async move {
            let mut url = url.to_owned();
            let mut as_post = true;
            let mut hops_left = MAX_REDIRECT_HOPS;
            loop {
                assert_public_url(&url, "Webhook URL")
                    .await
                    .map_err(PostError::Other)?;
                let request = if as_post {
                    self.client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(body.to_owned())
                } else {
                    self.client.get(&url)
                };
                let response = request
                    .header("User-Agent", "AhaChat-Webhook/1.0")
                    .send()
                    .await
                    .map_err(|e| post_error(&e))?;
                let status = response.status().as_u16();
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .filter(|l| !l.is_empty());
                let (true, Some(location)) = ((300..400).contains(&status), location) else {
                    return Ok(status);
                };
                if hops_left == 0 {
                    return Err(PostError::Other("Too many redirects".into()));
                }
                hops_left -= 1;
                url = url::Url::parse(&url)
                    .and_then(|base| base.join(location))
                    .map_err(|e| PostError::Other(format!("Invalid redirect location: {e}")))?
                    .to_string();
                as_post = as_post && matches!(status, 307 | 308);
            }
        })
    }
}

/// The `webhook` queue processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let data = job.data.get("data").cloned().unwrap_or(Value::Null);
    let workspace_id = str_of(&data, "workspaceId").unwrap_or("").to_owned();
    if crate::is_blocked_workspace(&deps.db, &workspace_id).await? {
        return Ok(Value::Null);
    }
    if str_of(&job.data, "type") == Some("evaluateWebhooks") {
        find_and_execute(&deps, &data).await?;
    }
    Ok(Value::Null)
}

/// `WebhookEventData` from the job.
pub struct WebhookEvent {
    pub workspace_id: String,
    pub contact_id: String,
    pub event_type: String,
    pub metadata: Map<String, Value>,
    pub timestamp: Value,
}

impl WebhookEvent {
    pub fn from_job(data: &Value) -> WebhookEvent {
        let s = |k| str_of(data, k).unwrap_or("").to_owned();
        WebhookEvent {
            workspace_id: s("workspaceId"),
            contact_id: s("contactId"),
            event_type: s("eventType"),
            metadata: data
                .get("eventData")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            timestamp: data.get("timestamp").cloned().unwrap_or(Value::Null),
        }
    }
}

/// The builders that spread their server-generated metadata.
pub(crate) const SPREAD_EVENT_TYPES: &[&str] = &[
    "dateTimeBasedTrigger",
    "incomingCall",
    "missedAudioCall",
    "callEnded",
    "callRecorded",
    "callTranscribed",
];

/// `isMatchableEventType`: emitted types plus the scanner-verified
/// `dateTimeBasedTrigger`.
pub fn is_matchable(event_type: &str) -> bool {
    registry::is_emitted(event_type) || event_type == "dateTimeBasedTrigger"
}

/// `conditionListensToSource`.
pub fn condition_listens_to_source(
    event_type: &str,
    condition_source: Option<&str>,
    event_source: Option<&str>,
) -> bool {
    let Some(event_source) = event_source.filter(|s| !s.is_empty()) else {
        return true;
    };
    if condition_source == Some(event_source) {
        return true;
    }
    condition_source.is_none_or(str::is_empty) && registry::is_pipeline(event_type)
}

/// A webhook with its conditions (`WebhookWithConditions`).
pub struct ActiveWebhook {
    pub id: i64,
    pub url: String,
    pub conditions: Vec<Condition>,
}

async fn list_active_with_conditions(
    deps: &Deps,
    workspace_id: i64,
) -> Result<Vec<ActiveWebhook>, JobError> {
    deps.db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT w."id", w."url", c."type", c."sourceId", c."operator", c."value"
                   FROM "Webhook" w LEFT JOIN "Condition" c ON c."webhookId" = w."id"
                   WHERE w."workspaceId" = ?1 AND w."active" = 1 ORDER BY w."id", c."id""#,
            )?;
            let rows = stmt.query_map([workspace_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut out: Vec<ActiveWebhook> = Vec::new();
            for row in rows {
                let (id, url, r#type, source_id, operator, value) = row?;
                if out.last().is_none_or(|w| w.id != id) {
                    out.push(ActiveWebhook {
                        id,
                        url,
                        conditions: Vec::new(),
                    });
                }
                if let (Some(r#type), Some(last)) = (r#type, out.last_mut()) {
                    last.conditions.push(Condition {
                        r#type,
                        source_id,
                        operator,
                        value: value.and_then(|v| serde_json::from_str(&v).ok()),
                    });
                }
            }
            Ok(out)
        })
        .await
        .map_err(db_fail)
}

/// The workspace's zone; `None` = no workspace row.
pub(crate) async fn workspace_timezone(
    deps: &Deps,
    workspace_id: i64,
) -> Result<Option<String>, JobError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [workspace_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// `findAndExecuteWebhooks`. Returns the ids of the webhooks it delivered to.
pub async fn find_and_execute(deps: &Deps, data: &Value) -> Result<Vec<i64>, JobError> {
    let event = WebhookEvent::from_job(data);
    if !is_matchable(&event.event_type) {
        return Ok(Vec::new());
    }
    let Ok(workspace_id) = event.workspace_id.parse::<i64>() else {
        return Ok(Vec::new());
    };
    let source_id = event.metadata.get("sourceId").and_then(Value::as_str);
    let webhooks: Vec<ActiveWebhook> = list_active_with_conditions(deps, workspace_id)
        .await?
        .into_iter()
        .filter(|w| {
            w.conditions.iter().any(|c| {
                c.r#type == event.event_type
                    && condition_listens_to_source(
                        &event.event_type,
                        c.source_id.as_deref(),
                        source_id,
                    )
            })
        })
        .collect();
    if webhooks.is_empty() {
        return Ok(Vec::new());
    }
    let Some(timezone) = workspace_timezone(deps, workspace_id).await? else {
        return Ok(Vec::new());
    };
    let mut matched = Vec::new();
    for webhook in webhooks {
        match webhook_matches(deps, &webhook, &event, &timezone).await {
            Ok(true) => matched.push(webhook),
            Ok(false) => {}
            Err(err) => tracing::error!(
                err = %err,
                "Failed to evaluate conditions for webhook {} in workspace {}",
                webhook.id,
                event.workspace_id
            ),
        }
    }
    if matched.is_empty() {
        return Ok(Vec::new());
    }
    // Built once, before the first delivery: a failure fails the job so the
    // queue retries the event (nothing was sent yet).
    let payload = Arc::new(build_payload(deps, &event).await?.to_string());
    let mut set = tokio::task::JoinSet::new();
    let ids: Vec<i64> = matched.iter().map(|w| w.id).collect();
    for webhook in matched {
        let http = Arc::clone(&deps.webhook_http);
        let payload = Arc::clone(&payload);
        set.spawn(async move { execute(&*http, &webhook, &payload).await });
    }
    while set.join_next().await.is_some() {}
    Ok(ids)
}

/// `evaluateWebhookConditions`: any condition of this event's type matches.
async fn webhook_matches(
    deps: &Deps,
    webhook: &ActiveWebhook,
    event: &WebhookEvent,
    timezone: &str,
) -> Result<bool, String> {
    for c in &webhook.conditions {
        if event.event_type == "dateTimeBasedTrigger" {
            // Scanner-verified: the scanner already checked the time.
            if c.r#type == event.event_type
                && c.source_id.as_deref() == event.metadata.get("sourceId").and_then(Value::as_str)
            {
                return Ok(true);
            }
            continue;
        }
        if c.r#type != event.event_type {
            continue;
        }
        let ctx = EvalContext {
            metadata: &event.metadata,
            contact_id: &event.contact_id,
            workspace_timezone: Some(timezone),
            now_ms: chatbotx_db::repo::now_ms(),
        };
        if condition::evaluate(&deps.db, c, &ctx).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `WebhookExecutor.execute`: up to 3 attempts, retrying only connection
/// errors (1 s, 2 s); a non-2xx answer or a timeout is logged and final.
pub async fn execute(http: &dyn WebhookHttp, webhook: &ActiveWebhook, payload: &str) {
    for attempt in 1..=MAX_RETRIES {
        match http.post(&webhook.url, payload).await {
            Ok(status) => {
                if !(200..300).contains(&status) {
                    tracing::warn!(webhook_id = webhook.id, url = %webhook.url, status, attempt, "Webhook endpoint returned non-2xx response");
                }
                return;
            }
            Err(PostError::Timeout) => {
                tracing::warn!(webhook_id = webhook.id, url = %webhook.url, attempt, "Webhook endpoint timed out");
                return;
            }
            Err(PostError::Other(err)) => {
                tracing::warn!(err = %err, webhook_id = webhook.id, url = %webhook.url, attempt, "Webhook request failed without retry");
                return;
            }
            Err(PostError::Connect(err)) => {
                tracing::warn!(err = %err, webhook_id = webhook.id, url = %webhook.url, attempt, "Webhook connection failed");
            }
        }
        if attempt < MAX_RETRIES {
            tracing::warn!(webhook_id = webhook.id, url = %webhook.url, attempt, max_retries = MAX_RETRIES, "Retrying webhook after connection failure");
            tokio::time::sleep(RETRY_DELAY * attempt).await;
        }
    }
    tracing::error!(webhook_id = webhook.id, url = %webhook.url, "Webhook failed after {MAX_RETRIES} retries");
}

/// `EVENT_NAMES`.
fn event_name(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        "tagApplied" => "tag_applied",
        "tagRemoved" => "tag_removed",
        "customFieldValueChanged" => "custom_field_changed",
        "contactInfoUpdated" => "contact_info_updated",
        "conversationTransferredToHuman" => "conversation_transferred_to_human",
        "conversationTransferredToBot" => "conversation_transferred_to_bot",
        "newContact" => "new_contact",
        "contactUnsubscribedFormBroadcast" => "contact_unsubscribed",
        "archived" => "conversation_archived",
        "followUp" => "marked_as_follow_up",
        "conversationAssigned" => "conversation_assigned",
        "conversationUnassigned" => "conversation_unassigned",
        "subscribedToSequence" => "subscribed_to_sequence",
        "unsubscribedFromSequence" => "unsubscribed_from_sequence",
        "contactReferredANewContact" => "contact_referred_a_new_contact",
        "contactReferredExistingContact" => "contact_referred_existing_contact",
        "contactBlockedBot" => "contact_blocked_bot",
        "contactUnblockedBot" => "contact_unblocked_bot",
        "dateTimeBasedTrigger" => "datetime_based_trigger",
        "incomingCall" => "incoming_call",
        "missedAudioCall" => "missed_audio_call",
        "callEnded" => "call_ended",
        "callRecorded" => "call_recorded",
        "callTranscribed" => "call_transcribed",
        "shoppingCartSent" => "shopping_cart_sent",
        "userAskedAboutProduct" => "user_asked_about_product",
        "newOrder" => "new_order",
        "orderAccepted" => "order_accepted",
        "orderShipped" => "order_shipped",
        "orderConcluded" => "order_concluded",
        "orderCancelled" => "order_cancelled",
        "orderPaid" => "order_paid",
        "orderRefunded" => "order_refunded",
        "productOrdered" => "product_ordered",
        "productAddedToCart" => "product_added_to_cart",
        "productRemovedFromCart" => "product_removed_from_cart",
        "categoryAddedToCart" => "category_added_to_cart",
        "cartAbandoned" => "cart_abandoned",
        "pipelineItemCreated" => "pipeline_item_created",
        "pipelineItemMovedToStage" => "pipeline_item_moved_to_stage",
        "pipelineItemValueChanged" => "pipeline_item_value_changed",
        "pipelineItemStatusChanged" => "pipeline_item_status_changed",
        "pipelineItemPriorityChanged" => "pipeline_item_priority_changed",
        _ => return None,
    })
}

/// `value as string` that is truthy, else `fallback`.
fn or_str(data: &Map<String, Value>, key: &str, fallback: &str) -> Value {
    match data.get(key) {
        Some(v) if chatbotx_core::flow_config::z::truthy(Some(v)) => v.clone(),
        _ => Value::from(fallback),
    }
}

/// `x || null`.
fn or_null(value: Option<&Value>) -> Value {
    match value {
        Some(v) if chatbotx_core::flow_config::z::truthy(Some(v)) => v.clone(),
        _ => Value::Null,
    }
}

/// `data.<key>` (`undefined` is dropped from the JSON body).
fn field(data: &Map<String, Value>, key: &str) -> Option<Value> {
    data.get(key).cloned()
}

fn product_ref(reference: &Value) -> Map<String, Value> {
    let product = reference.get("product").filter(|p| !p.is_null());
    let mut out = Map::new();
    out.insert(
        "product_id".into(),
        product
            .and_then(|p| p.get("productId"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    out.insert(
        "retailer_id".into(),
        reference
            .get("catalogReference")
            .and_then(|c| c.get("retailerId"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    out.insert(
        "name".into(),
        product
            .and_then(|p| p.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    out
}

fn commerce_product(product: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for (to, from) in [
        ("product_id", "productId"),
        ("variant_id", "variantId"),
        ("name", "name"),
        ("category_ids", "categoryIds"),
    ] {
        if let Some(v) = product.get(from) {
            out.insert(to.into(), v.clone());
        }
    }
    out
}

fn items(data: &Map<String, Value>, key: &str) -> Vec<Value> {
    data.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Inserts `(to, data[from])` for each present key.
fn copy(out: &mut Map<String, Value>, data: &Map<String, Value>, keys: &[(&str, &str)]) {
    for (to, from) in keys {
        if let Some(v) = field(data, from) {
            out.insert((*to).into(), v);
        }
    }
}

/// `buildWebhookPayload`.
pub async fn build_payload(deps: &Deps, event: &WebhookEvent) -> Result<Value, JobError> {
    let name = event_name(&event.event_type)
        .ok_or_else(|| fail(format!("No webhook payload for {}", event.event_type)))?;
    let mut out = Map::new();
    out.insert("event".into(), Value::from(name));
    out.insert("contact_id".into(), Value::from(event.contact_id.as_str()));
    out.insert("timestamp".into(), event.timestamp.clone());
    let data = &event.metadata;
    let workspace_id = event.workspace_id.parse::<i64>().unwrap_or(-1);
    match event.event_type.as_str() {
        "tagApplied" | "tagRemoved" => {
            let tag_id = data
                .get("tagId")
                .and_then(Value::as_str)
                .and_then(|t| t.parse::<i64>().ok());
            let tag = match tag_id {
                Some(id) => tag_name(deps, workspace_id, id).await?,
                None => None,
            };
            out.insert("tag".into(), Value::from(tag.unwrap_or_default()));
        }
        "customFieldValueChanged" => {
            let mut cf = Map::new();
            copy(
                &mut cf,
                data,
                &[
                    ("name", "customFieldName"),
                    ("old_value", "oldValue"),
                    ("new_value", "newValue"),
                ],
            );
            out.insert("custom_field".into(), Value::Object(cf));
        }
        "contactInfoUpdated" => {
            copy(&mut out, data, &[("info_type", "infoType")]);
            out.insert("old_value".into(), or_null(data.get("oldValue")));
            copy(&mut out, data, &[("new_value", "newValue")]);
        }
        "conversationTransferredToHuman" | "conversationTransferredToBot" => {
            copy(&mut out, data, &[("conversation_id", "conversationId")]);
            let fallback = if event.event_type == "conversationTransferredToHuman" {
                "bot"
            } else {
                "system"
            };
            out.insert(
                "transferred_by".into(),
                or_str(data, "transferredBy", fallback),
            );
        }
        "newContact" => new_contact(deps, workspace_id, event, &mut out).await?,
        "contactUnsubscribedFormBroadcast" => {}
        "archived" => {
            copy(&mut out, data, &[("conversation_id", "conversationId")]);
            out.insert("archived_by".into(), or_str(data, "archivedBy", "system"));
        }
        "followUp" => {
            copy(&mut out, data, &[("conversation_id", "conversationId")]);
            out.insert("marked_by".into(), or_str(data, "markedBy", "system"));
        }
        "conversationAssigned" => {
            copy(
                &mut out,
                data,
                &[
                    ("conversation_id", "conversationId"),
                    ("assigned_to", "assignedTo"),
                ],
            );
            out.insert("assigned_by".into(), or_str(data, "assignedBy", "system"));
        }
        "conversationUnassigned" => {
            copy(&mut out, data, &[("conversation_id", "conversationId")]);
            out.insert(
                "unassigned_by".into(),
                or_str(data, "unassignedBy", "system"),
            );
        }
        "subscribedToSequence" | "unsubscribedFromSequence" => {
            copy(
                &mut out,
                data,
                &[
                    ("sequence_id", "sequenceId"),
                    ("sequence_name", "sequenceName"),
                ],
            );
        }
        "contactReferredANewContact" | "contactReferredExistingContact" => {
            copy(
                &mut out,
                data,
                &[("ref_name", "refName"), ("reflink_id", "reflinkId")],
            );
        }
        "contactBlockedBot" | "contactUnblockedBot" => {
            let channel = data
                .get("channel")
                .filter(|c| c.is_string())
                .cloned()
                .unwrap_or(Value::Null);
            out.insert("channel".into(), channel);
        }
        // Server-generated metadata passes straight through.
        t if SPREAD_EVENT_TYPES.contains(&t) => {
            for (k, v) in data {
                out.insert(k.clone(), v.clone());
            }
        }
        "shoppingCartSent" | "userAskedAboutProduct" => {
            copy(
                &mut out,
                data,
                &[
                    ("channel", "channel"),
                    ("conversation_id", "conversationId"),
                    ("message_id", "messageId"),
                    ("catalog_id", "catalogId"),
                ],
            );
            if event.event_type == "shoppingCartSent" {
                copy(&mut out, data, &[("note", "note")]);
                let products = items(data, "products")
                    .iter()
                    .map(|item| {
                        let mut p = product_ref(item);
                        for (to, from) in [
                            ("quantity", "quantity"),
                            ("item_price", "itemPrice"),
                            ("currency", "currency"),
                        ] {
                            if let Some(v) = item.get(from) {
                                p.insert(to.into(), v.clone());
                            }
                        }
                        Value::Object(p)
                    })
                    .collect();
                out.insert("products".into(), Value::Array(products));
            } else {
                let product = data.get("product").map(|p| Value::Object(product_ref(p)));
                out.insert("product".into(), product.unwrap_or(Value::Null));
            }
        }
        "newOrder" | "orderAccepted" | "orderShipped" | "orderConcluded" | "orderCancelled"
        | "orderPaid" | "orderRefunded" | "productOrdered" => {
            copy(
                &mut out,
                data,
                &[
                    ("order_id", "orderId"),
                    ("order_number", "orderNumber"),
                    ("status", "status"),
                    ("source", "source"),
                    ("total", "total"),
                    ("currency", "currency"),
                    ("discount_code", "discountCode"),
                    ("channel", "channel"),
                    ("conversation_id", "conversationId"),
                ],
            );
            let products = items(data, "products")
                .iter()
                .map(|p| {
                    let mut o = commerce_product(p);
                    for (to, from) in [("quantity", "quantity"), ("line_total", "lineTotal")] {
                        if let Some(v) = p.get(from) {
                            o.insert(to.into(), v.clone());
                        }
                    }
                    Value::Object(o)
                })
                .collect();
            out.insert("products".into(), Value::Array(products));
        }
        "productAddedToCart" | "productRemovedFromCart" | "categoryAddedToCart" => {
            copy(&mut out, data, &[("channel", "channel")]);
            let product = data
                .get("product")
                .map_or(Value::Null, |p| Value::Object(commerce_product(p)));
            out.insert("product".into(), product);
            copy(
                &mut out,
                data,
                &[("quantity", "quantity"), ("change", "change")],
            );
        }
        "cartAbandoned" => {
            copy(
                &mut out,
                data,
                &[
                    ("channel", "channel"),
                    ("conversation_id", "conversationId"),
                    ("item_count", "itemCount"),
                    ("total", "total"),
                    ("currency", "currency"),
                ],
            );
            let list = items(data, "items")
                .iter()
                .map(|p| {
                    let mut o = commerce_product(p);
                    if let Some(q) = p.get("quantity") {
                        o.insert("quantity".into(), q.clone());
                    }
                    Value::Object(o)
                })
                .collect();
            out.insert("items".into(), Value::Array(list));
        }
        _ if registry::is_pipeline(&event.event_type) => pipeline_item(data, &mut out),
        _ => {}
    }
    Ok(Value::Object(out))
}

/// `buildPipelineItemPayload`: the `previous*` keys only on the event that
/// changed that attribute; never `contactInboxId`.
fn pipeline_item(data: &Map<String, Value>, out: &mut Map<String, Value>) {
    copy(
        out,
        data,
        &[
            ("pipeline_id", "pipelineId"),
            ("pipeline_name", "pipelineName"),
            ("pipeline_reference_name", "pipelineReferenceName"),
            ("item_id", "itemId"),
            ("item_number", "itemNumber"),
            ("title", "title"),
            ("stage_id", "stageId"),
            ("stage_name", "stageName"),
            ("status", "status"),
            ("status_label", "statusLabel"),
            ("priority", "priority"),
            ("value", "value"),
            ("currency", "currency"),
            ("due_date", "dueDate"),
            ("actor_kind", "actorKind"),
            ("actor_user_id", "actorUserId"),
            ("from_stage_id", "fromStageId"),
        ],
    );
    if data.contains_key("fromStageId") {
        out.insert(
            "from_stage_name".into(),
            data.get("fromStageName").cloned().unwrap_or(Value::Null),
        );
    }
    copy(
        out,
        data,
        &[
            ("from_pipeline_id", "fromPipelineId"),
            ("previous_status", "previousStatus"),
            ("previous_value", "previousValue"),
            ("previous_priority", "previousPriority"),
        ],
    );
}

/// `tagService.findNameByIdForWorkspace`.
async fn tag_name(deps: &Deps, workspace_id: i64, id: i64) -> Result<Option<String>, JobError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "name" FROM "Tag" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
                    params![id, workspace_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// `buildNewContactPayload`: the stored contact (committed before the
/// event) wins over the metadata.
async fn new_contact(
    deps: &Deps,
    workspace_id: i64,
    event: &WebhookEvent,
    out: &mut Map<String, Value>,
) -> Result<(), JobError> {
    let contact_id = event.contact_id.parse::<i64>().unwrap_or(-1);
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let (contact, fields): (Option<Row>, Vec<(String, String)>) = deps
        .db
        .read(move |conn| {
            let contact: Option<Row> = conn
                .query_row(
                    r#"SELECT "fullName", "firstName", "lastName", "phoneNumber", "email" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![contact_id, workspace_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let mut fields = Vec::new();
            if contact.is_some() {
                let mut stmt = conn.prepare(
                    r#"SELECT f."name", c."value" FROM "ContactCustomField" c JOIN "CustomField" f ON f."id" = c."customFieldId"
                       WHERE c."contactId" = ?1 ORDER BY c."id""#,
                )?;
                for row in stmt.query_map([contact_id], |r| Ok((r.get(0)?, r.get(1)?)))? {
                    fields.push(row?);
                }
            }
            Ok((contact, fields))
        })
        .await
        .map_err(db_fail)?;
    let pick = |stored: Option<&Option<String>>, key: Option<&str>| -> Value {
        match stored.and_then(Option::as_deref).filter(|s| !s.is_empty()) {
            Some(s) => Value::from(s),
            None => key.map_or(Value::Null, |k| or_null(event.metadata.get(k))),
        }
    };
    out.insert(
        "name".into(),
        pick(contact.as_ref().map(|c| &c.0), Some("name")),
    );
    out.insert(
        "first_name".into(),
        pick(contact.as_ref().map(|c| &c.1), None),
    );
    out.insert(
        "last_name".into(),
        pick(contact.as_ref().map(|c| &c.2), None),
    );
    out.insert(
        "phone".into(),
        pick(contact.as_ref().map(|c| &c.3), Some("phone")),
    );
    out.insert(
        "email".into(),
        pick(contact.as_ref().map(|c| &c.4), Some("email")),
    );
    let custom: Map<String, Value> = fields
        .into_iter()
        .map(|(k, v)| (k, Value::from(v)))
        .collect();
    out.insert("custom_fields".into(), Value::Object(custom));
    Ok(())
}

/// A [`WebhookHttp`] that records each POST and answers `status`.
#[derive(Default)]
pub struct RecordingWebhookHttp {
    pub posts: std::sync::Mutex<Vec<(String, Value)>>,
    /// URLs that fail with a non-retried error.
    pub failing: std::sync::Mutex<Vec<String>>,
}

impl WebhookHttp for RecordingWebhookHttp {
    fn post<'a>(&'a self, url: &'a str, body: &'a str) -> BoxFuture<'a, Result<u16, PostError>> {
        self.posts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((
                url.to_owned(),
                serde_json::from_str(body).unwrap_or(json!(null)),
            ));
        let failing = self
            .failing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|u| u == url);
        Box::pin(std::future::ready(if failing {
            Err(PostError::Other("boom".into()))
        } else {
            Ok(200)
        }))
    }
}

#[cfg(test)]
#[path = "webhook_tests.rs"]
mod tests;
