//! Port of `integrations/whatsapp/__tests__/automatic-events-webhook.test.ts`:
//! `extractAutomaticEventPayloads` is the `adsAutomaticEvent` part of
//! `webhook_jobs` (the signature check has its own tests), and the webhook
//! enqueue logs and skips a failed automatic-event job.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use chatbotx_worker::channels::whatsapp_webhook::{enqueue_webhook_jobs, webhook_jobs};
use serde_json::{Value, json};

fn automatic_event(overrides: Value) -> Value {
    let mut event = json!({
        "event_name": "LeadSubmitted",
        "id": "wamid.lead-1",
        "timestamp": "1800000000",
        "ctwa_clid": "clid-1",
        "custom_data": { "currency": "USD", "value": "19.99" },
    });
    for (k, v) in overrides.as_object().cloned().unwrap_or_default() {
        if v.is_null() {
            event.as_object_mut().unwrap().remove(&k);
        } else {
            event[k] = v;
        }
    }
    event
}

fn automatic_value(events: Vec<Value>, phone: &str) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "metadata": { "phone_number_id": phone },
        "automatic_events": events,
    })
}

fn body(entries: Value) -> Value {
    json!({ "object": "whatsapp_business_account", "entry": entries })
}

/// `{ phoneNumberId, wabaId, payload }` of each `adsAutomaticEvent` job.
fn extracted(body: &Value) -> Vec<Value> {
    webhook_jobs(body, None)
        .into_iter()
        .filter(|j| j.name == "adsAutomaticEvent")
        .map(|j| {
            let d = &j.data["data"];
            json!({ "phoneNumberId": d["phoneNumberId"], "wabaId": d["wabaId"], "payload": d["payload"] })
        })
        .collect()
}

/// Port of: "finds automatic events in a realistic multi-entry payload"
#[test]
fn finds_automatic_events_in_a_multi_entry_payload() {
    let result = extracted(&body(json!([
        {
            "id": "waba-1",
            "changes": [
                { "field": "messages", "value": { "metadata": { "phone_number_id": "phone-ignored" } } },
                { "field": "automatic_events", "value": automatic_value(vec![
                    automatic_event(json!({})),
                    automatic_event(json!({ "event_name": "Purchase", "id": "wamid.purchase-1", "timestamp": 1_800_000_001, "ctwa_clid": "clid-2" })),
                ], "phone-1") },
            ],
        },
        {
            "id": "waba-2",
            "changes": [
                { "field": "automatic_events", "value": automatic_value(vec![
                    automatic_event(json!({ "id": "wamid.lead-2", "timestamp": "1800000002", "ctwa_clid": "clid-3" })),
                ], "phone-2") },
            ],
        },
    ])));
    let custom = json!({ "currency": "USD", "value": "19.99" });
    assert_eq!(
        result,
        vec![
            json!({ "phoneNumberId": "phone-1", "wabaId": "waba-1", "payload": {
                "event_name": "LeadSubmitted", "id": "wamid.lead-1", "timestamp": "1800000000",
                "ctwa_clid": "clid-1", "custom_data": custom } }),
            json!({ "phoneNumberId": "phone-1", "wabaId": "waba-1", "payload": {
                "event_name": "Purchase", "id": "wamid.purchase-1", "timestamp": 1_800_000_001,
                "ctwa_clid": "clid-2", "custom_data": custom } }),
            json!({ "phoneNumberId": "phone-2", "wabaId": "waba-2", "payload": {
                "event_name": "LeadSubmitted", "id": "wamid.lead-2", "timestamp": "1800000002",
                "ctwa_clid": "clid-3", "custom_data": custom } }),
        ]
    );
}

/// Port of: "skips malformed values without throwing"
#[test]
fn skips_malformed_values() {
    let result = extracted(&body(json!([{
        "id": "waba-1",
        "changes": [
            { "field": "automatic_events", "value": automatic_value(vec![automatic_event(json!({ "ctwa_clid": null }))], "phone-1") },
            { "field": "automatic_events", "value": "garbage" },
        ],
    }])));
    assert!(result.is_empty());
}

