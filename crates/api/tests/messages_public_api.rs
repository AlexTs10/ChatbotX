//! Port of `apps/builder/__tests__/messages-public-api.test.ts` (P3.6-D3,
//! slice messages). Adapted: the captured oRPC handlers and the mocked
//! `conversationService` / `messageService` / `listMessages` / `editMessage`
//! / `deleteMessage` / `changeMessageAttributes` become `handlers::dispatch`
//! calls over real rows; "X was called with Y" becomes the stored row or the
//! response. The Flow response view, WhatsApp templates and call permission
//! are not ported (501 `notPorted`).

mod handlers_common;

use chatbotx_api::routes::{PUBLIC_ROUTES, RouteAuth};
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

const AT: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

/// Contact 123 with contact inbox 50 (webchat inbox 5) and 52 (whatsapp
/// inbox 6), conversation 70; contact 124 with conversation 71. Message 900
/// (chat, conversation 70) and 901 (outgoing comment, conversation 70).
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId")
                   VALUES (5, ?1, 'Web', 'webchat', 'web-5'), (6, ?1, 'Wa', 'whatsapp', 'wa-6')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (123, ?1, 'Ada'), (124, ?1, 'Bo')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastMessageAt")
                   VALUES (50, 123, 123, 5, 'webchat', 'direct', 'guest-123', 1767225600000),
                          (52, 123, 123, 6, 'whatsapp', 'direct', 'wa-123', 1767225500000),
                          (51, 124, 124, 5, 'webchat', 'direct', 'guest-124', 1767225600000)"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "createdAt", "workspaceId", "contactId") VALUES (70, 0, ?1, 123), (71, 0, ?1, 124)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
                     "messageType", "contentType", "senderType", "type", "sourceId")
                   VALUES (900, ?1, ?1, 70, 50, ?2, 'Chat', 'outgoing', 'text', 'api', 'message', NULL),
                          (901, ?1, ?1, 70, 50, ?2, 'Comment', 'outgoing', 'text', 'user', 'comment', 'fb-901')"#,
                params![AT, WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

fn at_query() -> String {
    format!("createdAt={}", iso(AT))
}

async fn on_message(
    f: &Fixture,
    route: &str,
    conversation: &'static str,
    message: &'static str,
    body: Value,
) -> (u16, Value) {
    call(
        f,
        route,
        WS,
        &[("conversationId", conversation), ("messageId", message)],
        None,
        Some(body),
    )
    .await
}

#[test]
fn registers_the_messages_public_router_under_the_inbox_scope() {
    let routes: Vec<_> = PUBLIC_ROUTES
        .iter()
        .filter(|r| r.id.starts_with("messages."))
        .collect();
    assert_eq!(routes.len(), 9);
    for route in routes {
        assert!(
            matches!(route.auth, RouteAuth::WorkspaceToken(scopes) if scopes == ["inbox"]),
            "{}",
            route.id
        );
    }
}

// ─── GET /v1/conversations/{conversationId}/messages ────────────────────

#[tokio::test]
async fn get_messages_validates_the_conversation_exists_then_delegates_to_listmessages() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "messages.list",
        WS,
        &[("conversationId", "70")],
        Some("perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|m| m["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids.len(), 2, "{body}");
    assert!(ids.contains(&"900") && ids.contains(&"901"));
    assert_eq!(body["nextCursor"], Value::Null);
    assert_eq!(body["prevCursor"], Value::Null);
}

#[tokio::test]
async fn get_messages_propagates_the_404_when_the_conversation_does_not_exist() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "messages.list",
        WS,
        &[("conversationId", "999")],
        Some("perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Conversation not found"));
}

// ─── GET /v1/conversations/{conversationId}/messages/{messageId} ────────

#[tokio::test]
async fn get_message_delegates_to_messageservice_findbyidwithurls() {
    let f = fixture().await;
    seed(&f).await;
    let q = at_query();
    let (status, body) = call(
        &f,
        "messages.get",
        WS,
        &[("conversationId", "70"), ("messageId", "900")],
        Some(&q),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("900"));
    assert_eq!(body["conversationId"], json!("70"));
}

