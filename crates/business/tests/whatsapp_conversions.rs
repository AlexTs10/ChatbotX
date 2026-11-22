//! Port of `integrations/whatsapp/__tests__/conversions.test.ts`.

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::meta_capi_send::{
    WhatsappEvent, ensure_whatsapp_dataset, send_whatsapp_event,
};
use serde_json::{Value, json};

const AT: i64 = 1_786_357_230_000; // 2026-08-10T10:20:30.000Z

struct Script {
    reply: String,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Script {
    fn new(reply: Value) -> Self {
        Script {
            reply: reply.to_string(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn only(&self) -> (String, Option<String>, Option<Value>) {
        let seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let r = &seen[0];
        let auth = r
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.clone());
        let body = r.body.as_deref().map(|b| serde_json::from_str(b).unwrap());
        (r.url.clone(), auth, body)
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().unwrap().push(request);
        Ok((200, self.reply.clone()))
    }
}

fn lead(id: &str, clid: &str) -> WhatsappEvent {
    WhatsappEvent {
        event_type: "lead".into(),
        occurred_at: AT,
        source_event_id: id.into(),
        ctwa_clid: clid.into(),
        waba_id: "waba-1".into(),
        ..WhatsappEvent::default()
    }
}

async fn sent(event: &WhatsappEvent) -> Value {
    let up = Script::new(json!({ "events_received": 1 }));
    send_whatsapp_event(&up, "v23.0", "dataset-1", "token-1", event)
        .await
        .unwrap();
    up.only().2.unwrap()["data"][0].clone()
}

/// Port of: "sendConversionEvent builds the WhatsApp business messaging payload"
#[tokio::test]
async fn builds_the_whatsapp_business_messaging_payload() {
    let up = Script::new(json!({ "events_received": 1 }));
    let event = WhatsappEvent {
        event_type: "purchase".into(),
        currency: Some("USD".into()),
        value: Some("42.50".into()),
        automatic: true,
        ..lead("source-event-1", "clid-1")
    };
    send_whatsapp_event(&up, "v23.0", "dataset-1", "token-1", &event)
        .await
        .unwrap();
    let (url, auth, body) = up.only();
    assert_eq!(url, "https://graph.facebook.com/v23.0/dataset-1/events");
    assert_eq!(auth.as_deref(), Some("Bearer token-1"));
    assert_eq!(
        body.unwrap(),
        json!({
            "data": [{
                "event_name": "Purchase",
                "event_time": 1_786_357_230,
                "event_id": "source-event-1",
                "action_source": "business_messaging",
                "messaging_channel": "whatsapp",
                "messaging_outcome_data": { "outcome_type": "automatic_events" },
                "user_data": { "whatsapp_business_account_id": "waba-1", "ctwa_clid": "clid-1" },
                "custom_data": { "currency": "USD", "value": 42.5 },
            }],
            "partner_agent": "ChatbotX",
        })
    );
}

/// Port of: "ensureDataset names the WABA dataset via dataset_name"
#[tokio::test]
async fn ensure_dataset_names_the_waba_dataset() {
    let up = Script::new(json!({ "id": "dataset-1" }));
    let id = ensure_whatsapp_dataset(
        &up,
        "v23.0",
        "waba-1",
        "token-1",
        Some("Shop Trần Event Data"),
    )
    .await
    .unwrap();
    assert_eq!(id, "dataset-1");
    let (url, auth, body) = up.only();
    assert_eq!(url, "https://graph.facebook.com/v23.0/waba-1/dataset");
    assert_eq!(auth.as_deref(), Some("Bearer token-1"));
    assert_eq!(
        body,
        Some(json!({ "dataset_name": "Shop Trần Event Data" }))
    );
}

/// Port of: "ensureDataset omits dataset_name when no name is given"
#[tokio::test]
async fn ensure_dataset_omits_dataset_name_without_a_name() {
    let up = Script::new(json!({ "id": "dataset-1" }));
    ensure_whatsapp_dataset(&up, "v23.0", "waba-1", "token-1", None)
        .await
        .unwrap();
    let (url, _, body) = up.only();
    assert_eq!(url, "https://graph.facebook.com/v23.0/waba-1/dataset");
    assert_eq!(body, None);
}

/// Port of: "maps lead events without purchase custom_data"
#[tokio::test]
async fn maps_lead_events_without_custom_data() {
    let data = sent(&lead("source-event-2", "clid-2")).await;
    assert_eq!(data["event_name"], "LeadSubmitted");
    assert_eq!(data["action_source"], "business_messaging");
    assert_eq!(data["messaging_channel"], "whatsapp");
    assert_eq!(
        data["user_data"],
        json!({ "whatsapp_business_account_id": "waba-1", "ctwa_clid": "clid-2" })
    );
    assert!(data.get("custom_data").is_none());
    assert!(data.get("messaging_outcome_data").is_none());
}

/// Port of: "merges hashed customer-info into user_data after the wa identity keys"
#[tokio::test]
async fn merges_hashed_customer_info_into_user_data() {
    let event = WhatsappEvent {
        user_data: Some(json!({ "em": ["hash-em"], "external_id": ["hash-ext"] })),
        ..lead("source-event-3", "clid-3")
    };
    let data = sent(&event).await;
    assert_eq!(
        data["user_data"],
        json!({
            "whatsapp_business_account_id": "waba-1", "ctwa_clid": "clid-3",
            "em": ["hash-em"], "external_id": ["hash-ext"],
        })
    );
}

/// Port of: "emits the fixed top-level Limited Data Use triple when limitedDataUse is true"
#[tokio::test]
async fn emits_the_limited_data_use_triple() {
    let event = WhatsappEvent {
        limited_data_use: true,
        ..lead("source-event-4", "clid-4")
    };
    let data = sent(&event).await;
    assert_eq!(data["data_processing_options"], json!(["LDU"]));
    assert_eq!(data["data_processing_options_country"], json!(0));
    assert_eq!(data["data_processing_options_state"], json!(0));
}

/// Port of: "renders Purchase order_id and contents with num_items summed by quantity"
#[tokio::test]
async fn renders_purchase_order_id_and_contents() {
    let event = WhatsappEvent {
        event_type: "purchase".into(),
        currency: Some("USD".into()),
        value: Some("35".into()),
        order_id: Some("order-123".into()),
        contents: Some(vec![
            json!({ "id": "sku-1", "quantity": 2, "itemPrice": 10 }),
            json!({ "id": "sku-2", "quantity": 1, "itemPrice": 15 }),
        ]),
        ..lead("source-event-5", "clid-5")
    };
    let data = sent(&event).await;
    assert_eq!(
        data["custom_data"],
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
