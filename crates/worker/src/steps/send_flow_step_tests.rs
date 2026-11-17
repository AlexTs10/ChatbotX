//! `apps/worker/__tests__/send-flow-step.test.ts` on the worker fixtures
//! (Q.W3): the chat-queue `sendFlowStep` / `sendChatMessage` of
//! `send_flow_step.rs`. The mocked repository / service calls become the
//! stored `Message` and `Conversation` / `ContactInbox` rows and the calls
//! of a `RecordingChannel`. The cases of features that are not in Rust yet
//! are n-a (specs/Q.W3.md test map).

use serde_json::{Value, json};

use super::test_support::*;
use crate::channel::RecordingChannel;
use crate::send_flow_step::{send_chat_message, send_flow_step};

fn job(step: Value, extra: Value) -> Value {
    let mut data = json!({
        "conversationId": "6",
        "contactInboxId": "5",
        "flowId": "10",
        "flowVersionId": "11",
        "step": step,
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        data[k] = v;
    }
    data
}

fn text_step(text: &str) -> Value {
    json!({ "id": "s-1", "nodeId": "n1", "stepType": "sendText", "text": text, "buttons": [] })
}

/// `(text, type, contentAttributes, contactInboxId, sendError)` of every message.
async fn messages(
    fx: &Fixture,
) -> Vec<(Option<String>, Option<String>, Value, i64, Option<String>)> {
    fx.deps
        .db
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "text", "type", "contentAttributes", "contactInboxId", "sendError" FROM "Message" ORDER BY "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| {
                    let attributes: Option<String> = r.get(2)?;
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        attributes
                            .and_then(|a| serde_json::from_str(&a).ok())
                            .unwrap_or(Value::Null),
                        r.get(3)?,
                        r.get(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

async fn send(fx: &Fixture, data: Value) {
    send_flow_step(&fx.deps, &data).await.unwrap();
}

async fn messenger() -> (Fixture, std::sync::Arc<RecordingChannel>) {
    let mut fx = Fixture::new().await;
    let channel = fx.on_channel("messenger").await;
    (fx, channel)
}

#[tokio::test]
async fn returns_early_when_the_conversation_or_the_contact_inbox_is_not_found() {
    let (fx, channel) = messenger().await;
    send(
        &fx,
        job(text_step("hi"), json!({ "conversationId": "999" })),
    )
    .await;
    send(
        &fx,
        job(text_step("hi"), json!({ "contactInboxId": "999" })),
    )
    .await;
    assert!(messages(&fx).await.is_empty());
    assert!(channel.calls().is_empty());
}

#[tokio::test]
async fn opts_into_spintax_when_resolving_a_deliverable_step() {
    let (fx, channel) = messenger().await;
    send(&fx, job(text_step("{Hello|Hello} Ada"), json!({}))).await;
    assert_eq!(messages(&fx).await[0].0.as_deref(), Some("Hello Ada"));
    assert_eq!(channel.calls()[0].data["step"]["text"], "Hello Ada");
}

#[tokio::test]
async fn skips_non_deliverable_blank_and_channel_unsupported_steps() {
    // A non-deliverable AI step, a blank text and an omnichannel card on
    // Telegram (the channel policy marks it unsupported).
    let (fx, channel) = messenger().await;
    send(
        &fx,
        job(
            json!({ "id": "a", "stepType": "aiGenerateText", "prompt": "x" }),
            json!({}),
        ),
    )
    .await;
    send(&fx, job(text_step("   "), json!({}))).await;
    assert!(messages(&fx).await.is_empty());
    assert!(channel.calls().is_empty());

    let mut fx = Fixture::new().await;
    let channel = fx.on_channel("telegram").await;
    send(
        &fx,
        job(
            json!({ "id": "c", "stepType": "sendCard", "title": "T", "buttons": [] }),
            json!({}),
        ),
    )
    .await;
    assert!(messages(&fx).await.is_empty());
    assert!(channel.calls().is_empty());
}

#[tokio::test]
async fn stores_a_location_message_with_its_coordinates_on_whatsapp_and_skips_it_elsewhere() {
    let location = json!({ "id": "l", "nodeId": "n1", "stepType": "whatsappLocation", "latitude": 52.52, "longitude": 13.405, "name": "Office" });
    let mut fx = Fixture::new().await;
    let channel = fx.on_channel("whatsapp").await;
    send(&fx, job(location.clone(), json!({}))).await;
    let stored = messages(&fx).await;
    assert_eq!(stored[0].2["latitude"], "52.52");
    assert_eq!(stored[0].2["longitude"], "13.405");
    assert_eq!(channel.calls()[0].handler, "sendFlowStep");

    let (fx, channel) = messenger().await;
    send(&fx, job(location, json!({}))).await;
    assert!(messages(&fx).await.is_empty());
    assert!(channel.calls().is_empty());
}

#[tokio::test]
async fn stores_an_outgoing_bot_message_and_sends_the_step_through_the_channel() {
    let (fx, channel) = messenger().await;
    send(
        &fx,
        job(text_step("hello from flow"), json!({ "sendFrom": "inbox" })),
    )
    .await;
    let (outgoing, sender): (Option<String>, Option<String>) = fx
        .deps
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "messageType", "senderType" FROM "Message""#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        (outgoing.as_deref(), sender.as_deref()),
        (Some("outgoing"), Some("bot"))
    );
    let stored = messages(&fx).await;
    assert_eq!(stored[0].2["stepId"], "s-1");
    assert_eq!(stored[0].2["flowId"], "10");
    let call = &channel.calls()[0];
    assert_eq!(call.handler, "sendFlowStep");
    assert_eq!(call.data["step"]["text"], "hello from flow");
    assert_eq!(call.data["flowId"], "10");
}

#[tokio::test]
async fn uses_the_job_contact_inbox_instead_of_the_latest_inbox() {
    let (fx, _channel) = messenger().await;
    fx.exec(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastMessageAt")
           VALUES (7, 3, 3, 4, 'messenger', 'messenger', 's7', 99999999999999);"#,
    )
    .await;
    send(&fx, job(text_step("one"), json!({}))).await;
    let mut without = job(text_step("two"), json!({}));
    without.as_object_mut().unwrap().remove("contactInboxId");
    send(&fx, without).await;
    let inboxes: Vec<i64> = messages(&fx).await.iter().map(|m| m.3).collect();
    assert_eq!(inboxes, vec![5, 7]);
}

/// `awaitingHumanReplySince` and `lastActivityAt` after one send with `extra`.
async fn window_after(extra: Value) -> (Option<i64>, Option<i64>) {
    let (fx, _channel) = messenger().await;
    fx.exec(r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = 1000, "lastActivityAt" = 5 WHERE "id" = 6;"#).await;
    send(&fx, job(text_step("hi"), extra)).await;
    (
        fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        fx.int(r#"SELECT "lastActivityAt" FROM "Conversation" WHERE "id" = 6"#)
            .await,
    )
}

#[tokio::test]
async fn a_plain_bot_flow_step_closes_the_human_reply_window_and_bumps_activity() {
    let (since, activity) = window_after(json!({})).await;
    assert_eq!(since, None);
    assert!(activity.is_some_and(|a| a > 5));
}

#[tokio::test]
async fn inbox_sequence_and_broadcast_steps_keep_the_window_open() {
    let broadcast = json!({ "type": "broadcast", "broadcastId": "b", "contactInboxId": "5" });
    let sequence = json!({ "type": "sequenceSchedule", "contactInboxId": "5", "sequenceStepId": "1", "sequenceId": "2", "dispatchId": "3" });
    for extra in [
        json!({ "sendFrom": "inbox" }),
        json!({ "metadata": sequence }),
        json!({ "metadata": broadcast.clone() }),
        json!({ "metadata": broadcast, "isBulkBroadcast": true }),
    ] {
        assert_eq!(window_after(extra.clone()).await.0, Some(1000), "{extra}");
    }
}

#[tokio::test]
async fn a_bulk_broadcast_step_stays_out_of_the_inbox_sort() {
    let broadcast = json!({ "type": "broadcast", "broadcastId": "b", "contactInboxId": "5" });
    assert_eq!(
        window_after(json!({ "metadata": broadcast.clone(), "isBulkBroadcast": true }))
            .await
            .1,
        Some(5)
    );
    // A broadcast button continuation (not bulk) stays visible.
    assert!(
        window_after(json!({ "metadata": broadcast }))
            .await
            .1
            .is_some_and(|a| a > 5)
    );
}

#[tokio::test]
async fn a_paused_bot_or_a_comment_thread_never_closes_the_window() {
    let (fx, _channel) = messenger().await;
    fx.exec(r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = 1000, "botEnabled" = 0 WHERE "id" = 6;"#).await;
    send(&fx, job(text_step("hi"), json!({}))).await;
    assert_eq!(
        fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(1000)
    );
}

#[tokio::test]
async fn persists_rich_response_metadata_and_quick_replies_on_the_carrier_message() {
    let (fx, channel) = messenger().await;
    let preset = json!({ "type": "postback", "title": "Skip", "payload": "SKIP" });
    send(
        &fx,
        job(
            text_step("pick one"),
            json!({
                "richResponse": { "kind": "faq" },
                "quickReplies": [{ "id": "q1", "type": "quickReply", "title": "Yes" }],
                "presetQuickReplies": [preset.clone()],
            }),
        ),
    )
    .await;
    let attributes = &messages(&fx).await[0].2;
    assert_eq!(attributes["richResponse"], json!({ "kind": "faq" }));
    assert_eq!(attributes["type"], "template");
    let buttons = attributes["payload"]["buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 2);
    // The caller-encoded preset reaches the channel untouched, after the
    // step's own quick replies.
    assert_eq!(buttons[1], preset);
    let sent = channel.calls()[0].data["quickReplies"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], preset);
}

#[tokio::test]
async fn updates_contact_inbox_and_conversation_activity_and_marks_the_conversation_read() {
    let (fx, _channel) = messenger().await;
    send(&fx, job(text_step("hi"), json!({}))).await;
    assert!(
        fx.int(r#"SELECT "lastMessageAt" FROM "ContactInbox" WHERE "id" = 5"#)
            .await
            .is_some()
    );
    assert!(
        fx.int(r#"SELECT "lastActivityAt" FROM "Conversation" WHERE "id" = 6"#)
            .await
            .is_some()
    );
    assert!(
        fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn a_failed_send_records_the_error_and_does_not_mark_the_conversation_read() {
    let (fx, channel) = messenger().await;
    channel.fail_next(crate::channel::SendFailure::Other("provider down".into()));
    send(&fx, job(text_step("hi"), json!({}))).await;
    assert_eq!(messages(&fx).await[0].4.as_deref(), Some("provider down"));
    assert_eq!(
        fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        None
    );
}

#[tokio::test]
async fn forwards_a_private_comment_anchor_only_on_messenger_and_instagram() {
    let anchor = json!({ "commentId": "comment-1", "replyChannel": "private" });
    for (channel_name, forwarded) in [
        ("messenger", true),
        ("instagram", true),
        ("whatsapp", false),
    ] {
        let mut fx = Fixture::new().await;
        let channel = fx.on_channel(channel_name).await;
        send(
            &fx,
            job(text_step("hi"), json!({ "commentAnchor": anchor.clone() })),
        )
        .await;
        let call = &channel.calls()[0];
        assert_eq!(call.handler, "sendFlowStep", "{channel_name}");
        assert_eq!(
            call.data["commentAnchor"],
            if forwarded {
                anchor.clone()
            } else {
                Value::Null
            },
            "{channel_name}"
        );
    }
}

#[tokio::test]
async fn a_public_comment_anchor_is_a_comment_message_through_the_comment_channel() {
    for channel_name in ["messenger", "instagram"] {
        let mut fx = Fixture::new().await;
        let channel = fx.on_channel(channel_name).await;
        send(
            &fx,
            job(
                text_step("thanks!"),
                json!({ "commentAnchor": { "commentId": "comment-1", "replyChannel": "public" } }),
            ),
        )
        .await;
        let stored = messages(&fx).await;
        assert_eq!(stored[0].1.as_deref(), Some("comment"), "{channel_name}");
        assert_eq!(stored[0].2["replyToCommentId"], "comment-1");
        let handlers: Vec<&str> = channel.calls().iter().map(|c| c.handler).collect();
        assert_eq!(handlers, vec!["sendComment"], "{channel_name}");
        // The outbound mark-read never runs for a public reply.
        assert_eq!(
            fx.int(r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            None
        );
    }
}

/// `describe("sendChatMessage")`.
mod chat_message {
    use super::*;

    async fn chat(fx: &Fixture, extra: Value) {
        let mut data = json!({ "conversation": "6", "contactInbox": "5", "text": "hello" });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            data[k] = v;
        }
        send_chat_message(&fx.deps, &data).await.unwrap();
    }

    #[tokio::test]
    async fn updates_contact_inbox_last_message_and_conversation_activity() {
        let (fx, channel) = messenger().await;
        fx.exec(r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = 1000 WHERE "id" = 6;"#)
            .await;
        chat(&fx, json!({})).await;
        assert!(
            fx.int(r#"SELECT "lastMessageAt" FROM "ContactInbox" WHERE "id" = 5"#)
                .await
                .is_some()
        );
        assert!(
            fx.int(r#"SELECT "lastActivityAt" FROM "Conversation" WHERE "id" = 6"#)
                .await
                .is_some()
        );
        assert_eq!(
            fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            None
        );
        assert_eq!(channel.calls()[0].handler, "sendMessage");
    }

    #[tokio::test]
    async fn an_initial_bulk_broadcast_prompt_stays_out_of_the_inbox_sort() {
        let (fx, _channel) = messenger().await;
        fx.exec(r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = 1000, "lastActivityAt" = 5 WHERE "id" = 6;"#).await;
        let broadcast = json!({ "type": "broadcast", "broadcastId": "b", "contactInboxId": "5" });
        chat(
            &fx,
            json!({ "metadata": broadcast, "isBulkBroadcast": true }),
        )
        .await;
        assert_eq!(
            fx.int(r#"SELECT "lastActivityAt" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            Some(5)
        );
        assert_eq!(
            fx.int(r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = 6"#)
                .await,
            Some(1000)
        );
    }

    #[tokio::test]
    async fn falls_back_to_the_url_as_text_when_the_media_is_not_copied() {
        let (fx, channel) = messenger().await;
        chat(&fx, json!({ "url": "https://cdn.example.test/a.png" })).await;
        assert_eq!(
            messages(&fx).await[0].0.as_deref(),
            Some("hello\nhttps://cdn.example.test/a.png")
        );
        assert_eq!(channel.calls().len(), 1);
    }
}
