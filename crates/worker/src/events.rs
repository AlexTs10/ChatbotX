//! The `events` worker (`events/worker.ts`) on the SQLite event bus
//! (`chatbotx_jobs::events`): the error-log writer (`error-log:recorded`),
//! the `message:failed` listeners that have a Rust domain
//! (`contact-inbox-send-failure`, `error-log`), and `logProviderErrors`
//! (`business/error-log/service.ts`), which emits the error-log events.
//!
//! The `broadcast-ops` listeners (`message:sent` / `delivered` / `failed`)
//! record the broadcast recipients' outcome.
//!
//! ponytail: the other analytics listeners (sequence / flow /
//! comment-automation ops, broadcast `seen`, MAC tracking, active hourly, contact blocked
//! detection, ads contact-replied, the dashboard bus) wait for the analytics
//! package; their buses still run so the entries are acked.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_core::logger::scrub_secrets_in_string;
use chatbotx_core::utils::error_log::{MAX_STACK_LENGTH, is_error_log_provider};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::events::config::bus_config;
use chatbotx_jobs::events::{
    BusKind, EventBus, EventBuses, EventListener, EventModule, EventWorker, ListenerFn,
    ListenerOutcome, SqliteEventStore, emit, event_bus_message_id_key,
};
use serde_json::{Map, Value, json};

use crate::channel::{Category, SendFailure};

/// `MAX_DETAIL_LENGTH`.
const MAX_DETAIL_LENGTH: usize = 8192;
/// `UNKNOWN_ERROR_DETAIL`.
const UNKNOWN_ERROR_DETAIL: &str = "An unknown error occurred";

/// `LogProviderErrorInput`.
#[derive(Debug, Clone, Default)]
pub struct ProviderError {
    pub provider: String,
    pub workspace_id: String,
    pub contact_id: Option<String>,
    pub source_id: Option<String>,
    /// The thrown value as JSON (`{ message, statusCode, ... }` or a string).
    pub error: Value,
    /// `Some` overrides the status derived from `error` (`Some(None)` = none).
    pub http_code: Option<Option<String>>,
    pub stack_trace: Option<String>,
}

fn utf16_prefix(text: &str, max: usize) -> String {
    chatbotx_core::js::slice_utf16(text, 0, max).to_owned()
}

/// `resolveMessage` (shallow: a top-level `message` only).
fn resolve_message(error: &Value) -> String {
    match error {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .unwrap_or(UNKNOWN_ERROR_DETAIL)
            .to_owned(),
        Value::Null => UNKNOWN_ERROR_DETAIL.to_owned(),
        other => chatbotx_core::js::string_of(other),
    }
}

/// `numericStatus`: `statusCode` / `httpStatusCode` above 0.
fn numeric_status(error: &Value) -> Option<i64> {
    ["statusCode", "httpStatusCode"]
        .iter()
        .filter_map(|k| error.get(k).and_then(Value::as_f64))
        .find(|n| *n > 0.0)
        .map(|n| n as i64)
}

/// `toEntry`: the `error-log:recorded` payload (id minted here so a
/// redelivery re-inserts the same key; secrets scrubbed).
pub fn error_log_entry(input: &ProviderError) -> Value {
    let http_code = match &input.http_code {
        Some(code) => code.clone(),
        None => numeric_status(&input.error).map(|s| s.to_string()),
    };
    let mut error = Map::new();
    error.insert(
        "message".into(),
        Value::from(utf16_prefix(
            &scrub_secrets_in_string(&resolve_message(&input.error)),
            MAX_DETAIL_LENGTH,
        )),
    );
    error.insert(
        "httpCode".into(),
        http_code.map_or(Value::Null, Value::from),
    );
    if let Some(stack) = &input.stack_trace {
        error.insert(
            "stackTrace".into(),
            Value::from(scrub_secrets_in_string(&utf16_prefix(
                stack,
                MAX_STACK_LENGTH,
            ))),
        );
    }
    let mut entry = Map::new();
    entry.insert(
        "id".into(),
        Value::from(chatbotx_db::repo::new_id().to_string()),
    );
    entry.insert(
        "workspaceId".into(),
        Value::from(input.workspace_id.as_str()),
    );
    entry.insert("provider".into(), Value::from(input.provider.as_str()));
    if let Some(c) = &input.contact_id {
        entry.insert("contactId".into(), Value::from(c.as_str()));
    }
    if let Some(s) = &input.source_id {
        entry.insert("sourceId".into(), Value::from(s.as_str()));
    }
    entry.insert("error".into(), Value::Object(error));
    Value::Object(entry)
}

