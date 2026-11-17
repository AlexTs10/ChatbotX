//! `apps/worker/__tests__/send-message-handler.test.ts` on the worker
//! fixtures (Q.W3): `chat::send_message_to_channel`. Mocked service calls
//! become rows (`Message`, `Conversation`, `Contact`, `ContactState`) and the
//! calls of a `RecordingChannel`; the analytics / `message:failed` event
//! cases wait for the event workers (Q.W2).

use std::sync::Arc;

use serde_json::{Value, json};

use super::test_support::*;
use crate::channel::{Category, ChannelError, RecordingChannel, SendFailure, SendResult};
use crate::chat::send_message_to_channel;

/// Message 900 (`type`, `contentAttributes`) of conversation 6.
async fn message(fx: &Fixture, kind: &'static str, attributes: Value) -> Value {
    let text = attributes.to_string();
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Message" ("id", "conversationId", "contactInboxId", "workspaceId", "text", "contentAttributes",
                 "messageType", "contentType", "senderType", "type", "createdAt", "updatedAt")
               VALUES (900, 6, 5, 2, 'hello', ?1, 'outgoing', 'text', 'user', ?2, 1000, 1000)"#,
            chatbotx_db::rusqlite::params![text, kind],
        )?;
        Ok(())
    })
    .await;
    json!({ "id": "900", "createdAt": 1000, "type": kind, "text": "hello", "contentAttributes": attributes })
}

async fn send(
    fx: &Fixture,
    message: Value,
    extra: Value,
    attempts_made: u32,
) -> Result<SendResult, chatbotx_jobs::JobError> {
    let mut data = json!({ "conversation": "6", "contactInbox": "5", "message": message });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        data[k] = v;
    }
    send_message_to_channel(&fx.deps, &data, attempts_made, true).await
}

async fn on(channel: &'static str) -> (Fixture, Arc<RecordingChannel>) {
    let mut fx = Fixture::new().await;
    let recording = fx.on_channel(channel).await;
    (fx, recording)
}

