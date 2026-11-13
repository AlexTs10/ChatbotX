//! Port of `packages/events/__tests__/trigger-emitter-contact-inbox-roundtrip.test.ts`
//! (P3.6-X1): the trigger emitter queues the event metadata unchanged. The
//! `triggerQueue.add` mock is a recording enqueuer, the `hasActiveTriggers`
//! mock a fake active check. The emitter methods are the event builders of
//! `events::trigger` plus `QueueEmitter::emit`.

mod x1_support;

use std::sync::Arc;

use chatbotx_business::events::emitter::QueueEmitter;
use chatbotx_business::events::trigger::{
    contact_created, shopping_cart_sent, tag_applied, user_asked_about_product,
};
use chatbotx_jobs::QueueName;
use serde_json::{Map, Value, json};
use x1_support::{FakeActive, RecordingEnqueuer};

fn emitter(active: bool) -> (QueueEmitter, Arc<RecordingEnqueuer>, Arc<FakeActive>) {
    let jobs = RecordingEnqueuer::new();
    let fake = FakeActive::new(active);
    (
        QueueEmitter::trigger(jobs.clone(), fake.clone()),
        jobs,
        fake,
    )
}

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

/// `data` of the first queued job.
fn queued(jobs: &RecordingEnqueuer) -> Value {
    let calls = jobs.calls();
    let call = calls.first().expect("a queued job");
    assert_eq!(call.queue, QueueName::Trigger);
    assert_eq!(call.name, "evaluate-triggers");
    call.data["data"].clone()
}

// TriggerEventEmitter — contactInboxId queue round-trip > tagApplied's contactInboxId lands in the queued job's eventData
#[tokio::test]
async fn tag_applied_contact_inbox_id_lands_in_the_queued_event_data() {
    let (emitter, jobs, _) = emitter(true);
    emitter
        .emit(&tag_applied(
            "ws-1",
            "contact-1",
            "tag-1",
            Some("ci-whatsapp"),
        ))
        .await
        .expect("emit");
    assert_eq!(jobs.calls().len(), 1);
    assert_eq!(
        queued(&jobs)["eventData"],
        json!({ "sourceId": "tag-1", "tagId": "tag-1", "contactInboxId": "ci-whatsapp" })
    );
}

// TriggerEventEmitter — contactInboxId queue round-trip > tagApplied omitted contactInboxId never adds the key to eventData
#[tokio::test]
async fn tag_applied_without_contact_inbox_id_never_adds_the_key() {
    let (emitter, jobs, _) = emitter(true);
    emitter
        .emit(&tag_applied("ws-1", "contact-1", "tag-1", None))
        .await
        .expect("emit");
    let event_data = queued(&jobs)["eventData"].clone();
    assert_eq!(event_data, json!({ "sourceId": "tag-1", "tagId": "tag-1" }));
    assert!(event_data.get("contactInboxId").is_none());
}

// TriggerEventEmitter — contactInboxId queue round-trip > contactCreated's contactInboxId lands in the queued job's eventData alongside the rest of the metadata
#[tokio::test]
async fn contact_created_contact_inbox_id_lands_alongside_the_rest_of_the_metadata() {
    let (emitter, jobs, _) = emitter(true);
    emitter
        .emit(&contact_created(
            "ws-1",
            "contact-1",
            Some("Ada"),
            None,
            Some("ada@example.com"),
            Some("ci-webchat"),
        ))
        .await
        .expect("emit");
    // `phone: undefined` of the original is a missing key in JSON.
    assert_eq!(
        queued(&jobs)["eventData"],
        json!({ "name": "Ada", "email": "ada@example.com", "contactInboxId": "ci-webchat" })
    );
}

fn reference() -> Value {
    json!({
        "catalogReference": { "source": "metaCatalog", "catalogId": "cat-1", "retailerId": "sku-1" },
        "product": null,
    })
}

fn base() -> Map<String, Value> {
    map(json!({
        "channel": "whatsapp",
        "conversationId": "conv-1",
        "messageId": "msg-1",
        "catalogId": "cat-1",
    }))
}

// TriggerEventEmitter — catalog events > shoppingCartSent queues its metadata and contactInboxId, never a sourceId
#[tokio::test]
async fn shopping_cart_sent_queues_its_metadata_and_contact_inbox_id() {
    let (emitter, jobs, active) = emitter(true);
    let mut metadata = base();
    metadata.insert("note".into(), Value::Null);
    let mut product = reference();
    product["quantity"] = json!(2);
    product["itemPrice"] = json!(5);
    product["currency"] = json!("USD");
    metadata.insert("products".into(), json!([product]));
    emitter
        .emit(&shopping_cart_sent(
            "ws-1",
            "contact-1",
            metadata.clone(),
            Some("ci-1"),
        ))
        .await
        .expect("emit");
    let data = queued(&jobs);
    assert_eq!(data["eventType"], json!("shoppingCartSent"));
    let mut expected = metadata;
    expected.insert("contactInboxId".into(), json!("ci-1"));
    assert_eq!(data["eventData"], Value::Object(expected));
    assert_eq!(
        active.calls(),
        vec![(
            "ws-1".to_string(),
            vec!["shoppingCartSent".to_string()],
            None
        )]
    );
}

// TriggerEventEmitter — catalog events > userAskedAboutProduct queues its metadata and contactInboxId, never a sourceId
#[tokio::test]
async fn user_asked_about_product_queues_its_metadata_and_contact_inbox_id() {
    let (emitter, jobs, _) = emitter(true);
    let mut metadata = base();
    metadata.insert("product".into(), reference());
    emitter
        .emit(&user_asked_about_product(
            "ws-1",
            "contact-1",
            metadata.clone(),
            Some("ci-1"),
        ))
        .await
        .expect("emit");
    let data = queued(&jobs);
    assert_eq!(data["eventType"], json!("userAskedAboutProduct"));
    let mut expected = metadata;
    expected.insert("contactInboxId".into(), json!("ci-1"));
    assert_eq!(data["eventData"], Value::Object(expected));
    assert!(data["eventData"].get("sourceId").is_none());
}

// TriggerEventEmitter — catalog events > queues nothing when no active trigger listens for the event
#[tokio::test]
async fn queues_nothing_when_no_active_trigger_listens_for_the_event() {
    let (emitter, jobs, _) = emitter(false);
    let mut metadata = base();
    metadata.insert("product".into(), reference());
    emitter
        .emit(&user_asked_about_product(
            "ws-1",
            "contact-1",
            metadata,
            None,
        ))
        .await
        .expect("emit");
    assert!(jobs.calls().is_empty());
}
