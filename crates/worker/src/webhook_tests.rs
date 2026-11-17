//! `webhook-executor-payloads.test.ts`, `webhook-matcher-catalog.test.ts`,
//! `webhook-matcher-datetime.test.ts`, `webhook-pipeline-payload.test.ts`
//! and `webhook-payload-contact-inbox-leak.test.ts`. The mocked services are
//! DB rows (workspace 2, contact 3); the executor mock is a recording
//! transport. Ids are numeric strings.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{RecordingWebhookHttp, WebhookEvent, build_payload, find_and_execute};
use crate::steps::test_support::Fixture;

const TS: &str = "2026-07-11T10:00:00.000Z";

fn event(event_type: &str, metadata: Value) -> WebhookEvent {
    WebhookEvent {
        workspace_id: "2".into(),
        contact_id: "3".into(),
        event_type: event_type.into(),
        metadata: metadata.as_object().cloned().unwrap_or_default(),
        timestamp: json!(TS),
    }
}

fn base(name: &str) -> Value {
    json!({ "event": name, "contact_id": "3", "timestamp": TS })
}

fn with(mut a: Value, b: Value) -> Value {
    for (k, v) in b.as_object().unwrap() {
        a[k] = v.clone();
    }
    a
}

async fn payload_fixture() -> Fixture {
    let f = Fixture::new().await;
    f.exec(
        r#"UPDATE "Contact" SET "phoneNumber" = '+15551234567', "email" = 'ada@example.com' WHERE "id" = 3;
           INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (9, 'Other', 1);
           INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (40, 'VIP', 2), (41, 'Other workspace tag', 9);
           INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (20, 'plan', 'shortText', 2);
           INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (21, 3, 20, 'Pro');"#,
    )
    .await;
    f
}