/// `logProviderErrors`: emits one `error-log:recorded` per input; never
/// fails. Returns the indexes whose emit failed. No buses = nothing to do
/// (`isNoRedisEnv`).
pub async fn log_provider_errors(
    buses: Option<&EventBuses>,
    inputs: &[ProviderError],
) -> Vec<usize> {
    let Some(buses) = buses else {
        return Vec::new();
    };
    let mut failed = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        match emit(buses, "error-log:recorded", error_log_entry(input)) {
            None => {
                tracing::warn!("logProviderErrors: emit routing failed");
                failed.push(index);
            }
            Some(future) => match future.await {
                Ok(id) if id.is_empty() => {
                    tracing::warn!(workspace_id = %input.workspace_id, "logProviderErrors: error log rejected by schema, dropping");
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(err = %err, "logProviderErrors: failed to emit error log");
                    failed.push(index);
                }
            },
        }
    }
    failed
}

/// `logProviderErrorForChannel`: an unknown channel writes no row.
pub async fn log_provider_error_for_channel(
    buses: Option<&EventBuses>,
    channel: &str,
    mut input: ProviderError,
) {
    if !is_error_log_provider(channel) {
        return;
    }
    input.provider = channel.to_owned();
    log_provider_errors(buses, &[input]).await;
}

/// `aiErrorLogProvider`: each AI vendor keeps its own provider;
/// `openaiCompatible` is `openai-compatible`.
pub fn ai_error_log_provider(provider: &str) -> &str {
    match provider {
        "openaiCompatible" => "openai-compatible",
        other => other,
    }
}

fn category_str(category: Category) -> &'static str {
    match category {
        Category::RateLimited => "rate_limited",
        Category::AuthFailed => "auth_failed",
        Category::UserBlocked => "user_blocked",
        Category::InvalidRecipient => "invalid_recipient",
        Category::PermissionDenied => "permission_denied",
        Category::PayloadInvalid => "payload_invalid",
        Category::QuotaExceeded => "quota_exceeded",
        Category::NetworkError => "network_error",
        Category::Unknown => "unknown",
    }
}

/// `parseSdkError` of a send failure (`SdkException.getErrorData`); a
/// non-channel failure is an `SdkException` with the defaults.
pub fn parsed_error(error: &SendFailure) -> Value {
    match error {
        SendFailure::Channel(e) => json!({
            "message": if e.message.is_empty() { "Unknown error." } else { e.message.as_str() },
            "code": e.code.clone().map_or(json!(-1), Value::from),
            "statusCode": e.http_status_code,
            "subcode": -1,
            "category": category_str(e.category),
            "isRetryable": e.is_retryable(),
            "isPermanent": e.is_permanent(),
        }),
        SendFailure::Other(m) => json!({
            "message": if m.is_empty() { "Unknown error." } else { m.as_str() },
            "code": -1,
            "statusCode": 400,
            "subcode": -1,
        }),
    }
}

/// The `context` of a message event.
pub struct MessageContext {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub conversation_id: i64,
    pub channel: String,
    pub contact_inbox_id: i64,
    pub inbox_id: i64,
    pub source_id: String,
}

impl MessageContext {
    /// `eventLogData.context` of a send to `contact_inbox`.
    pub fn of(
        conversation: &chatbotx_db::repo::models::Conversation,
        contact_inbox: &chatbotx_db::repo::models::ContactInbox,
    ) -> Self {
        Self {
            workspace_id: conversation.workspace_id,
            contact_id: conversation.contact_id,
            conversation_id: conversation.id,
            channel: contact_inbox.channel.clone(),
            contact_inbox_id: contact_inbox.id,
            inbox_id: contact_inbox.inbox_id,
            source_id: contact_inbox.source_id.clone(),
        }
    }
}

