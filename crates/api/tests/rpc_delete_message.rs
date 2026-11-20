//! `deleteMessage` / `deleteMessageAction` through `/rpc/actions/*` (Q.P7-V5):
//! the answer `{ success, messageIds }` of
//! `apps/builder/src/features/messages/actions/delete-message.action.ts`
//! (the `messageIds` of `delete-message-action.test.ts`: a comment with a
//! `sourceId` deletes its children too).

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::params;
use rpc_common::{OWNER_TOKEN, WS, action};
use serde_json::{Value, json};

const AT: i64 = 1_767_225_600_000;

async fn seed(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (5, ?1, 'Web', 'webchat', 'web-5')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (123, ?1, 'Ada')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (50, 123, 123, 5, 'webchat', 'direct', 'guest-123')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "createdAt", "workspaceId", "contactId") VALUES (70, 0, ?1, 123)"#,
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

async fn run(fx: &rpc_common::Fixture, name: &str, args: Value) -> (StatusCode, Value) {
    action(
        fx,
        "actions",
        name,
        OWNER_TOKEN,
        json!({ "args": args, "workspaceId": WS.to_string() }),
    )
    .await
}

#[tokio::test]
async fn delete_message_action_answers_the_deleted_ids() {
    let fx = rpc_common::fixture().await;
    seed(&fx).await;
    let ws = WS.to_string();
    let (status, out) = run(
        &fx,
        "deleteMessageAction",
        json!([ws, "70", { "id": "900", "createdAt": iso(AT) }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["json"],
        json!({ "success": true, "messageIds": ["900"] }),
        "{out}"
    );

    let (status, out) = run(
        &fx,
        "deleteMessage",
        json!([{ "workspaceId": ws, "conversationId": "70",
                 "parsedInput": { "id": "901", "createdAt": iso(AT) } }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["success"], json!(true), "{out}");
    assert_eq!(out["json"]["messageIds"], json!(["901"]), "{out}");

    let (status, out) = run(
        &fx,
        "deleteMessageAction",
        json!([ws, "70", { "id": "999", "createdAt": iso(AT) }]),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{out}");
    assert!(out.to_string().contains("Comment not found"), "{out}");
}
