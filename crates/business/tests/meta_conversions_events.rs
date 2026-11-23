//! Port of `integrations/meta-conversions/__tests__/events.test.ts` (the
//! request body; the Graph path is `v24.0/{datasetId}/events`).

use chatbotx_business::meta_capi_send::{Identity, MetaEvent, meta_events_body};
use serde_json::{Value, json};

const AT: i64 = 1_786_357_230_000; // 2026-08-10T10:20:30.000Z

fn messenger(event_id: &str) -> MetaEvent {
    MetaEvent {
        event_name: "LeadSubmitted".into(),
        occurred_at: AT,
        event_id: event_id.into(),
        identity: Some(Identity::Messenger {
            page_id: "page-1".into(),
            psid: "psid-1".into(),
        }),
        ..MetaEvent::default()
    }
}

fn first(body: &Value) -> &Value {
    &body["data"][0]
}

fn messenger_lead_body() -> Value {
    json!({
        "data": [{
            "event_name": "LeadSubmitted",
            "event_time": 1_786_357_230,
            "event_id": "event-1",
            "action_source": "business_messaging",
            "messaging_channel": "messenger",
            "user_data": { "page_id": "page-1", "page_scoped_user_id": "psid-1" },
            "custom_data": {
                "currency": "USD", "value": 42.5,
                "content_category": "Education", "content_name": "Landing Page A",
            },
        }],
        "partner_agent": "ChatConnectX",
    })
}

fn messenger_lead() -> MetaEvent {
    MetaEvent {
        currency: Some("USD".into()),
        value: Some("42.50".into()),
        content_category: Some("Education".into()),
        content_name: Some("Landing Page A".into()),
        ..messenger("event-1")
    }
}

/// Port of: "builds the Messenger business messaging payload"
#[test]
fn builds_the_messenger_business_messaging_payload() {
    assert_eq!(
        meta_events_body(&messenger_lead(), None),
        messenger_lead_body()
    );
}

/// Port of: "builds the Instagram business messaging payload"
#[test]
fn builds_the_instagram_business_messaging_payload() {
    let event = MetaEvent {
        event_id: "event-2".into(),
        identity: Some(Identity::Instagram {
            account_id: "ig-business-1".into(),
            igsid: "ig-sid-1".into(),
        }),
        ..messenger("event-2")
    };
    let body = meta_events_body(&event, None);
    let data = first(&body);
    assert_eq!(data["messaging_channel"], "instagram");
    assert_eq!(data["action_source"], "business_messaging");
    assert_eq!(
        data["user_data"],
        json!({ "ig_account_id": "ig-business-1", "instagram_business_account_id": "ig-business-1", "ig_sid": "ig-sid-1" })
    );
    assert!(data.get("custom_data").is_none());
}

/// Port of: "builds the WhatsApp business messaging payload"
#[test]
fn builds_the_whatsapp_business_messaging_payload() {
    let event = MetaEvent {
        identity: Some(Identity::Whatsapp {
            waba_id: "waba-1".into(),
            ctwa_clid: "clid-1".into(),
        }),
        currency: Some("USD".into()),
        value: Some("42.50".into()),
        ..messenger("event-4")
    };
    assert_eq!(
        meta_events_body(&event, None),
        json!({
            "data": [{
                "event_name": "LeadSubmitted",
                "event_time": 1_786_357_230,
                "event_id": "event-4",
                "action_source": "business_messaging",
                "messaging_channel": "whatsapp",
                "user_data": { "whatsapp_business_account_id": "waba-1", "ctwa_clid": "clid-1" },
                "custom_data": { "currency": "USD", "value": 42.5 },
            }],
            "partner_agent": "ChatConnectX",
        })
    );
}

/// Port of: "includes content fields in the Instagram custom_data"
#[test]
fn includes_content_fields_in_the_instagram_custom_data() {
    let event = MetaEvent {
        identity: Some(Identity::Instagram {
            account_id: "ig-business-1".into(),
            igsid: "ig-sid-1".into(),
        }),
        content_category: Some("Education".into()),
        content_name: Some("Landing Page A".into()),
        ..messenger("event-3")
    };
    let body = meta_events_body(&event, None);
    assert_eq!(
        first(&body)["custom_data"],
        json!({ "content_category": "Education", "content_name": "Landing Page A" })
    );
}

/// Port of: "merges hashed customer-info into user_data after the channel identity keys"
#[test]
fn merges_hashed_customer_info_into_user_data() {
    let event = MetaEvent {
        user_data: Some(
            json!({ "em": ["hash-em"], "ph": ["hash-ph"], "external_id": ["hash-ext"] }),
        ),
        ..messenger("event-5")
    };
    let body = meta_events_body(&event, None);
    assert_eq!(
        first(&body)["user_data"],
        json!({
            "page_id": "page-1", "page_scoped_user_id": "psid-1",
            "em": ["hash-em"], "ph": ["hash-ph"], "external_id": ["hash-ext"],
        })
    );
}

