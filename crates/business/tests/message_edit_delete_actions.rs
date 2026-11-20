//! Port of `apps/builder/__tests__/edit-message-action.test.ts` and
//! `delete-message-action.test.ts` (Q.P6-U1). Adapted: the mocked services
//! become real rows; the realtime event and the chat job are the returned
//! [`Done`].

mod common;

use chatbotx_business::message_actions::{delete_message, edit_message};
use chatbotx_db::repo::models::Conversation;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_rt::protocol::{EventType, Route};
use common::{WS, setup, two_workspaces};
use serde_json::json;

/// 2026-01-01T00:00:00Z.
const AT: i64 = 1_767_225_600_000;

fn message(
    tx: &Connection,
    id: i64,
    conversation: i64,
    kind: &str,
    r#type: &str,
    source_id: Option<&str>,
    parent_id: Option<i64>,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
             "messageType", "contentType", "senderType", "type", "sourceId", "parentId")
           VALUES (?1, ?2, ?2, ?3, 50, ?4, 'old text', ?5, 'text', 'user', ?6, ?7, ?8)"#,
        params![id, AT, conversation, WS, kind, r#type, source_id, parent_id],
    )?;
    Ok(())
}

/// Conversation 70 of contact 1 (contact inbox 50 on messenger inbox 5) and
/// an empty conversation 71 of contact 2.
async fn seeded(
    seed: impl FnOnce(&Connection) -> Result<(), chatbotx_db::DbError> + Send + 'static,
) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        common::inbox(tx, 5, WS, "messenger")?;
        common::contact(tx, 1, WS)?;
        common::contact(tx, 2, WS)?;
        common::contact_inbox(tx, 50, 1, 5, "messenger", "psid-1")?;
        // One DM conversation per contact.
        for (id, contact) in [(70, 1), (71, 2)] {
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId") VALUES (?1, ?2, ?2, ?3, ?4)"#,
                params![id, AT, WS, contact],
            )?;
        }
        seed(tx)
    })
    .await;
    db
}

async fn conversation(db: &common::TestDb, id: i64) -> Conversation {
    chatbotx_business::tx::read(db, move |conn| {
        Ok(chatbotx_db::repo::find_one::<Conversation>(
            conn,
            r#""id" = ?1"#,
            &[&id],
        )?)
    })
    .await
    .expect("read")
    .expect("conversation")
}

fn empty_route() -> Option<Route> {
    Some(Route {
        assigned_team_ids: Vec::new(),
        assigned_user_ids: Vec::new(),
        inbox_id: None,
    })
}

// ---- editMessage ------------------------------------------------------------

#[tokio::test]
async fn broadcasts_messageupdated_with_the_new_text_and_no_attachment_change() {
    let db = seeded(|tx| message(tx, 1, 70, "outgoing", "comment", None, None)).await;
    let conv = conversation(&db, 70).await;
    let (_, done) = edit_message(&db, None, &conv, 1, AT, "updated text".into(), None, false)
        .await
        .expect("edit");
    let event = done.event.expect("event");
    assert_eq!(event.event_type, EventType::MessageUpdated);
    assert_eq!(
        event.data,
        json!({
            "messageId": "1",
            "newText": "updated text",
            "newAttachmentPath": null,
            "newAttachmentPublicUrl": null,
            "newAttachmentMimeType": null,
            "newAttachmentWidth": 0,
            "newAttachmentHeight": 0,
            "removedAttachment": false,
        })
    );
    assert_eq!(event.route, empty_route());
    let text: String = chatbotx_business::tx::read(&db, |conn| {
        Ok(conn
            .query_row(r#"SELECT "text" FROM "Message" WHERE "id" = 1"#, [], |r| {
                r.get(0)
            })
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .expect("text");
    assert_eq!(text, "updated text");
    assert_eq!(done.job.expect("job").name, "editChannelMessage");
}

#[tokio::test]
async fn broadcasts_removedattachment_true_when_the_caller_removes_the_attachment() {
    let db = seeded(|tx| message(tx, 1, 70, "outgoing", "comment", None, None)).await;
    let conv = conversation(&db, 70).await;
    let (_, done) = edit_message(&db, None, &conv, 1, AT, "updated text".into(), None, true)
        .await
        .expect("edit");
    let event = done.event.expect("event");
    assert_eq!(event.event_type, EventType::MessageUpdated);
    assert_eq!(event.data["removedAttachment"], json!(true));
}

#[tokio::test]
async fn throws_and_does_not_broadcast_when_the_message_is_not_an_editable_comment() {
    let db = seeded(|tx| message(tx, 1, 70, "incoming", "comment", None, None)).await;
    let conv = conversation(&db, 70).await;
    let error = edit_message(&db, None, &conv, 1, AT, "updated text".into(), None, false)
        .await
        .expect_err("not editable");
    assert!(
        error
            .to_string()
            .contains("Message is not an editable comment"),
        "{error}"
    );
}

// ---- deleteMessage ----------------------------------------------------------

#[tokio::test]
async fn broadcasts_messagedeleted_with_the_deleted_message_ids() {
    let db = seeded(|tx| message(tx, 1, 70, "outgoing", "message", None, None)).await;
    let conv = conversation(&db, 70).await;
    let done = delete_message(&db, &conv, 1, AT).await.expect("delete");
    let event = done.event.expect("event");
    assert_eq!(event.event_type, EventType::MessageDeleted);
    assert_eq!(event.data, json!({ "messageIds": ["1"] }));
    assert_eq!(event.route, empty_route());
    assert!(done.job.is_none());
}

#[tokio::test]
async fn queues_a_channel_deletion_job_when_the_message_has_a_sourceid() {
    let db = seeded(|tx| {
        message(tx, 1, 70, "outgoing", "comment", Some("source-1"), None)?;
        message(tx, 2, 70, "outgoing", "comment", Some("source-2"), Some(1))
    })
    .await;
    let conv = conversation(&db, 70).await;
    let done = delete_message(&db, &conv, 1, AT).await.expect("delete");
    let event = done.event.expect("event");
    let mut ids: Vec<String> = serde_json::from_value(event.data["messageIds"].clone()).unwrap();
    ids.sort();
    assert_eq!(ids, ["1", "2"]);
    let job = done.job.expect("job");
    assert_eq!(job.name, "deleteChannelMessage");
    assert_eq!(
        job.data["data"]["message"],
        json!({ "id": "1", "createdAt": "2026-01-01T00:00:00.000Z" })
    );
}

#[tokio::test]
async fn throws_when_the_message_does_not_belong_to_the_conversation() {
    let db = seeded(|tx| message(tx, 1, 70, "outgoing", "message", None, None)).await;
    let other = conversation(&db, 71).await;
    let error = delete_message(&db, &other, 1, AT)
        .await
        .expect_err("not found");
    assert!(error.to_string().contains("Comment not found"), "{error}");
}
