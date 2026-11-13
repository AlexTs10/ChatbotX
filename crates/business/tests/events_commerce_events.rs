//! Port of `packages/events/__tests__/commerce-events.test.ts` (P3.6-X1):
//! `emitCartItemChanged`. The two mocked emitters are two recording
//! subscribers of the bus.

use chatbotx_business::events::trigger::emit_cart_item_changed;
use chatbotx_business::events::{EventBus, RecordingSubscriber};
use serde_json::{Map, Value, json};

fn metadata(change: i64, category_ids: &[&str]) -> Map<String, Value> {
    match json!({
        "channel": "webshop",
        "product": { "productId": "p-1", "variantId": null, "name": "Shirt", "categoryIds": category_ids },
        "quantity": change.max(0),
        "change": change,
    }) {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

fn bus() -> (
    EventBus,
    std::sync::Arc<RecordingSubscriber>,
    std::sync::Arc<RecordingSubscriber>,
) {
    let bus = EventBus::new();
    let trigger = RecordingSubscriber::new();
    let webhook = RecordingSubscriber::new();
    bus.subscribe(trigger.clone());
    bus.subscribe(webhook.clone());
    (bus, trigger, webhook)
}

fn types(subscriber: &RecordingSubscriber) -> Vec<String> {
    subscriber
        .trigger_events()
        .into_iter()
        .map(|event| event.event_type)
        .collect()
}

// emitCartItemChanged > an addition is productAddedToCart and categoryAddedToCart
#[tokio::test]
async fn an_addition_is_product_added_to_cart_and_category_added_to_cart() {
    let (bus, trigger, webhook) = bus();
    emit_cart_item_changed(&bus, "ws-1", "c-1", &metadata(2, &["cat-1"]), Some("ci-1")).await;
    assert_eq!(
        types(&trigger),
        ["productAddedToCart", "categoryAddedToCart"]
    );
    assert_eq!(webhook.events().len(), 2);
}

// emitCartItemChanged > a product without a category emits no category event
#[tokio::test]
async fn a_product_without_a_category_emits_no_category_event() {
    let (bus, trigger, _webhook) = bus();
    emit_cart_item_changed(&bus, "ws-1", "c-1", &metadata(1, &[]), None).await;
    assert_eq!(types(&trigger), ["productAddedToCart"]);
}

// emitCartItemChanged > a removal is productRemovedFromCart only
#[tokio::test]
async fn a_removal_is_product_removed_from_cart_only() {
    let (bus, trigger, _webhook) = bus();
    emit_cart_item_changed(&bus, "ws-1", "c-1", &metadata(-1, &["cat-1"]), None).await;
    assert_eq!(types(&trigger), ["productRemovedFromCart"]);
}
