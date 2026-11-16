//! Port of `packages/business/__tests__/message-create-outgoing.test.ts` and
//! `message-list-for-conversation.test.ts` (P3.6-D3, slice contact
//! messages). Adapted: the mocked repository / queues / realtime become
//! real rows and the returned `Outgoing` (stored message, realtime payload,
//! jobs). Not applicable here (see the test map): the agent-reply metric,
//! buttons / quick replies, image limits, media files, the routing-bypass
//! flag, the WhatsApp product resolution and the attachment URL cases.

mod common;

use chatbotx_business::message::{
    ListForConversation, OutgoingInput, create_outgoing, find_for_contact, list_for_conversation,
    message_json,
};
use chatbotx_db::repo::message::PaginationCursor;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::QueueName;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

/// Contact 1 on webchat inbox 5 (contact inbox 50) and threads inbox 6
/// (contact inbox 60), conversation 70 (created at 2026-01-01); contact 2
/// on inbox 5 (contact inbox 51); flow 10 of `WS`, flow 20 of `OTHER_WS`.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::inbox(tx, 5, WS, "webchat")?;
        common::inbox(tx, 6, WS, "threads")?;
        common::contact(tx, 1, WS)?;
        common::contact(tx, 2, WS)?;
        common::contact_inbox(tx, 50, 1, 5, "webchat", "guest-1")?;
        common::contact_inbox(tx, 60, 1, 6, "threads", "th-1")?;
        common::contact_inbox(tx, 51, 2, 5, "webchat", "guest-2")?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId") VALUES (70, ?2, ?2, ?1, 1)"#,
            params![WS, JAN_1],
        )?;
        common::flow(tx, 10, WS)?;
        common::flow(tx, 20, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

/// 2026-01-01T00:00:00Z.
const JAN_1: i64 = 1_767_225_600_000;
/// 2026-06-10T15:42:00Z.
const JUNE_10_1542: i64 = 1_781_106_120_000;
const MINUTE: i64 = 60_000;

fn message(
    tx: &Connection,
    id: i64,
    contact_inbox_id: i64,
    text: &str,
    created_at: i64,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
             "messageType", "contentType", "senderType")
           VALUES (?1, ?2, ?2, 70, ?3, ?4, ?5, 'incoming', 'text', 'contact')"#,
        params![id, created_at, contact_inbox_id, WS, text],
    )?;
    Ok(())
}

async fn models(db: &common::TestDb, contact_inbox_id: i64) -> (Conversation, ContactInbox) {
    chatbotx_business::tx::read(db, move |conn| {
        let conversation = chatbotx_db::repo::find_one::<Conversation>(conn, r#""id" = 70"#, &[])?
            .expect("conversation");
        let contact_inbox = chatbotx_db::repo::find_one::<ContactInbox>(
            conn,
            r#""id" = ?1"#,
            &[&contact_inbox_id],
        )?
        .expect("contact inbox");
        Ok((conversation, contact_inbox))
    })
    .await
    .expect("models")
}

fn text(text: &str) -> OutgoingInput {
    OutgoingInput {
        text: Some(text.to_string()),
        ..OutgoingInput::default()
    }
}

async fn stored(db: &common::TestDb) -> Vec<serde_json::Map<String, Value>> {
    common::rows(
        db,
        r#"SELECT "id", "createdAt", "text", "senderType", "senderId", "type", "parentId", "contentAttributes", "contactInboxId"
           FROM "Message" WHERE "conversationId" = ?1 ORDER BY "id""#,
        70,
    )
    .await
}

// describe("messageService.createOutgoing")

#[tokio::test]
async fn updates_contact_inbox_last_message_at_from_the_created_message_timestamp() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(&db, &conversation, &contact_inbox, text("hello"), None)
        .await
        .expect("send");
    let created_at = outgoing
        .message
        .as_ref()
        .expect("message")
        .message
        .created_at;
    let row = common::rows(
        &db,
        r#"SELECT "firstInteractionAt", "lastMessageAt" FROM "ContactInbox" WHERE "id" = ?1"#,
        50,
    )
    .await;
    assert_eq!(row[0]["firstInteractionAt"], json!(created_at));
    assert_eq!(row[0]["lastMessageAt"], json!(created_at));
    let rows = stored(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["createdAt"], json!(created_at));
    assert_eq!(rows[0]["senderType"], json!("api"));
    assert_eq!(rows[0]["senderId"], Value::Null);
}