fn context_json(ctx: &MessageContext) -> Value {
    json!({
        "workspaceId": ctx.workspace_id.to_string(),
        "contactId": ctx.contact_id.to_string(),
        "conversationId": ctx.conversation_id.to_string(),
        "channel": ctx.channel,
        "contactInboxId": ctx.contact_inbox_id.to_string(),
        "inboxId": ctx.inbox_id.to_string(),
        "sourceId": ctx.source_id,
    })
}

/// Emits one message event (`eventLogData` + `occurredAt`); a bus failure is
/// logged, never raised. `extra` adds the event's own fields.
pub async fn emit_message_event(
    buses: Option<&EventBuses>,
    event_type: &str,
    ctx: &MessageContext,
    action: Value,
    metadata: Option<&Value>,
    extra: Value,
) {
    let Some(buses) = buses else { return };
    let mut payload = json!({
        "context": context_json(ctx),
        "action": action,
        "occurredAt": chatbotx_db::repo::iso(chatbotx_db::repo::now_ms()),
    });
    if let Some(metadata) = metadata.filter(|m| m.is_object()) {
        payload["metadata"] = metadata.clone();
    }
    if let (Value::Object(target), Value::Object(extra)) = (&mut payload, extra) {
        target.extend(extra);
    }
    if let Some(future) = emit(buses, event_type, payload)
        && let Err(err) = future.await
    {
        tracing::warn!(err = %err, event_type, "message event emit failed");
    }
}

/// Emits `message:sent` (`sendFlowStep`, the template sends) with the
/// action `{ flowId?, flowVersionId?, messageId, sourceId? }`.
pub async fn emit_message_sent(
    buses: Option<&EventBuses>,
    ctx: &MessageContext,
    action: Value,
    metadata: Option<&Value>,
) {
    emit_message_event(buses, "message:sent", ctx, action, metadata, json!({})).await;
}

/// Emits `message:failed` (`sendMessageToChannel`'s catch block and the
/// terminal `sendFlowStep` catch).
pub async fn emit_message_failed(
    buses: Option<&EventBuses>,
    ctx: &MessageContext,
    action: Value,
    error_data: Value,
    metadata: Option<&Value>,
    will_retry: bool,
) {
    emit_message_event(
        buses,
        "message:failed",
        ctx,
        action,
        metadata,
        json!({ "errorData": error_data, "willRetry": will_retry }),
    )
    .await;
}

/// `{ messageId }` of an event action (`""` when the row is unknown).
pub fn message_action(message_id: Option<i64>) -> Value {
    json!({ "messageId": message_id.map(|id| id.to_string()).unwrap_or_default() })
}

/// The broadcast recipient of a broadcast send's `metadata`
/// (`BROADCAST_PAYLOAD_TYPE`): `(broadcastId, contactInboxId)`.
fn broadcast_target(payload: &Value) -> Option<(i64, i64)> {
    let m = payload.get("metadata")?;
    if m.get("type").and_then(Value::as_str) != Some("broadcast") {
        return None;
    }
    let id = |k: &str| m.get(k).and_then(Value::as_str)?.parse::<i64>().ok();
    Some((id("broadcastId")?, id("contactInboxId")?))
}

fn occurred_at_ms(payload: &Value) -> i64 {
    payload
        .get("occurredAt")
        .and_then(Value::as_str)
        .and_then(chatbotx_db::repo::parse_iso)
        .unwrap_or_else(chatbotx_db::repo::now_ms)
}

