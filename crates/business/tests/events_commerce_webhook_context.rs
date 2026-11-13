//! Port of `packages/events/__tests__/commerce-webhook-context.test.ts`
//! (P3.6-X1): store events reach webhooks outside a channel context.

mod x1_support;

use std::sync::Arc;

use chatbotx_business::events::emitter::QueueEmitter;
use chatbotx_business::events::trigger::{order_status, tag_applied};
use serde_json::{Map, Value, json};
use x1_support::{FakeActive, RecordingEnqueuer};

fn order() -> Map<String, Value> {
    match json!({
        "orderId": "o-1",
        "orderNumber": 1,
        "status": "accepted",
        "source": "webshop",
        "total": 10,
        "currency": "USD",
        "discountCode": null,
        "channel": null,
        "conversationId": null,
        "products": [],
    }) {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

fn emitter() -> (QueueEmitter, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    let emitter = QueueEmitter::webhook(jobs.clone(), FakeActive::new(true));
    (emitter, jobs)
}

// webhooks for store events > an order event reaches webhooks outside a channel context
#[tokio::test]
async fn an_order_event_reaches_webhooks_outside_a_channel_context() {
    let (emitter, jobs) = emitter();
    emitter
        .emit(&order_status("orderAccepted", "ws-1", "c-1", order(), None))
        .await
        .expect("emit");
    assert_eq!(jobs.calls().len(), 1);
}

// webhooks for store events > the %s payment event reaches webhooks outside a channel context
#[tokio::test]
async fn the_payment_events_reach_webhooks_outside_a_channel_context() {
    for event_type in ["orderPaid", "orderRefunded"] {
        let (emitter, jobs) = emitter();
        emitter
            .emit(&order_status(event_type, "ws-1", "c-1", order(), None))
            .await
            .expect("emit");
        assert_eq!(jobs.calls().len(), 1, "{event_type}");
    }
}

// webhooks for store events > other events still need the channel context
#[tokio::test]
async fn other_events_still_need_the_channel_context() {
    let (emitter, jobs) = emitter();
    emitter
        .emit(&tag_applied("ws-1", "c-1", "tag-1", None))
        .await
        .expect("emit");
    assert!(jobs.calls().is_empty());
}