#[tokio::test]
async fn broadcasts_a_created_message_directly_instead_of_queueing_it() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            client_id: Some("client-1".into()),
            ..text("hello")
        },
        None,
    )
    .await
    .expect("send");
    let message = outgoing.message.expect("message");
    let realtime = outgoing.realtime.expect("realtime payload");
    assert_eq!(realtime["clientId"], json!("client-1"));
    assert_eq!(realtime["id"], json!(message.message.id.to_string()));
    assert_eq!(realtime["text"], json!("hello"));
    assert_eq!(realtime["messageType"], json!("outgoing"));
    assert_eq!(realtime["attachments"], json!([]));
}

/// Adapted: the job data carries the stored message; an inbox user is the
/// sender (`senderType` user), and a plain text send has no quick replies.
#[tokio::test]
async fn sends_no_buttons_for_a_plain_text_message() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        text("hello"),
        Some(1001),
    )
    .await
    .expect("send");
    let job = &outgoing.jobs[0];
    assert_eq!(job.queue, QueueName::Chat);
    assert_eq!(job.name, "sendChannelMessage");
    assert_eq!(job.data["type"], json!("sendChannelMessage"));
    assert_eq!(job.data["data"]["sendFrom"], json!("inbox"));
    assert!(job.data["data"].get("quickReplies").is_none());
    assert_eq!(job.data["data"]["message"]["text"], json!("hello"));
    assert_eq!(job.data["data"]["contactInbox"]["id"], json!("50"));
    assert_eq!(job.data["data"]["conversation"]["id"], json!("70"));
    assert_eq!(job.attempts, None);
    let rows = stored(&db).await;
    assert_eq!(rows[0]["senderType"], json!("user"));
    assert_eq!(rows[0]["senderId"], json!(1001));
    // An inbox user's text also runs the outbound keyword check.
    assert_eq!(
        outgoing.jobs.iter().map(|j| j.name).collect::<Vec<_>>(),
        vec!["sendChannelMessage", "checkOutboundAutomatedResponse"]
    );
}

#[tokio::test]
async fn uses_attempts_1_for_manual_threads_comment_replies() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 60).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            reply_to_message_id: Some("parent-1".into()),
            reply_to_message_created_at: Some(1_786_492_800_000),
            ..text("hello")
        },
        Some(1001),
    )
    .await
    .expect("send");
    let job = &outgoing.jobs[0];
    assert_eq!(job.name, "sendChannelMessage");
    assert_eq!(job.data["data"]["message"]["type"], json!("comment"));
    assert_eq!(
        job.data["data"]["message"]["parentCreatedAt"],
        json!("2026-08-12T00:00:00.000Z")
    );
    assert_eq!(job.attempts, Some(1));
    let rows = stored(&db).await;
    assert_eq!(rows[0]["type"], json!("comment"));
    assert_eq!(rows[0]["parentId"], json!("parent-1"));
}

#[tokio::test]
async fn keeps_default_queue_options_for_non_threads_manual_comment_replies() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            reply_to_message_id: Some("parent-1".into()),
            ..text("hello")
        },
        Some(1001),
    )
    .await
    .expect("send");
    assert_eq!(
        outgoing.jobs[0].data["data"]["message"]["type"],
        json!("comment")
    );
    assert_eq!(outgoing.jobs[0].attempts, None);
}

// describe("messageService.createOutgoing with a flow")

#[tokio::test]
async fn queues_the_flow_when_it_belongs_to_the_conversation_s_workspace() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            flow_id: Some(10),
            ..OutgoingInput::default()
        },
        None,
    )
    .await
    .expect("send flow");
    assert_eq!(outgoing.message, None);
    assert_eq!(outgoing.jobs.len(), 1);
    let job = &outgoing.jobs[0];
    assert_eq!(job.queue, QueueName::Integration);
    assert_eq!(job.name, "sendFlow");
    assert_eq!(job.data["type"], json!("sendFlow"));
    assert_eq!(job.data["data"]["flowId"], json!("10"));
    assert_eq!(job.data["data"]["sendFrom"], json!("inbox"));
    assert!(stored(&db).await.is_empty());
}

