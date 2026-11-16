//! Port of `apps/builder/__tests__/contacts-messages-public-api.test.ts`
//! (P3.6-D2, slice contact messages). Adapted: the captured oRPC handlers and
//! the mocked `conversationService` / `messageService` /
//! `automatedResponseService` / `listMessages` become `handlers::dispatch`
//! calls over real rows; "createOutgoing was called with X" becomes the
//! stored `Message` row (the send / flow jobs wait for the worker port).

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Contact 123 on webchat inbox 5 (contact inbox 50, conversation 70);
/// contact 124 without a conversation; flow 300; inbound keyword "hello" →
/// "Welcome!".
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (5, ?1, 'Web', 'webchat', 'web-5')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (123, ?1, 'Ada'), (124, ?1, 'Silent')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (50, 123, 123, 5, 'webchat', 'direct', 'guest-123'), (51, 124, 124, 5, 'webchat', 'direct', 'guest-124')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (70, ?1, 123)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (300, ?1, 'Welcome flow')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "text", "type")
                   VALUES (400, ?1, '["hello"]', 1, 'Welcome!', 'inbound')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

/// An outgoing api text message of contact 123's conversation.
async fn message(f: &Fixture, id: i64, contact_inbox_id: i64, text: &'static str, created_at: i64) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
                     "messageType", "contentType", "senderType")
                   VALUES (?1, ?2, ?2, 70, ?3, ?4, ?5, 'outgoing', 'text', 'api')"#,
                params![id, created_at, contact_inbox_id, WS, text],
            )?;
            Ok(())
        })
        .await
        .expect("seed message");
}

async fn post(f: &Fixture, route: &str, identifier: &'static str, body: Value) -> (u16, Value) {
    call(
        f,
        route,
        WS,
        &[("identifier", identifier)],
        None,
        Some(body),
    )
    .await
}

async fn stored_texts(f: &Fixture, conversation_id: i64) -> Vec<String> {
    let rows = scalar(
        f,
        r#"SELECT json_group_array("text") FROM (SELECT "text" FROM "Message" WHERE "conversationId" = ?1 ORDER BY "id")"#,
        conversation_id,
    )
    .await;
    rows.and_then(|v| v.as_str().map(str::to_string))
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

// ─── POST /v1/contacts/{identifier}/messages ────────────────────────────

#[tokio::test]
async fn post_messages_resolves_the_contact_inbox_via_resolvecontactinboxforsend_and_sends() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.sendMessage",
        "id:123",
        json!({ "text": "hello", "inboxId": "5" }),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored_texts(&f, 70).await, vec!["hello"]);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT json_array("contactInboxId", "messageType", "senderType", "contentType", "workspaceId") FROM "Message" WHERE "conversationId" = ?1"#,
            70,
        )
        .await,
        Some(json!(format!("[50,\"outgoing\",\"api\",\"text\",{WS}]")))
    );
}

/// Adapted: product 1 is a real, published product of the workspace; the
/// stored message carries the resolved product content (the TS case checks
/// that the input reaches `createOutgoing` unchanged).
#[tokio::test]
async fn post_messages_hands_a_product_message_to_createoutgoing_unchanged() {
    let f = fixture().await;
    seed(&f).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Product" ("id", "workspaceId", "name", "price", "status") VALUES (1, ?1, 'Mug', 9.5, 'active')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed product");
    let (status, body) = post(
        &f,
        "contacts.sendMessage",
        "id:123",
        json!({
            "inboxId": "5",
            "products": { "display": "products", "body": "Look", "sections": [{ "productIds": ["1"] }] },
        }),
    )
    .await;
    // Orchestrator fix (P3.6-D3): only WhatsApp has a product catalog source
    // (packages/business/src/product-message/catalog-sources.ts:169), so the real
    // `prepareOutgoing` refuses a product message on this webchat inbox.
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], json!("productMessageUnresolvable"), "{body}");
    assert_eq!(
        body["data"]["reason"],
        json!("channelUnsupported"),
        "{body}"
    );
    assert_eq!(stored_texts(&f, 70).await, Vec::<String>::new());
}

/// Adapted: an unknown product is what makes the product message
/// unsendable (the TS case mocks the service's error).
#[tokio::test]
async fn post_messages_lets_an_unsendable_product_message_surface_as_the_service_s_error() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.sendMessage",
        "id:123",
        json!({ "products": { "display": "products", "sections": [{ "productIds": ["999"] }] } }),
    )
    .await;
    assert!(status >= 400, "{status} {body}");
    assert_eq!(stored_texts(&f, 70).await, Vec::<String>::new());
}