#[tokio::test]
async fn sends_the_current_payload_for_each_event_type() {
    let f = payload_fixture().await;
    let cases = [
        (
            "tagApplied",
            json!({ "tagId": "40" }),
            with(base("tag_applied"), json!({ "tag": "VIP" })),
        ),
        (
            "tagRemoved",
            json!({ "tagId": "40" }),
            with(base("tag_removed"), json!({ "tag": "VIP" })),
        ),
        (
            "customFieldValueChanged",
            json!({ "customFieldName": "Plan", "oldValue": "Free", "newValue": "Pro" }),
            with(
                base("custom_field_changed"),
                json!({ "custom_field": { "name": "Plan", "old_value": "Free", "new_value": "Pro" } }),
            ),
        ),
        (
            "conversationTransferredToHuman",
            json!({ "conversationId": "conversation-1", "transferredBy": "bot" }),
            with(
                base("conversation_transferred_to_human"),
                json!({ "conversation_id": "conversation-1", "transferred_by": "bot" }),
            ),
        ),
        (
            "conversationTransferredToBot",
            json!({ "conversationId": "conversation-1" }),
            with(
                base("conversation_transferred_to_bot"),
                json!({ "conversation_id": "conversation-1", "transferred_by": "system" }),
            ),
        ),
        (
            // The stored contact, not the metadata, is the source of truth.
            "newContact",
            json!({ "name": "Ada", "phone": "+15550000000", "email": "stale@example.com" }),
            with(
                base("new_contact"),
                json!({ "name": "Ada Lovelace", "first_name": "Ada", "last_name": "Lovelace", "phone": "+15551234567", "email": "ada@example.com", "custom_fields": { "plan": "Pro" } }),
            ),
        ),
        (
            "contactUnsubscribedFormBroadcast",
            json!({}),
            base("contact_unsubscribed"),
        ),
        (
            "contactBlockedBot",
            json!({ "channel": "telegram" }),
            with(
                base("contact_blocked_bot"),
                json!({ "channel": "telegram" }),
            ),
        ),
        (
            "contactUnblockedBot",
            json!({ "channel": "telegram" }),
            with(
                base("contact_unblocked_bot"),
                json!({ "channel": "telegram" }),
            ),
        ),
        (
            "archived",
            json!({ "conversationId": "conversation-1" }),
            with(
                base("conversation_archived"),
                json!({ "conversation_id": "conversation-1", "archived_by": "system" }),
            ),
        ),
        (
            "followUp",
            json!({ "conversationId": "conversation-1", "markedBy": "agent-1" }),
            with(
                base("marked_as_follow_up"),
                json!({ "conversation_id": "conversation-1", "marked_by": "agent-1" }),
            ),
        ),
        (
            "conversationAssigned",
            json!({ "conversationId": "conversation-1", "assignedTo": "agent-2", "assignedBy": "agent-1" }),
            with(
                base("conversation_assigned"),
                json!({ "conversation_id": "conversation-1", "assigned_to": "agent-2", "assigned_by": "agent-1" }),
            ),
        ),
        (
            "conversationUnassigned",
            json!({ "conversationId": "conversation-1" }),
            with(
                base("conversation_unassigned"),
                json!({ "conversation_id": "conversation-1", "unassigned_by": "system" }),
            ),
        ),
        (
            "subscribedToSequence",
            json!({ "sequenceId": "sequence-1", "sequenceName": "Welcome" }),
            with(
                base("subscribed_to_sequence"),
                json!({ "sequence_id": "sequence-1", "sequence_name": "Welcome" }),
            ),
        ),
        (
            "unsubscribedFromSequence",
            json!({ "sequenceId": "sequence-1", "sequenceName": "Welcome" }),
            with(
                base("unsubscribed_from_sequence"),
                json!({ "sequence_id": "sequence-1", "sequence_name": "Welcome" }),
            ),
        ),
        (
            "contactReferredANewContact",
            json!({ "refName": "Summer", "reflinkId": "reflink-1" }),
            with(
                base("contact_referred_a_new_contact"),
                json!({ "ref_name": "Summer", "reflink_id": "reflink-1" }),
            ),
        ),
        (
            "contactReferredExistingContact",
            json!({ "refName": "Summer", "reflinkId": "reflink-1" }),
            with(
                base("contact_referred_existing_contact"),
                json!({ "ref_name": "Summer", "reflink_id": "reflink-1" }),
            ),
        ),
        (
            "dateTimeBasedTrigger",
            json!({ "sourceId": "custom-field-1" }),
            with(
                base("datetime_based_trigger"),
                json!({ "sourceId": "custom-field-1" }),
            ),
        ),
        (
            "shoppingCartSent",
            json!({
                "channel": "whatsapp", "conversationId": "conversation-1", "messageId": "message-1", "catalogId": "catalog-1",
                "note": "after 5pm", "contactInboxId": "contact-inbox-internal",
                "products": [
                    { "catalogReference": { "source": "metaCatalog", "catalogId": "catalog-1", "retailerId": "sku-1" },
                      "product": { "productId": "product-1", "name": "Shoe", "imageUrl": null, "price": 10, "currency": "USD" },
                      "quantity": 2, "itemPrice": 10, "currency": "USD" },
                    { "catalogReference": { "source": "metaCatalog", "catalogId": "catalog-1", "retailerId": "sku-unknown" },
                      "product": null, "quantity": 1, "itemPrice": 3, "currency": "USD" },
                ],
            }),
            with(
                base("shopping_cart_sent"),
                json!({
                    "channel": "whatsapp", "conversation_id": "conversation-1", "message_id": "message-1", "catalog_id": "catalog-1", "note": "after 5pm",
                    "products": [
                        { "product_id": "product-1", "retailer_id": "sku-1", "name": "Shoe", "quantity": 2, "item_price": 10, "currency": "USD" },
                        { "product_id": null, "retailer_id": "sku-unknown", "name": null, "quantity": 1, "item_price": 3, "currency": "USD" },
                    ],
                }),
            ),
        ),
        (
            "userAskedAboutProduct",
            json!({
                "channel": "whatsapp", "conversationId": "conversation-1", "messageId": "message-1", "catalogId": "catalog-1",
                "contactInboxId": "contact-inbox-internal",
                "product": { "catalogReference": { "source": "metaCatalog", "catalogId": "catalog-1", "retailerId": "sku-unknown" }, "product": null },
            }),
            with(
                base("user_asked_about_product"),
                json!({
                    "channel": "whatsapp", "conversation_id": "conversation-1", "message_id": "message-1", "catalog_id": "catalog-1",
                    "product": { "product_id": null, "retailer_id": "sku-unknown", "name": null },
                }),
            ),
        ),
    ];
    let http = Arc::new(RecordingWebhookHttp::default());
    let webhook = super::ActiveWebhook {
        id: 1,
        url: "https://example.com/webhook".into(),
        conditions: vec![],
    };
    for (t, metadata, expected) in cases {
        let payload = build_payload(&f.deps, &event(t, metadata)).await.unwrap();
        super::execute(&*http, &webhook, &payload.to_string()).await;
        let (url, body) = http.posts.lock().unwrap().pop().unwrap();
        assert_eq!(url, "https://example.com/webhook");
        assert_eq!(body, expected, "{t}");
    }
}

#[tokio::test]
async fn does_not_leak_a_tag_of_another_workspace() {
    let f = payload_fixture().await;
    let payload = build_payload(&f.deps, &event("tagApplied", json!({ "tagId": "41" })))
        .await
        .unwrap();
    assert_eq!(payload, with(base("tag_applied"), json!({ "tag": "" })));
}