#[tokio::test]
async fn records_the_inbox_user_who_sent_the_flow_on_the_job() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            flow_id: Some(10),
            ..OutgoingInput::default()
        },
        Some(1001),
    )
    .await
    .expect("send flow");
    assert_eq!(outgoing.jobs[0].data["data"]["sentByUserId"], json!("1001"));
}

#[tokio::test]
async fn leaves_the_sender_off_the_job_when_no_user_is_attached_token_api() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let outgoing = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            flow_id: Some(10),
            ..OutgoingInput::default()
        },
        None,
    )
    .await
    .expect("send flow");
    assert!(outgoing.jobs[0].data["data"].get("sentByUserId").is_none());
}

#[tokio::test]
async fn rejects_a_flow_of_another_workspace_with_404_and_queues_nothing() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    for flow in [20, 999] {
        let error = create_outgoing(
            &db,
            &conversation,
            &contact_inbox,
            OutgoingInput {
                flow_id: Some(flow),
                ..OutgoingInput::default()
            },
            None,
        )
        .await
        .expect_err("foreign flow");
        assert_eq!(error.code(), Some("notFound"));
        assert_eq!(error.status(), Some(404));
        assert_eq!(error.message(), "Flow not found");
    }
}

// describe("messageService.createOutgoing with products")

/// Adapted: the real `prepareOutgoing` refuses every channel without a
/// catalog source (only WhatsApp has one) with `channelUnsupported`.
#[tokio::test]
async fn a_channel_unsupported_message_is_refused_with_its_reason_and_nothing_is_created() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    let error = create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        OutgoingInput {
            products: Some(json!({ "display": "products", "sections": [{ "productIds": ["1"] }] })),
            ..OutgoingInput::default()
        },
        Some(1001),
    )
    .await
    .expect_err("unsupported");
    assert_eq!(error.code(), Some("productMessageUnresolvable"));
    assert_eq!(error.status(), Some(422));
    assert_eq!(
        error.message(),
        "Product message cannot be sent: channelUnsupported"
    );
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { data: Some(d), .. } if d["reason"] == json!("channelUnsupported")
    ));
    assert!(stored(&db).await.is_empty());
}

#[tokio::test]
async fn refuses_products_as_a_comment_reply_before_resolving_anything_or_creating_a_conversation()
{
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    for input in [
        OutgoingInput {
            reply_to_message_id: Some("parent-1".into()),
            ..OutgoingInput::default()
        },
        OutgoingInput {
            is_private_reply: true,
            ..OutgoingInput::default()
        },
    ] {
        let error = create_outgoing(
            &db,
            &conversation,
            &contact_inbox,
            OutgoingInput {
                products: Some(json!({ "display": "products", "sections": [] })),
                ..input
            },
            Some(1001),
        )
        .await
        .expect_err("comment reply");
        assert_eq!(error.code(), Some("validation"));
        assert_eq!(error.message(), "Products can't be sent as a comment reply");
        assert!(matches!(
            &error,
            chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "products"
        ));
    }
    assert!(stored(&db).await.is_empty());
}

#[tokio::test]
async fn a_plain_text_send_never_touches_the_product_service() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 50).await;
    create_outgoing(
        &db,
        &conversation,
        &contact_inbox,
        text("hello"),
        Some(1001),
    )
    .await
    .expect("send");
    let rows = stored(&db).await;
    assert_eq!(rows[0]["contentAttributes"], Value::Null);
    assert_eq!(rows[0]["text"], json!("hello"));
}

// describe("message list-for-conversation") > describe("findForContact")

#[tokio::test]
async fn uses_conversation_scoped_lookup_without_requiring_created_at_from_the_caller() {
    let db = seeded().await;
    setup(&db, |tx| message(tx, 900, 50, "hi", JAN_1 + MINUTE)).await;
    let found = find_for_contact(&db, WS, 900, 70).await.expect("find");
    assert_eq!(found.message.id, 900);
    let json = message_json(&found);
    assert_eq!(json["id"], json!("900"));
    assert_eq!(json["conversationId"], json!("70"));
    assert_eq!(json["attachments"], json!([]));
    let error = find_for_contact(&db, OTHER_WS, 900, 70)
        .await
        .expect_err("other workspace");
    assert_eq!(error.message(), "Message not found");
}