async fn last_delivered(fx: &Fixture) -> Option<i64> {
    fx.int(r#"SELECT "lastDeliveredAt" FROM "ContactState" WHERE "contactId" = 3"#)
        .await
}

async fn read_at(fx: &Fixture) -> Option<i64> {
    fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
        .await
}

/// `describe("last delivered")`.
mod last_delivered {
    use super::*;

    #[tokio::test]
    async fn records_a_successful_send_as_delivered_on_a_channel_without_receipts() {
        let (fx, _channel) = on("messenger").await;
        let msg = message(&fx, "message", json!({})).await;
        send(&fx, msg, json!({}), 0).await.unwrap();
        assert!(last_delivered(&fx).await.is_some());
    }

    #[tokio::test]
    async fn leaves_a_channel_that_reports_receipts_to_the_delivered_listener() {
        let (mut fx, _channel) = on("messenger").await;
        let mut receipts = RecordingChannel::default();
        receipts.receipts = true;
        fx.deps.channels = crate::Channels::default().with("messenger", Arc::new(receipts));
        let msg = message(&fx, "message", json!({})).await;
        send(&fx, msg, json!({}), 0).await.unwrap();
        assert_eq!(last_delivered(&fx).await, None);
    }

    #[tokio::test]
    async fn records_nothing_for_an_unregistered_channel_or_a_failed_send() {
        let (mut fx, channel) = on("messenger").await;
        let msg = message(&fx, "message", json!({})).await;
        channel.fail_next(SendFailure::Channel(ChannelError::new(
            "bad",
            Category::PayloadInvalid,
        )));
        send(&fx, msg.clone(), json!({}), 0).await.unwrap();
        assert_eq!(last_delivered(&fx).await, None);
        fx.deps.channels = crate::Channels::default();
        assert!(send(&fx, msg, json!({}), 0).await.is_err());
        assert_eq!(last_delivered(&fx).await, None);
    }

    #[tokio::test]
    async fn does_not_record_a_public_comment_reply_but_records_a_private_reply() {
        let (fx, _channel) = on("messenger").await;
        let msg = message(&fx, "comment", json!({})).await;
        send(&fx, msg, json!({}), 0).await.unwrap();
        assert_eq!(last_delivered(&fx).await, None);

        let (fx, _channel) = on("messenger").await;
        let msg = message(&fx, "comment", json!({ "isPrivateReply": true })).await;
        send(&fx, msg, json!({}), 0).await.unwrap();
        assert!(last_delivered(&fx).await.is_some());
    }
}

#[tokio::test]
async fn passes_send_from_to_the_channel_and_persists_the_provider_id_as_source_id() {
    let (fx, channel) = on("messenger").await;
    let msg = message(&fx, "message", json!({})).await;
    send(&fx, msg, json!({ "sendFrom": "inbox" }), 0)
        .await
        .unwrap();
    let call = &channel.calls()[0];
    assert_eq!(
        (call.handler, call.data["sendFrom"].clone()),
        ("sendMessage", json!("inbox"))
    );
    assert_eq!(
        fx.text(r#"SELECT "sourceId" FROM "Message" WHERE "id" = 900"#)
            .await
            .as_deref(),
        Some("mid-1")
    );
}

#[tokio::test]
async fn marks_direct_and_broadcast_sends_read_but_not_a_public_comment_reply() {
    let broadcast = json!({ "type": "broadcast", "broadcastId": "b", "contactInboxId": "5" });
    for (kind, attributes, extra, marked) in [
        ("message", json!({}), json!({}), true),
        (
            "message",
            json!({}),
            json!({ "metadata": broadcast, "isBulkBroadcast": true }),
            true,
        ),
        ("comment", json!({}), json!({}), false),
        (
            "comment",
            json!({ "isPrivateReply": true }),
            json!({}),
            true,
        ),
    ] {
        let (fx, _channel) = on("messenger").await;
        let msg = message(&fx, kind, attributes.clone()).await;
        send(&fx, msg, extra, 0).await.unwrap();
        assert_eq!(read_at(&fx).await.is_some(), marked, "{kind} {attributes}");
    }
}

#[tokio::test]
async fn auto_unblocks_after_a_successful_non_comment_send_only() {
    for (kind, unblocked) in [("message", true), ("comment", false)] {
        let (fx, _channel) = on("messenger").await;
        fx.exec(r#"UPDATE "Contact" SET "blockedAt" = 5 WHERE "id" = 3;"#)
            .await;
        let msg = message(&fx, kind, json!({})).await;
        send(&fx, msg, json!({}), 0).await.unwrap();
        let blocked = fx
            .int(r#"SELECT "blockedAt" FROM "Contact" WHERE "id" = 3"#)
            .await;
        assert_eq!(blocked.is_none(), unblocked, "{kind}");
    }
}

#[tokio::test]
async fn a_failed_send_persists_the_error_and_a_retry_success_clears_it() {
    let (fx, channel) = on("messenger").await;
    let msg = message(&fx, "message", json!({})).await;
    channel.fail_next(SendFailure::Channel(ChannelError::new(
        "bad payload",
        Category::PayloadInvalid,
    )));
    // A non-retryable ChannelError is recorded, never thrown.
    let result = send(&fx, msg.clone(), json!({}), 0).await.unwrap();
    assert_eq!(result.sent_count, 0);
    let error = || fx.text(r#"SELECT "sendError" FROM "Message" WHERE "id" = 900"#);
    assert_eq!(error().await.as_deref(), Some("bad payload"));
    assert_eq!(read_at(&fx).await, None);
    // A first-attempt success leaves it; a retry (attemptsMade > 0) clears it.
    send(&fx, msg.clone(), json!({}), 0).await.unwrap();
    assert_eq!(error().await.as_deref(), Some("bad payload"));
    send(&fx, msg, json!({}), 1).await.unwrap();
    assert_eq!(error().await, None);
}

#[tokio::test]
async fn retryable_channel_errors_retry_only_outside_messenger_and_instagram() {
    for (channel_name, category, retried) in [
        ("messenger", Category::NetworkError, false),
        ("instagram", Category::NetworkError, false),
        ("whatsapp", Category::RateLimited, true),
    ] {
        let (fx, channel) = on(channel_name).await;
        let msg = message(&fx, "message", json!({})).await;
        channel.fail_next(SendFailure::Channel(ChannelError::new(
            "network error",
            category,
        )));
        let result = send(&fx, msg, json!({}), 0).await;
        assert_eq!(result.is_err(), retried, "{channel_name}");
        if let Ok(result) = result {
            assert_eq!(result.sent_count, 0);
        }
        assert_eq!(read_at(&fx).await, None);
    }
}

#[tokio::test]
async fn does_not_retry_a_missing_integration_auth() {
    let (fx, channel) = on("messenger").await;
    fx.exec(r#"DELETE FROM "IntegrationMessenger";"#).await;
    let msg = message(&fx, "message", json!({})).await;
    let result = send(&fx, msg, json!({}), 0).await.unwrap();
    assert_eq!(result.sent_count, 0);
    assert!(channel.calls().is_empty());
}