#[tokio::test]
async fn falls_back_to_metadata_when_the_contact_no_longer_exists() {
    let f = payload_fixture().await;
    let mut e = event(
        "newContact",
        json!({ "name": "Ada", "phone": "+15550000000" }),
    );
    e.contact_id = "999".into();
    let payload = build_payload(&f.deps, &e).await.unwrap();
    assert_eq!(
        payload,
        json!({ "event": "new_contact", "contact_id": "999", "timestamp": TS, "name": "Ada", "first_name": null,
                "last_name": null, "phone": "+15550000000", "email": null, "custom_fields": {} })
    );
}

fn item() -> Value {
    json!({
        "pipelineId": "p-1", "pipelineName": "Sales", "pipelineReferenceName": "sales", "itemId": "i-1", "itemNumber": 12,
        "title": "Big deal", "contactId": "c-1", "stageId": "s-2", "stageName": "Negotiation", "status": "open",
        "statusLabel": "Open", "priority": "high", "value": 1500, "currency": "USD", "dueDate": null,
        "actorKind": "user", "actorUserId": "u-1", "contactInboxId": "ci-secret",
    })
}

#[tokio::test]
async fn pipeline_item_payloads() {
    let f = Fixture::new().await;
    for (t, name) in [
        ("pipelineItemCreated", "pipeline_item_created"),
        ("pipelineItemMovedToStage", "pipeline_item_moved_to_stage"),
        ("pipelineItemValueChanged", "pipeline_item_value_changed"),
        ("pipelineItemStatusChanged", "pipeline_item_status_changed"),
        (
            "pipelineItemPriorityChanged",
            "pipeline_item_priority_changed",
        ),
    ] {
        let p = build_payload(&f.deps, &event(t, item())).await.unwrap();
        for (k, v) in [
            ("event", json!(name)),
            ("contact_id", json!("3")),
            ("pipeline_id", json!("p-1")),
            ("pipeline_reference_name", json!("sales")),
            ("item_id", json!("i-1")),
            ("item_number", json!(12)),
            ("stage_name", json!("Negotiation")),
            ("status_label", json!("Open")),
            ("value", json!(1500)),
            ("currency", json!("USD")),
            ("actor_kind", json!("user")),
        ] {
            assert_eq!(p[k], v, "{t} {k}");
        }
        assert!(!p.to_string().contains("ci-secret"));
    }
    let created = build_payload(&f.deps, &event("pipelineItemCreated", item()))
        .await
        .unwrap();
    let moved = build_payload(
        &f.deps,
        &event(
            "pipelineItemMovedToStage",
            with(
                item(),
                json!({ "fromStageId": "s-1", "fromStageName": "Lead" }),
            ),
        ),
    )
    .await
    .unwrap();
    let value_changed = build_payload(
        &f.deps,
        &event(
            "pipelineItemValueChanged",
            with(item(), json!({ "previousValue": null })),
        ),
    )
    .await
    .unwrap();
    assert!(created.get("from_stage_id").is_none() && created.get("previous_value").is_none());
    assert_eq!(
        (
            moved["from_stage_id"].clone(),
            moved["from_stage_name"].clone()
        ),
        (json!("s-1"), json!("Lead"))
    );
    assert_eq!(value_changed.get("previous_value"), Some(&Value::Null));
}

#[tokio::test]
async fn contact_inbox_id_never_leaks_from_projecting_builders() {
    let f = payload_fixture().await;
    let sample = json!({ "tagId": "40", "conversationId": "c", "channel": "whatsapp", "orderId": "o", "products": [], "items": [],
                         "product": { "productId": "p", "categoryIds": [] }, "contactInboxId": "LEAK-SENTINEL" });
    // The TS list leaves out the spreading builders (dateTimeBasedTrigger, call events).
    for t in chatbotx_business::events::registry::EMITTED_EVENT_TYPES
        .iter()
        .filter(|t| !super::SPREAD_EVENT_TYPES.contains(t))
    {
        let p = build_payload(&f.deps, &event(t, sample.clone()))
            .await
            .unwrap();
        assert!(!p.to_string().contains("LEAK-SENTINEL"), "{t}");
    }
    // The documented spread exception: dateTimeBasedTrigger passes its metadata through.
    let p = build_payload(
        &f.deps,
        &event(
            "dateTimeBasedTrigger",
            json!({ "sourceId": "cf", "contactInboxId": "LEAK-SENTINEL" }),
        ),
    )
    .await
    .unwrap();
    assert_eq!(p["contactInboxId"], json!("LEAK-SENTINEL"));
}

/// Webhooks 50 / 51 (… with `conditions`) in workspace 2.
async fn webhooks(f: &Fixture, rows: &'static str) {
    f.exec(rows).await;
}