#[tokio::test]
async fn get_message_404s_when_the_message_belongs_to_a_different_conversation() {
    let f = fixture().await;
    seed(&f).await;
    let q = at_query();
    let (status, body) = call(
        &f,
        "messages.get",
        WS,
        &[("conversationId", "71"), ("messageId", "900")],
        Some(&q),
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Message not found"));
}

// ─── GET …/messages/{messageId}/flow-response ───────────────────────────

/// Adapted: the Flow response view (labels from the Meta Flow JSON) is not
/// ported; a Flow response answers 501, any other message 404.
#[tokio::test]
async fn get_flow_response_labels_the_response_through_the_shared_lib_scoped_to_the_token_s_workspace()
 {
    let f = fixture().await;
    seed(&f).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Message" SET "contentAttributes" = '{"type":"whatsapp_flow_response","flowResponse":{"name":"An"}}' WHERE "id" = 900"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("flow response");
    let q = at_query();
    let mut answers = Vec::new();
    for message in ["900", "901"] {
        answers.push(
            call(
                &f,
                "messages.getFlowResponse",
                WS,
                &[("conversationId", "70"), ("messageId", message)],
                Some(&q),
                None,
            )
            .await,
        );
    }
    let (status, body) = &answers[0];
    assert_eq!(*status, 501, "{body}");
    assert_eq!(body["code"], json!("notPorted"));
    let (status, body) = &answers[1];
    assert_eq!(*status, 404, "{body}");
    assert_eq!(body["message"], json!("WhatsApp Flow response not found"));
}

// ─── POST /v1/conversations/{conversationId}/messages ───────────────────

#[tokio::test]
async fn post_messages_sends_without_a_user_a_workspace_token_has_no_user() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "messages.create",
        WS,
        &[("conversationId", "70")],
        None,
        Some(json!({ "text": "hello" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["text"], json!("hello"));
    // `resolveContactInboxForConversation` without inboxId: the most recent.
    assert_eq!(body["contactInboxId"], json!("50"));
    assert_eq!(body["senderType"], json!("api"));
    assert_eq!(body["senderId"], Value::Null);
}

/// Adapted: the product message reaches `createOutgoing` on the requested
/// (WhatsApp) contact inbox, where the Meta catalog step answers 501
/// `notPorted` (the webchat inbox would answer 422 channelUnsupported).
#[tokio::test]
async fn post_messages_hands_a_product_message_to_createoutgoing_unchanged_on_the_requested_inbox()
{
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "messages.create",
        WS,
        &[("conversationId", "70")],
        None,
        Some(json!({
            "inboxId": "6",
            "products": { "display": "products", "body": "Look", "sections": [{ "productIds": ["1", "2"] }] },
        })),
    )
    .await;
    assert_eq!(status, 501, "{body}");
    assert_eq!(body["code"], json!("notPorted"));
}

#[tokio::test]
async fn post_messages_lets_an_unsendable_product_message_surface_as_the_service_s_error() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "messages.create",
        WS,
        &[("conversationId", "70")],
        None,
        Some(
            json!({ "products": { "display": "products", "sections": [{ "productIds": ["1"] }] } }),
        ),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], json!("productMessageUnresolvable"));
    assert_eq!(body["data"], json!({ "reason": "channelUnsupported" }));
}

// ─── PUT …/messages/{messageId} ─────────────────────────────────────────

#[tokio::test]
async fn put_message_delegates_to_editmessage() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = on_message(
        &f,
        "messages.update",
        "70",
        "901",
        json!({ "createdAt": iso(AT), "newText": "edited" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["messageId"], json!("901"));
    assert_eq!(body["newText"], json!("edited"));
    let text = scalar(&f, r#"SELECT "text" FROM "Message" WHERE "id" = ?1"#, 901).await;
    assert_eq!(text, Some(json!("edited")));
    // A chat message is not an editable comment.
    let (status, body) = on_message(
        &f,
        "messages.update",
        "70",
        "900",
        json!({ "createdAt": iso(AT), "newText": "edited" }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], json!("Message is not an editable comment"));
}

// ─── DELETE …/messages/{messageId} ──────────────────────────────────────

#[tokio::test]
async fn delete_message_delegates_to_deletemessage_with_the_id_field_it_expects() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = on_message(
        &f,
        "messages.delete",
        "70",
        "900",
        json!({ "createdAt": iso(AT) }),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    let deleted = scalar(
        &f,
        r#"SELECT "deletedAt" FROM "Message" WHERE "id" = ?1"#,
        900,
    )
    .await;
    assert!(deleted.is_some_and(|v| v.is_i64()));
    let (status, body) = on_message(
        &f,
        "messages.delete",
        "70",
        "999",
        json!({ "createdAt": iso(AT) }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], json!("Comment not found"));
}

// ─── POST …/messages/{messageId}/attributes ─────────────────────────────

#[tokio::test]
async fn post_attributes_delegates_to_changemessageattributes() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = on_message(
        &f,
        "messages.changeAttributes",
        "70",
        "901",
        json!({ "createdAt": iso(AT), "liked": true }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true, "messageId": "901" }));
}

// ─── whatsapp-template / whatsapp-call-permission ──────────────────────

/// Adapted: the four `whatsapp-template` cases and the call-permission case
/// need the WhatsApp template / calling ports; both routes answer 501.
#[tokio::test]
async fn whatsapp_template_and_call_permission_are_not_ported() {
    let f = fixture().await;
    seed(&f).await;
    for route in [
        "messages.sendWhatsappTemplate",
        "messages.requestWhatsappCallPermission",
    ] {
        let (status, body) = call(
            &f,
            route,
            WS,
            &[("conversationId", "70")],
            None,
            Some(json!({ "templateId": "5", "text": "May we call you?" })),
        )
        .await;
        assert_eq!(status, 501, "{route} {body}");
    }
}
