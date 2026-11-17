//! `record-provider-error-log.test.ts`, `write-error-log.test.ts` and
//! `ai-error-log-provider.test.ts`. The mocked `logProviderErrors` is a
//! recording emitter; the mocked `insertErrorLogs` is the DB. Stack
//! capture cases are n-a (no JS stacks in Rust: `errorStack` is never set).

use std::sync::{Arc, Mutex};

use chatbotx_jobs::BoxFuture;
use chatbotx_jobs::events::{EventBusError, EventBuses, EventEmitter};
use serde_json::{Value, json};

use super::*;
use crate::steps::test_support::Fixture;

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventEmitter for Recorder {
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Value,
    ) -> Result<BoxFuture<'a, Result<String, EventBusError>>, EventBusError> {
        self.0
            .lock()
            .unwrap()
            .push((event_type.to_owned(), payload));
        Ok(Box::pin(async { Ok("1-0".to_owned()) }))
    }
}

fn buses() -> (EventBuses, Arc<Recorder>) {
    let r = Arc::new(Recorder::default());
    (
        EventBuses {
            message: r.clone(),
            flow: r.clone(),
            dashboard: r.clone(),
            error_log: r.clone(),
        },
        r,
    )
}

fn failed(channel: &str, error: Value, extra: Value) -> Value {
    let mut p = json!({
        "context": { "workspaceId": "ws-1", "contactId": "c-1", "conversationId": "conv-1", "channel": channel },
        "action": { "messageId": "m-1" },
        "errorData": error,
        "occurredAt": "2026-07-11T00:00:00.000Z",
    });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

async fn logged(payloads: &[Value]) -> Vec<Value> {
    let (b, r) = buses();
    record_provider_error_log(Some(&b), payloads).await;
    r.0.lock().unwrap().iter().map(|(_, p)| p.clone()).collect()
}

#[tokio::test]
async fn record_provider_error_log_cases() {
    let terminal = json!({ "message": "(#10) outside allowed window", "code": 10, "statusCode": 400, "subcode": 2018278, "isRetryable": false });
    // a terminal channel failure, the channel as provider
    let rows = logged(&[failed("messenger", terminal.clone(), json!({}))]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0]["provider"].clone(),
            rows[0]["workspaceId"].clone(),
            rows[0]["contactId"].clone()
        ),
        (json!("messenger"), json!("ws-1"), json!("c-1"))
    );
    assert_eq!(rows[0]["error"]["httpCode"], json!("400"));
    // the context sourceId is forwarded; none = no sourceId
    let mut with_source = failed("messenger", terminal.clone(), json!({}));
    with_source["context"]["sourceId"] = json!("psid-1");
    assert_eq!(logged(&[with_source]).await[0]["sourceId"], json!("psid-1"));
    assert!(rows[0].get("sourceId").is_none());
    // no stack when the payload carries none
    assert!(rows[0]["error"].get("stackTrace").is_none());
    // retryable: skipped; willRetry is authoritative both ways; absent isRetryable = terminal
    assert!(
        logged(&[failed(
            "messenger",
            json!({ "message": "x", "isRetryable": true }),
            json!({})
        )])
        .await
        .is_empty()
    );
    assert_eq!(
        logged(&[failed(
            "messenger",
            json!({ "message": "x", "isRetryable": true }),
            json!({ "willRetry": false })
        )])
        .await
        .len(),
        1
    );
    assert!(
        logged(&[failed(
            "messenger",
            terminal.clone(),
            json!({ "willRetry": true })
        )])
        .await
        .is_empty()
    );
    assert_eq!(
        logged(&[failed("messenger", json!({ "message": "x" }), json!({}))])
            .await
            .len(),
        1
    );
    // unknown channel and omnichannel: no row
    assert!(
        logged(&[failed("carrier-pigeon", terminal.clone(), json!({}))])
            .await
            .is_empty()
    );
    assert!(
        logged(&[failed("omnichannel", terminal.clone(), json!({}))])
            .await
            .is_empty()
    );
    // broadcast and sequence-schedule sends: no row; other metadata: a row
    assert!(
        logged(&[failed(
            "whatsapp",
            terminal.clone(),
            json!({ "metadata": { "type": "broadcast" } })
        )])
        .await
        .is_empty()
    );
    assert!(
        logged(&[failed(
            "whatsapp",
            terminal.clone(),
            json!({ "metadata": { "type": "sequenceSchedule" } })
        )])
        .await
        .is_empty()
    );
    assert_eq!(
        logged(&[failed(
            "whatsapp",
            terminal.clone(),
            json!({ "metadata": { "type": "flow" } })
        )])
        .await
        .len(),
        1
    );
    // a batch logs each terminal payload once
    let batch = [
        failed("messenger", terminal.clone(), json!({})),
        failed(
            "whatsapp",
            terminal.clone(),
            json!({ "metadata": { "type": "broadcast" } }),
        ),
        failed("telegram", terminal.clone(), json!({})),
    ];
    let rows = logged(&batch).await;
    assert_eq!(
        rows.iter()
            .map(|r| r["provider"].clone())
            .collect::<Vec<_>>(),
        vec![json!("messenger"), json!("telegram")]
    );
    // an errorData that is not an object
    let rows = logged(&[failed("messenger", json!("boom"), json!({}))]).await;
    assert_eq!(
        (
            rows[0]["error"]["message"].clone(),
            rows[0]["error"]["httpCode"].clone()
        ),
        (json!("boom"), Value::Null)
    );
    // no buses: resolves without throwing
    record_provider_error_log(None, &batch).await;
}