/// Port of: "emits the fixed top-level Limited Data Use triple when limitedDataUse is true"
/// (+ "omits data_processing_options entirely when limitedDataUse is false/absent")
#[test]
fn limited_data_use_triple_only_when_set() {
    let on = MetaEvent {
        limited_data_use: true,
        ..messenger("event-6")
    };
    let body = meta_events_body(&on, None);
    let data = first(&body);
    assert_eq!(data["data_processing_options"], json!(["LDU"]));
    assert_eq!(data["data_processing_options_country"], json!(0));
    assert_eq!(data["data_processing_options_state"], json!(0));
    let off = meta_events_body(&messenger("event-7"), None);
    assert!(first(&off).get("data_processing_options").is_none());
}

/// Port of: "renders Purchase order_id and contents with num_items summed by quantity"
#[test]
fn renders_purchase_order_id_and_contents() {
    let event = MetaEvent {
        event_name: "Purchase".into(),
        currency: Some("USD".into()),
        value: Some("35".into()),
        order_id: Some("order-123".into()),
        contents: Some(vec![
            json!({ "id": "sku-1", "quantity": 2, "itemPrice": 10 }),
            json!({ "id": "sku-2", "quantity": 1, "itemPrice": 15 }),
        ]),
        ..messenger("event-8")
    };
    let body = meta_events_body(&event, None);
    assert_eq!(
        first(&body)["custom_data"],
        json!({
            "currency": "USD", "value": 35, "order_id": "order-123",
            "content_type": "product", "num_items": 3,
            "contents": [
                { "id": "sku-1", "quantity": 2, "item_price": 10 },
                { "id": "sku-2", "quantity": 1, "item_price": 15 },
            ],
        })
    );
}

/// Port of: "sends test_event_code at the request top level only when provided"
#[test]
fn sends_test_event_code_at_the_top_level_only_when_provided() {
    let event = MetaEvent {
        event_name: "Purchase".into(),
        value: Some("250".into()),
        currency: Some("VND".into()),
        ..messenger("event-test")
    };
    let with_code = meta_events_body(&event, Some("TEST33520"));
    let without = meta_events_body(&event, None);
    assert_eq!(with_code["test_event_code"], "TEST33520");
    assert!(without.get("test_event_code").is_none());
    assert_eq!(with_code["data"], without["data"]);
}

/// Port of: "explicit actionSource business_messaging is identical to omitting it"
#[test]
fn explicit_business_messaging_is_identical_to_omitting_it() {
    // The identity carries the action source: a messaging identity is
    // `business_messaging` whether or not it was named.
    assert_eq!(
        meta_events_body(&messenger_lead(), None),
        messenger_lead_body()
    );
}

/// Port of: "builds a non-messaging email event with no messaging identity"
#[test]
fn builds_a_non_messaging_email_event() {
    let hashed = json!({ "em": ["hash-em"], "ph": ["hash-ph"], "external_id": ["hash-ext"] });
    let event = MetaEvent {
        event_name: "Lead".into(),
        occurred_at: AT,
        event_id: "event-9".into(),
        identity: Some(Identity::NonMessaging {
            action_source: "email".into(),
        }),
        user_data: Some(hashed.clone()),
        ..MetaEvent::default()
    };
    let body = meta_events_body(&event, None);
    let data = first(&body);
    assert_eq!(data["event_name"], "Lead");
    assert_eq!(data["action_source"], "email");
    assert!(data.get("messaging_channel").is_none());
    assert_eq!(data["user_data"], hashed);
}

/// Port of: "includes content_type and content_ids when provided, omits them when absent"
#[test]
fn includes_content_type_and_ids_only_when_provided() {
    let event = MetaEvent {
        event_name: "ViewContent".into(),
        content_type: Some("product_group".into()),
        content_ids: Some(vec!["sku-1".into(), "sku-2".into()]),
        ..messenger("event-10")
    };
    let body = meta_events_body(&event, None);
    assert_eq!(
        first(&body)["custom_data"],
        json!({ "content_type": "product_group", "content_ids": ["sku-1", "sku-2"] })
    );
    let bare = MetaEvent {
        event_name: "ViewContent".into(),
        ..messenger("event-11")
    };
    assert!(
        first(&meta_events_body(&bare, None))
            .get("custom_data")
            .is_none()
    );
}

/// Port of: "explicit contentType product_group beats the contents[]-derived product default"
#[test]
fn explicit_content_type_beats_the_contents_default() {
    let event = MetaEvent {
        event_name: "Purchase".into(),
        content_type: Some("product_group".into()),
        contents: Some(vec![
            json!({ "id": "sku-1", "quantity": 1, "itemPrice": 10 }),
        ]),
        ..messenger("event-12")
    };
    let body = meta_events_body(&event, None);
    assert_eq!(first(&body)["custom_data"]["content_type"], "product_group");
    assert_eq!(first(&body)["custom_data"]["num_items"], json!(1));
}

/// Port of: "passes a custom event name through unchanged"
#[test]
fn passes_a_custom_event_name_through_unchanged() {
    let event = MetaEvent {
        event_name: "MyCustomEvent".into(),
        occurred_at: AT,
        event_id: "event-13".into(),
        identity: Some(Identity::NonMessaging {
            action_source: "other".into(),
        }),
        user_data: Some(json!({ "external_id": ["hash-ext"] })),
        ..MetaEvent::default()
    };
    let body = meta_events_body(&event, None);
    assert_eq!(first(&body)["event_name"], "MyCustomEvent");
    assert_eq!(first(&body)["action_source"], "other");
}