#[tokio::test]
async fn throws_when_the_conversation_does_not_exist() {
    let db = seeded().await;
    setup(&db, |tx| message(tx, 900, 50, "hi", JAN_1 + MINUTE)).await;
    let error = find_for_contact(&db, WS, 900, 999)
        .await
        .expect_err("missing conversation");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Message not found");
    let error = find_for_contact(&db, WS, 901, 70)
        .await
        .expect_err("missing message");
    assert_eq!(error.message(), "Message not found");
}

// describe("listForConversation")

#[tokio::test]
async fn scopes_conversation_metadata_lookup_by_workspace_id() {
    let db = seeded().await;
    setup(&db, |tx| message(tx, 900, 50, "hi", JAN_1 + MINUTE)).await;
    let page = list_for_conversation(
        &db,
        ListForConversation {
            workspace_id: OTHER_WS,
            conversation_id: Some(70),
            limit: 20,
            ..ListForConversation::default()
        },
    )
    .await
    .expect("list");
    assert!(page.data.is_empty());
    assert_eq!(page.next_cursor, None);
}

/// Adapted: the seeded cursor is observed through which messages the
/// page shows: the end of the 15:00 hour of `lastMessageAt` 15:42.
#[tokio::test]
async fn seeds_the_cursor_from_the_contact_inbox_s_last_message_hour_when_none_is_given() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = ?1 WHERE "id" = 50"#,
            params![JUNE_10_1542],
        )?;
        message(tx, 900, 50, "before", JUNE_10_1542 - MINUTE)?;
        message(tx, 901, 50, "same hour", JUNE_10_1542 + 10 * MINUTE)?;
        message(tx, 902, 50, "next hour", JUNE_10_1542 + 30 * MINUTE)?;
        Ok(())
    })
    .await;
    let page = list_for_conversation(
        &db,
        ListForConversation {
            workspace_id: WS,
            conversation_id: Some(70),
            limit: 20,
            ..ListForConversation::default()
        },
    )
    .await
    .expect("list");
    let ids: Vec<i64> = page.data.iter().map(|m| m.message.id).collect();
    assert_eq!(ids, vec![901, 900]);
}

#[tokio::test]
async fn a_contact_inbox_id_narrows_the_repository_query_and_is_looked_up_within_the_conversation_s_contact()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        message(tx, 900, 50, "web", JAN_1 + MINUTE)?;
        message(tx, 901, 60, "threads", JAN_1 + 2 * MINUTE)?;
        message(tx, 902, 50, "web 2", JAN_1 + 3 * MINUTE)?;
        Ok(())
    })
    .await;
    let page = list_for_conversation(
        &db,
        ListForConversation {
            workspace_id: WS,
            conversation_id: Some(70),
            contact_inbox_id: Some(50),
            limit: 1,
            ..ListForConversation::default()
        },
    )
    .await
    .expect("list");
    assert_eq!(
        page.data.iter().map(|m| m.message.id).collect::<Vec<_>>(),
        vec![902]
    );
    let cursor = page.next_cursor.expect("next cursor");
    assert_eq!(cursor.id, Some(902));
    let next = list_for_conversation(
        &db,
        ListForConversation {
            workspace_id: WS,
            conversation_id: Some(70),
            contact_inbox_id: Some(50),
            cursor: Some(PaginationCursor {
                created_at: cursor.created_at,
                id: cursor.id,
                shard_id: None,
            }),
            limit: 1,
        },
    )
    .await
    .expect("list");
    assert_eq!(
        next.data.iter().map(|m| m.message.id).collect::<Vec<_>>(),
        vec![900]
    );
    assert_eq!(next.next_cursor, None);
}

#[tokio::test]
async fn a_contact_inbox_id_of_another_contact_still_filters_the_query_empty_result() {
    let db = seeded().await;
    setup(&db, |tx| message(tx, 900, 50, "web", JAN_1 + MINUTE)).await;
    let page = list_for_conversation(
        &db,
        ListForConversation {
            workspace_id: WS,
            conversation_id: Some(70),
            contact_inbox_id: Some(51),
            limit: 20,
            ..ListForConversation::default()
        },
    )
    .await
    .expect("list");
    assert!(page.data.is_empty());
    assert_eq!(page.next_cursor, None);
}