#[test]
fn ai_error_log_provider_keeps_vendors_distinct() {
    for p in [
        "openai",
        "gemini",
        "claude",
        "deepseek",
        "openrouter",
        "muse",
    ] {
        assert_eq!(ai_error_log_provider(p), p);
    }
    assert_eq!(
        ai_error_log_provider("openaiCompatible"),
        "openai-compatible"
    );
}

fn entry(id: &str, contact: Option<&str>, workspace: &str) -> Value {
    let mut e = json!({
        "id": id, "workspaceId": workspace, "provider": "google-ads",
        "error": { "message": "Google Ads API: code 3 INVALID_ARGUMENT", "httpCode": "400", "stackTrace": "    at send (x.ts:1)" },
        "__eventBusMessageId": "1-0",
    });
    if let Some(c) = contact {
        e["contactId"] = json!(c);
        e["sourceId"] = json!("psid-1");
    }
    e
}

#[tokio::test]
async fn write_error_logs_cases() {
    let f = Fixture::new().await;
    // provider as action, message as detail, the producer's id, contact and sourceId kept
    write_error_logs(
        &f.deps.db,
        vec![entry("100", Some("3"), "2"), entry("101", None, "2")],
    )
    .await
    .unwrap();
    assert_eq!(f.text(r#"SELECT "action" || '|' || "detail" || '|' || "httpCode" || '|' || "contactId" || '|' || "sourceId" FROM "ErrorLog" WHERE "id" = 100"#).await.as_deref(),
        Some("google-ads|Google Ads API: code 3 INVALID_ARGUMENT|400|3|psid-1"));
    assert_eq!(
        f.text(r#"SELECT "stackTrace" FROM "ErrorLog" WHERE "id" = 100"#)
            .await
            .as_deref(),
        Some("    at send (x.ts:1)")
    );
    assert_eq!(
        f.int(
            r#"SELECT "contactId" IS NULL AND "sourceId" IS NULL FROM "ErrorLog" WHERE "id" = 101"#
        )
        .await,
        Some(1)
    );
    // a redelivery cannot duplicate
    write_error_logs(&f.deps.db, vec![entry("100", Some("3"), "2")])
        .await
        .unwrap();
    assert_eq!(f.int(r#"SELECT COUNT(*) FROM "ErrorLog""#).await, Some(2));
    // a deleted contact keeps the row without attribution; a gone workspace drops the row
    let mut null_code = entry("103", Some("999"), "2");
    null_code["error"]["httpCode"] = Value::Null;
    write_error_logs(&f.deps.db, vec![null_code, entry("104", None, "999")])
        .await
        .unwrap();
    assert_eq!(f.int(r#"SELECT "contactId" IS NULL AND "httpCode" IS NULL AND "sourceId" = 'psid-1' FROM "ErrorLog" WHERE "id" = 103"#).await, Some(1));
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ErrorLog" WHERE "id" = 104"#)
            .await,
        Some(0)
    );
    // an empty batch does nothing
    write_error_logs(&f.deps.db, vec![]).await.unwrap();
}

#[tokio::test]
async fn message_failed_reaches_the_error_log_and_the_contact_inbox() {
    let f = Fixture::new().await;
    let (buses, worker) = start(f.deps.db.clone()).await.unwrap();
    emit_message_failed(
        Some(&buses),
        &MessageContext {
            workspace_id: 2,
            contact_id: 3,
            conversation_id: 6,
            channel: "webchat".into(),
            contact_inbox_id: 5,
            inbox_id: 4,
            source_id: "s".into(),
        },
        message_action(Some(77)),
        parsed_error(&SendFailure::Other("socket closed".into())),
        None,
        false,
    )
    .await;
    let mut rows = None;
    for _ in 0..100 {
        rows = f.text(r#"SELECT "action" || '|' || "detail" || '|' || "httpCode" || '|' || "sourceId" FROM "ErrorLog""#).await;
        if rows.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(rows.as_deref(), Some("webchat|socket closed|400|s"));
    assert_eq!(f.text(r#"SELECT "lastErrorLog" || '|' || "consecutiveFailedReply" FROM "ContactInbox" WHERE "id" = 5"#).await.as_deref(), Some("socket closed|1"));
    worker.stop().await;
}

/// `broadcast-ops`: `message:sent` marks the recipient delivered (not on
/// WhatsApp); `message:failed` keeps the first failure.
#[tokio::test]
async fn broadcast_ops_record_the_recipient_outcome() {
    let f = Fixture::new().await;
    f.exec(
        r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
             VALUES (20, 'B', 2, 'sending', 'now', 0, 'allContacts', 'omnichannel');
           INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent")
             VALUES (20, 3, 5, 6, 1);"#,
    )
    .await;
    let meta = json!({ "type": "broadcast", "broadcastId": "20", "contactInboxId": "5" });
    let event = |channel: &str, at: &str| {
        json!({ "context": { "channel": channel }, "metadata": meta, "occurredAt": at,
                "errorData": { "message": "boom" } })
    };
    record_broadcast_delivered(&f.deps.db, &[event("whatsapp", "2026-01-01T00:00:00.000Z")], true).await;
    assert_eq!(f.text(r#"SELECT CAST("deliveredAt" AS TEXT) FROM "ContactOnBroadcast""#).await, None);
    record_broadcast_delivered(&f.deps.db, &[event("webchat", "2026-01-01T00:00:00.000Z")], true).await;
    assert_eq!(
        f.text(r#"SELECT CAST("deliveredAt" AS TEXT) FROM "ContactOnBroadcast""#).await.as_deref(),
        Some("1767225600000")
    );
    record_broadcast_failed(&f.deps.db, &[event("webchat", "2026-01-01T00:00:01.000Z")]).await;
    record_broadcast_failed(&f.deps.db, &[event("webchat", "2026-01-01T00:00:02.000Z")]).await;
    assert_eq!(
        f.text(r#"SELECT "failedAt" || '|' || "errorContent" FROM "ContactOnBroadcast""#).await.as_deref(),
        Some(r#"1767225601000|{"message":"boom"}"#)
    );
}