/// `broadcastAnalyticsService.onMessageSent` / `onDelivered`
/// (`processBroadcastEvents(…, "deliveredAt")`): sets `deliveredAt` on the
/// unread recipient rows. `message:sent` skips WhatsApp (its receipts report
/// the delivery).
pub async fn record_broadcast_delivered(db: &Db, payloads: &[Value], skip_whatsapp: bool) {
    let items: Vec<(i64, i64, i64)> = payloads
        .iter()
        .filter(|p| {
            !(skip_whatsapp
                && p.pointer("/context/channel").and_then(Value::as_str) == Some("whatsapp"))
        })
        .filter_map(|p| broadcast_target(p).map(|(b, ci)| (b, ci, occurred_at_ms(p))))
        .collect();
    if items.is_empty() {
        return;
    }
    let result = db
        .write(move |tx| {
            for (broadcast, contact_inbox, at) in &items {
                tx.execute(
                    r#"UPDATE "ContactOnBroadcast" SET "deliveredAt" = ?3
                       WHERE "broadcastId" = ?1 AND "contactInboxId" = ?2 AND "isRead" = 0"#,
                    params![broadcast, contact_inbox, at],
                )?;
            }
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::error!(err = %err, "broadcast-ops: deliveredAt update failed");
    }
}

/// `broadcastAnalyticsService.onFailed` (`updateFailedBulk`): the first
/// failure time and error of each recipient row.
pub async fn record_broadcast_failed(db: &Db, payloads: &[Value]) {
    let items: Vec<(i64, i64, i64, String)> = payloads
        .iter()
        .filter_map(|p| {
            let (b, ci) = broadcast_target(p)?;
            let error = serde_json::to_string(p.get("errorData").unwrap_or(&Value::Null)).ok()?;
            Some((b, ci, occurred_at_ms(p), error))
        })
        .collect();
    if items.is_empty() {
        return;
    }
    let result = db
        .write(move |tx| {
            for (broadcast, contact_inbox, at, error) in &items {
                tx.execute(
                    r#"UPDATE "ContactOnBroadcast"
                       SET "failedAt" = COALESCE("failedAt", ?3), "errorContent" = COALESCE("errorContent", ?4)
                       WHERE "broadcastId" = ?1 AND "contactInboxId" = ?2"#,
                    params![broadcast, contact_inbox, at, error],
                )?;
            }
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::error!(err = %err, "broadcast-ops: failedAt update failed");
    }
}

/// `isBulkSendOrigin`.
fn is_bulk_send_origin(metadata: Option<&Value>) -> bool {
    matches!(
        metadata.and_then(|m| m.get("type")).and_then(Value::as_str),
        Some("broadcast" | "sequenceSchedule")
    )
}

/// `recordProviderErrorLog`: one row per terminal, non-bulk failure.
pub async fn record_provider_error_log(buses: Option<&EventBuses>, payloads: &[Value]) {
    let mut eligible = Vec::new();
    for payload in payloads {
        if is_bulk_send_origin(payload.get("metadata")) {
            continue;
        }
        let error = payload.get("errorData").cloned().unwrap_or(Value::Null);
        let is_retryable = error.get("isRetryable") == Some(&Value::Bool(true));
        let will_retry = payload
            .get("willRetry")
            .and_then(Value::as_bool)
            .unwrap_or(is_retryable);
        if will_retry {
            continue;
        }
        let ctx = &payload["context"];
        let channel = ctx.get("channel").and_then(Value::as_str).unwrap_or("");
        if !is_error_log_provider(channel) {
            tracing::warn!(
                channel,
                "recordProviderErrorLog: unmapped channel, skipping error log"
            );
            continue;
        }
        let status = error.get("statusCode").and_then(Value::as_f64);
        eligible.push(ProviderError {
            provider: channel.to_owned(),
            workspace_id: ctx
                .get("workspaceId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            contact_id: ctx
                .get("contactId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_id: ctx
                .get("sourceId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            http_code: Some(status.map(chatbotx_core::js::number_to_string)),
            stack_trace: payload
                .get("errorStack")
                .and_then(Value::as_str)
                .map(str::to_owned),
            error,
        });
    }
    if eligible.is_empty() {
        return;
    }
    let failed = log_provider_errors(buses, &eligible).await;
    if !failed.is_empty() {
        tracing::error!(
            count = failed.len(),
            total = eligible.len(),
            "recordProviderErrorLog: error logs dropped, emit failed and this bus cannot retry"
        );
    }
}

/// `formatSendFailureError`.
fn format_send_failure_error(error: &Value) -> String {
    match error {
        Value::String(s) => s.clone(),
        Value::Object(map) => match map.get("message").and_then(Value::as_str) {
            Some(m) if !m.is_empty() => m.to_owned(),
            _ => error.to_string(),
        },
        Value::Null => "Failed to send message".to_owned(),
        other => other.to_string(),
    }
}

/// `recordContactInboxSendFailure`: `consecutiveFailedReply + 1` and the
/// last error on the contact's inbox (workspace-scoped).
pub async fn record_contact_inbox_send_failure(db: &Db, payloads: &[Value]) -> Vec<String> {
    let mut failed = Vec::new();
    for payload in payloads {
        let ctx = &payload["context"];
        let id = |k: &str| {
            ctx.get(k)
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok())
        };
        let (Some(contact_inbox), Some(contact), Some(workspace)) =
            (id("contactInboxId"), id("contactId"), id("workspaceId"))
        else {
            continue;
        };
        let error = format_send_failure_error(payload.get("errorData").unwrap_or(&Value::Null));
        let result = db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE "ContactInbox" SET "lastErrorLog" = ?1, "consecutiveFailedReply" = "consecutiveFailedReply" + 1, "updatedAt" = ?5
                       WHERE "id" = ?2 AND "contactId" = ?3 AND "contactId" IN (SELECT "id" FROM "Contact" WHERE "workspaceId" = ?4)"#,
                    params![error, contact_inbox, contact, workspace, chatbotx_db::repo::now_ms()],
                )?;
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(err = %err, contact_inbox_id = contact_inbox, "recordSendFailure failed");
            if let Some(id) = payload
                .get(event_bus_message_id_key())
                .and_then(Value::as_str)
            {
                failed.push(id.to_owned());
            }
        }
    }
    failed
}

/// `writeErrorLogs`: one row per event (`ON CONFLICT DO NOTHING` absorbs a
/// redelivery). A deleted contact drops the attribution; a deleted
/// workspace drops the row.
pub async fn write_error_logs(db: &Db, payloads: Vec<Value>) -> Result<(), String> {
    if payloads.is_empty() {
        return Ok(());
    }
    db.write(move |tx| {
        let now = chatbotx_db::repo::now_ms();
        for p in &payloads {
            let s = |k: &str| p.get(k).and_then(Value::as_str);
            let Some(id) = s("id").and_then(|v| v.parse::<i64>().ok()) else {
                continue;
            };
            let Some(workspace) = s("workspaceId").and_then(|v| v.parse::<i64>().ok()) else {
                continue;
            };
            let exists: Option<i64> = tx
                .query_row(r#"SELECT 1 FROM "Workspace" WHERE "id" = ?1"#, [workspace], |r| r.get(0))
                .optional()?;
            if exists.is_none() {
                tracing::warn!(workspace_id = workspace, "writeErrorLogs: row permanently unwritable, dropping");
                continue;
            }
            let contact = match s("contactId").and_then(|v| v.parse::<i64>().ok()) {
                Some(c) => tx
                    .query_row(r#"SELECT "id" FROM "Contact" WHERE "id" = ?1"#, [c], |r| r.get::<_, i64>(0))
                    .optional()?,
                None => None,
            };
            let error = &p["error"];
            tx.execute(
                r#"INSERT INTO "ErrorLog" ("id", "createdAt", "updatedAt", "action", "detail", "httpCode", "workspaceId", "contactId", "sourceId", "stackTrace")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT ("id") DO NOTHING"#,
                params![
                    id,
                    now,
                    s("provider").unwrap_or(""),
                    error.get("message").and_then(Value::as_str).unwrap_or(""),
                    error.get("httpCode").and_then(Value::as_str),
                    workspace,
                    contact,
                    s("sourceId"),
                    error.get("stackTrace").and_then(Value::as_str),
                ],
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
}

fn listener(name: &str, handler: ListenerFn) -> EventListener {
    EventListener {
        name: name.to_owned(),
        handler,
    }
}

/// The four buses on the job DB, their listeners, and the started worker.
pub async fn start(db: Arc<Db>) -> Result<(EventBuses, EventWorker), String> {
    let store = Arc::new(
        SqliteEventStore::open(Arc::clone(&db))
            .await
            .map_err(|e| e.to_string())?,
    );
    let clock: Arc<dyn chatbotx_jobs::clock::Clock> = Arc::new(chatbotx_jobs::SystemClock);
    let bus = |kind| {
        Arc::new(EventBus::new(
            store.clone(),
            bus_config(kind),
            Arc::clone(&clock),
        ))
    };
    let (message, flow, dashboard, error_log) = (
        bus(BusKind::Message),
        bus(BusKind::Flow),
        bus(BusKind::Dashboard),
        bus(BusKind::ErrorLog),
    );
    let buses = EventBuses {
        message: message.clone(),
        flow: flow.clone(),
        dashboard: dashboard.clone(),
        error_log: error_log.clone(),
    };

    let mut error_log_listeners = BTreeMap::new();
    let writer_db = Arc::clone(&db);
    error_log_listeners.insert(
        "error-log:recorded".to_owned(),
        vec![listener(
            "error-log-writer",
            Arc::new(move |payloads, _| {
                let db = Arc::clone(&writer_db);
                Box::pin(async move {
                    let ids: Vec<String> = payloads
                        .iter()
                        .filter_map(|p| {
                            p.get(event_bus_message_id_key())
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect();
                    match write_error_logs(&db, payloads).await {
                        Ok(()) => Ok(ListenerOutcome::default()),
                        Err(err) => {
                            tracing::error!(err = %err, "writeErrorLogs: batch insert failed");
                            Ok(ListenerOutcome {
                                failed_message_ids: ids,
                            })
                        }
                    }
                })
            }),
        )],
    );

    let mut message_listeners = BTreeMap::new();
    let inbox_db = Arc::clone(&db);
    let log_buses = buses.clone();
    message_listeners.insert(
        "message:failed".to_owned(),
        vec![
            listener(
                "contact-inbox-send-failure",
                Arc::new(move |payloads, _| {
                    let db = Arc::clone(&inbox_db);
                    Box::pin(async move {
                        let failed_message_ids =
                            record_contact_inbox_send_failure(&db, &payloads).await;
                        Ok(ListenerOutcome { failed_message_ids })
                    })
                }),
            ),
            listener(
                "error-log",
                Arc::new(move |payloads, _| {
                    let buses = log_buses.clone();
                    Box::pin(async move {
                        record_provider_error_log(Some(&buses), &payloads).await;
                        Ok(ListenerOutcome::default())
                    })
                }),
            ),
        ],
    );

    let sent_db = Arc::clone(&db);
    message_listeners.insert(
        "message:sent".to_owned(),
        vec![listener(
            "broadcast-ops",
            Arc::new(move |payloads, _| {
                let db = Arc::clone(&sent_db);
                Box::pin(async move {
                    record_broadcast_delivered(&db, &payloads, true).await;
                    Ok(ListenerOutcome::default())
                })
            }),
        )],
    );
    let delivered_db = Arc::clone(&db);
    message_listeners.insert(
        "message:delivered".to_owned(),
        vec![listener(
            "broadcast-ops",
            Arc::new(move |payloads, _| {
                let db = Arc::clone(&delivered_db);
                Box::pin(async move {
                    record_broadcast_delivered(&db, &payloads, false).await;
                    Ok(ListenerOutcome::default())
                })
            }),
        )],
    );
    if let Some(failed) = message_listeners.get_mut("message:failed") {
        let failed_db = Arc::clone(&db);
        failed.insert(
            0,
            listener(
                "broadcast-ops",
                Arc::new(move |payloads, _| {
                    let db = Arc::clone(&failed_db);
                    Box::pin(async move {
                        record_broadcast_failed(&db, &payloads).await;
                        Ok(ListenerOutcome::default())
                    })
                }),
            ),
        );
    }

    let worker = EventWorker::new();
    worker
        .start(
            vec![
                EventModule {
                    bus: message,
                    listeners: message_listeners,
                },
                EventModule {
                    bus: flow,
                    listeners: BTreeMap::new(),
                },
                EventModule {
                    bus: dashboard,
                    listeners: BTreeMap::new(),
                },
                EventModule {
                    bus: error_log,
                    listeners: error_log_listeners,
                },
            ],
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok((buses, worker))
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