fn job(event_type: &str, metadata: Value) -> Value {
    json!({ "workspaceId": "2", "contactId": "3", "eventType": event_type, "eventData": metadata, "timestamp": TS })
}

fn cart(product: &str) -> Value {
    json!({ "products": [{ "catalogReference": { "source": "metaCatalog", "catalogId": "cat-1", "retailerId": "sku" },
                           "product": { "productId": product, "name": "n", "imageUrl": null, "price": 1, "currency": "USD" },
                           "quantity": 1 }] })
}

#[tokio::test]
async fn catalog_matching() {
    let mut f = Fixture::new().await;
    let http = Arc::new(RecordingWebhookHttp::default());
    f.deps.webhook_http = http.clone();
    webhooks(
        &f,
        r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId", "active") VALUES (50, 'mixed', 'https://a.test/h', 2, 1);
           INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId") VALUES (1, 50, 'shoppingCartSent', '101'), (2, 50, 'userAskedAboutProduct', NULL);"#,
    )
    .await;
    // a sibling condition for another event cannot override the product filter
    assert!(
        find_and_execute(&f.deps, &job("shoppingCartSent", cart("202")))
            .await
            .unwrap()
            .is_empty()
    );
    // the same webhook still fires for a cart that contains its product
    assert_eq!(
        find_and_execute(&f.deps, &job("shoppingCartSent", cart("101")))
            .await
            .unwrap(),
        vec![50]
    );
    // an any-product condition for the event fires for any cart
    f.exec(r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId", "active") VALUES (51, 'any', 'https://b.test/h', 2, 1);
              INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId") VALUES (3, 51, 'shoppingCartSent', NULL);"#)
        .await;
    assert_eq!(
        find_and_execute(&f.deps, &job("shoppingCartSent", cart("999")))
            .await
            .unwrap(),
        vec![51]
    );
    assert_eq!(http.posts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn datetime_matching() {
    let mut f = Fixture::new().await;
    let http = Arc::new(RecordingWebhookHttp::default());
    f.deps.webhook_http = http.clone();
    webhooks(
        &f,
        r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId", "active") VALUES (50, 'w1', 'https://a.test/h', 2, 1), (51, 'w2', 'https://b.test/h', 2, 1);
           INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId", "value") VALUES
             (1, 50, 'dateTimeBasedTrigger', '20', '{"triggerType":"after","timeValue":2,"timeType":"days"}'),
             (2, 51, 'dateTimeBasedTrigger', '20', '{"triggerType":"after","timeValue":2,"timeType":"days"}');"#,
    )
    .await;
    // the scanner already matched the source id: no re-evaluation (no custom field value exists)
    let mut ids = find_and_execute(
        &f.deps,
        &job("dateTimeBasedTrigger", json!({ "sourceId": "20" })),
    )
    .await
    .unwrap();
    ids.sort_unstable();
    assert_eq!(ids, vec![50, 51]);
    // one payload for both deliveries
    let posts = http.posts.lock().unwrap().clone();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0].1,
        json!({ "event": "datetime_based_trigger", "contact_id": "3", "timestamp": TS, "sourceId": "20" })
    );
    assert_eq!(posts[0].1, posts[1].1);
    // a different source id executes nothing
    assert!(
        find_and_execute(
            &f.deps,
            &job("dateTimeBasedTrigger", json!({ "sourceId": "21" }))
        )
        .await
        .unwrap()
        .is_empty()
    );
    // one failing delivery does not stop the other
    http.failing.lock().unwrap().push("https://a.test/h".into());
    assert_eq!(
        find_and_execute(
            &f.deps,
            &job("dateTimeBasedTrigger", json!({ "sourceId": "20" }))
        )
        .await
        .unwrap()
        .len(),
        2
    );
    assert_eq!(http.posts.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn blocked_workspace_and_unmatchable_events_do_nothing() {
    let mut f = Fixture::new().await;
    let http = Arc::new(RecordingWebhookHttp::default());
    f.deps.webhook_http = http.clone();
    webhooks(
        &f,
        r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId", "active") VALUES (50, 'w', 'https://a.test/h', 2, 1);
           INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId") VALUES (1, 50, 'tagApplied', '40');
           UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = 2;"#,
    )
    .await;
    let job_ctx = crate::steps::test_support::job(
        json!({ "type": "evaluateWebhooks", "data": job("tagApplied", json!({ "sourceId": "40", "tagId": "40" })) }),
    );
    super::handle(f.deps.clone(), job_ctx).await.unwrap();
    assert!(http.posts.lock().unwrap().is_empty());
    assert!(
        find_and_execute(&f.deps, &job("notAnEvent", json!({})))
            .await
            .unwrap()
            .is_empty()
    );
}