#[tokio::test]
async fn post_messages_propagates_the_404_when_the_contact_has_no_matching_inbox() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.sendMessage",
        "id:124",
        json!({ "text": "hello" }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
    let (status, body) = post(
        &f,
        "contacts.sendMessage",
        "id:123",
        json!({ "text": "hello", "inboxId": "1" }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
    assert_eq!(stored_texts(&f, 70).await, Vec::<String>::new());
}

// ─── GET /v1/contacts/{identifier}/messages ─────────────────────────────

#[tokio::test]
async fn get_messages_lists_messages_for_the_contact_s_conversation() {
    let f = fixture().await;
    seed(&f).await;
    message(&f, 900, 50, "first", 1_000).await;
    message(&f, 901, 50, "second", 2_000).await;
    let (status, body) = call(
        &f,
        "contacts.listMessages",
        WS,
        &[("identifier", "id:123")],
        Some("perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let texts: Vec<&str> = body["data"]
        .as_array()
        .map(|rows| rows.iter().filter_map(|m| m["text"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(texts, vec!["second", "first"]);
    assert_eq!(body["nextCursor"], Value::Null);
    assert_eq!(body["prevCursor"], Value::Null);
}

#[tokio::test]
async fn get_messages_forwards_contactinboxid_to_narrow_the_list_to_one_channel() {
    let f = fixture().await;
    seed(&f).await;
    message(&f, 900, 50, "own", 1_000).await;
    let (status, own) = call(
        &f,
        "contacts.listMessages",
        WS,
        &[("identifier", "id:123")],
        Some("perPage=20&contactInboxId=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{own}");
    assert_eq!(own["data"].as_array().map(Vec::len), Some(1));
    let (status, other) = call(
        &f,
        "contacts.listMessages",
        WS,
        &[("identifier", "id:123")],
        Some("perPage=20&contactInboxId=51"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{other}");
    assert_eq!(other["data"], json!([]));
}

#[tokio::test]
async fn get_messages_404s_when_the_contact_has_no_conversation() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.listMessages",
        WS,
        &[("identifier", "id:124")],
        Some("perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
}

// ─── GET /v1/contacts/{identifier}/messages/{messageId} ─────────────────

#[tokio::test]
async fn get_message_returns_the_message_scoped_to_the_contact_s_conversation() {
    let f = fixture().await;
    seed(&f).await;
    message(&f, 900, 50, "first", 1_000).await;
    let (status, body) = call(
        &f,
        "contacts.getMessage",
        WS,
        &[("identifier", "id:123"), ("messageId", "900")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("900"));
    assert_eq!(body["conversationId"], json!("70"));
    assert_eq!(body["text"], json!("first"));
    assert_eq!(body["attachments"], json!([]));
}

#[tokio::test]
async fn get_message_404s_when_the_contact_has_no_conversation() {
    let f = fixture().await;
    seed(&f).await;
    message(&f, 900, 50, "first", 1_000).await;
    let (status, body) = call(
        &f,
        "contacts.getMessage",
        WS,
        &[("identifier", "id:124"), ("messageId", "900")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
}

// ─── POST /v1/contacts/{identifier}/auto-replies ────────────────────────

#[tokio::test]
async fn post_auto_replies_404s_when_no_automated_response_matches_the_keyword() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.triggerAutoReply",
        "id:124",
        json!({ "keyword": "hell" }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["message"],
        json!("No automated response found for this keyword")
    );
}

#[tokio::test]
async fn post_auto_replies_404s_when_the_contact_has_no_matching_inbox() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.triggerAutoReply",
        "id:124",
        json!({ "keyword": "hello" }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
}

#[tokio::test]
async fn post_auto_replies_sends_the_auto_reply_text_through_the_resolved_inbox() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.triggerAutoReply",
        "id:123",
        json!({ "keyword": "hello" }),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored_texts(&f, 70).await, vec!["Welcome!"]);
}

// ─── POST /v1/contacts/{identifier}/flows ───────────────────────────────

#[tokio::test]
async fn post_flows_404s_when_the_contact_has_no_matching_inbox() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.sendFlow",
        "id:124",
        json!({ "flowId": "300" }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
}

/// Adapted: the flow is queued for the worker, so no message is stored; an
/// unknown flow is a 404 (`Flow not found`).
#[tokio::test]
async fn post_flows_sends_the_flow_through_the_resolved_inbox() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = post(
        &f,
        "contacts.sendFlow",
        "id:123",
        json!({ "flowId": "300", "inboxId": "5" }),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored_texts(&f, 70).await, Vec::<String>::new());
    let (status, body) = post(&f, "contacts.sendFlow", "id:123", json!({ "flowId": "1" })).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Flow not found"));
}