/// Port of: "skips unknown event_name values while preserving valid siblings"
#[test]
fn skips_unknown_event_names_and_keeps_valid_siblings() {
    let result = extracted(&body(json!([{
        "id": "waba-1",
        "changes": [{ "field": "automatic_events", "value": automatic_value(vec![
            automatic_event(json!({ "event_name": "OtherEvent" })),
            automatic_event(json!({ "id": "wamid.lead-2" })),
        ], "phone-1") }],
    }])));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["payload"]["id"], "wamid.lead-2");
}

#[derive(Default)]
struct Queue {
    added: Mutex<Vec<(String, Value, Option<String>)>>,
    fail_on: Option<&'static str>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        _: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_default();
        let fail = self.fail_on.is_some_and(|id| job_id.ends_with(id));
        self.added
            .lock()
            .unwrap()
            .push((name.to_owned(), data, opts.job_id));
        Box::pin(async move {
            if fail {
                return Err(JobsError::Other("redis down".into()));
            }
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }
}

fn db() -> Arc<Db> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-automatic-events-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    Arc::new(Db::open(dir.join("test.db")).expect("open"))
}

fn delivery(ids: &[(&str, &str)]) -> Vec<u8> {
    let events = ids
        .iter()
        .map(|(name, id)| automatic_event(json!({ "event_name": name, "id": id })))
        .collect();
    body(json!([{ "id": "waba-1", "changes": [
        { "field": "automatic_events", "value": automatic_value(events, "phone-1") },
    ] }]))
    .to_string()
    .into_bytes()
}

/// Port of: "enqueues one BullMQ job for each automatic event item"
#[tokio::test]
async fn enqueues_one_job_per_automatic_event() {
    let queue = Queue::default();
    let raw = delivery(&[
        ("LeadSubmitted", "wamid.lead-1"),
        ("Purchase", "wamid.purchase-1"),
    ]);
    enqueue_webhook_jobs(
        &db(),
        &queue,
        &raw,
        None,
        chatbotx_worker::channels::whatsapp_webhook::AccountScope::Unscoped,
    )
    .await
    .expect("acknowledged");
    let added = queue.added.lock().unwrap();
    assert_eq!(added.len(), 2);
    let (name, data, job_id) = &added[0];
    assert_eq!(name, "adsAutomaticEvent");
    assert_eq!(data["type"], "adsAutomaticEvent");
    assert_eq!(data["data"]["integrationIdentifier"], "phone-1");
    assert_eq!(data["data"]["phoneNumberId"], "phone-1");
    assert_eq!(data["data"]["wabaId"], "waba-1");
    assert_eq!(data["data"]["payload"]["id"], "wamid.lead-1");
    assert_eq!(job_id.as_deref(), Some("ads-auto-phone-1-wamid.lead-1"));
    assert_eq!(added[1].1["data"]["payload"]["id"], "wamid.purchase-1");
    assert_eq!(
        added[1].2.as_deref(),
        Some("ads-auto-phone-1-wamid.purchase-1")
    );
}

/// Port of: "enqueues the 1st and 3rd events when the 2nd enqueue throws"
#[tokio::test]
async fn continues_when_one_automatic_event_job_fails() {
    let queue = Queue {
        fail_on: Some("wamid.purchase-1"),
        ..Queue::default()
    };
    let raw = delivery(&[
        ("LeadSubmitted", "wamid.lead-1"),
        ("Purchase", "wamid.purchase-1"),
        ("LeadSubmitted", "wamid.lead-2"),
    ]);
    enqueue_webhook_jobs(
        &db(),
        &queue,
        &raw,
        None,
        chatbotx_worker::channels::whatsapp_webhook::AccountScope::Unscoped,
    )
    .await
    .expect("still acknowledged");
    let ids: Vec<Option<String>> = queue
        .added
        .lock()
        .unwrap()
        .iter()
        .map(|a| a.2.clone())
        .collect();
    assert_eq!(
        ids,
        [
            Some("ads-auto-phone-1-wamid.lead-1".into()),
            Some("ads-auto-phone-1-wamid.purchase-1".into()),
            Some("ads-auto-phone-1-wamid.lead-2".into()),
        ]
    );
}
