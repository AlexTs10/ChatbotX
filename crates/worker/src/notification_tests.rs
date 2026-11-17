//! `push-notification-content.test.ts`, `send-push-expo.test.ts` and
//! `expo-push-enabled-env.test.ts`. The mocked services are the fixture DB
//! (conversation 6, contact 3 "Ada Lovelace", workspace 2); the mocked Expo
//! client is `RecordingPush` with the test's chunk size of 2. The mock token
//! check (`Expo[`) is the real `isExpoPushToken` (`ExponentPushToken[..]`).

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::steps::test_support::Fixture;

fn incoming(extra: Value) -> Value {
    let mut data = json!({ "workspaceId": "2", "conversationId": "6", "messageId": "m-1" });
    for (k, v) in extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    json!({ "type": "notifyIncomingMessage", "data": data })
}

#[test]
fn push_notification_content() {
    let body = |extra: Value, lang: Option<&str>| {
        notification_content(&incoming(extra), Some("Jane"), lang).1
    };
    assert_eq!(
        notification_content(
            &incoming(json!({ "messageText": "Hey there" })),
            Some("Jane Doe"),
            Some("en")
        ),
        ("Jane Doe".to_owned(), "Hey there".to_owned())
    );
    assert_eq!(
        notification_content(
            &incoming(json!({ "messageText": "Hey there" })),
            None,
            Some("en")
        )
        .0,
        "New message"
    );
    assert_eq!(
        body(json!({ "contentType": "location" }), Some("en")),
        "Shared a location"
    );
    assert_eq!(
        body(json!({ "contentType": "refLink" }), Some("en")),
        "Sent a link"
    );
    assert_eq!(
        body(json!({ "attachmentCount": 1 }), Some("en")),
        "Sent an attachment"
    );
    assert_eq!(
        body(json!({ "attachmentCount": 2 }), Some("en")),
        "Sent 2 attachments"
    );
    let assigned = json!({ "type": "notifyConversationAssigned", "data": { "workspaceId": "2", "conversationId": "6", "assignedUserId": "1" } });
    assert_eq!(
        notification_content(&assigned, Some("Jane"), Some("en")),
        (
            "Jane".to_owned(),
            "You were assigned a conversation".to_owned()
        )
    );
    assert_eq!(
        body(json!({ "contentType": "location" }), Some("xx")),
        "Shared a location"
    );
    assert_eq!(
        body(json!({ "contentType": "location" }), Some("vi")),
        "Đã chia sẻ vị trí"
    );
    assert_eq!(
        body(json!({ "contentType": "location" }), None),
        "Shared a location"
    );
}

#[test]
fn expo_push_enabled_kill_switch() {
    assert!(!push_enabled(Some("false")));
    assert!(!push_enabled(Some("0")));
    assert!(push_enabled(Some("true")));
    assert!(push_enabled(None));
    assert!(
        is_expo_push_token("ExponentPushToken[abc]") && is_expo_push_token("ExpoPushToken[abc]")
    );
    assert!(is_expo_push_token("a1b2c3d4-e5f6-a7b8-c9d0-e1f2a3b4c5d6"));
    assert!(!is_expo_push_token("placeholder-legacy-token"));
}

const T1: &str = "ExponentPushToken[token-1]";
const T2: &str = "ExponentPushToken[token-2]";
const T3: &str = "ExponentPushToken[token-3]";

/// Users 1 (assignee of conversation 6) and 7; `tokens` = (user, token).
async fn fixture(
    tokens: &[(i64, &str)],
    tickets: Vec<Result<Vec<Value>, String>>,
) -> (Fixture, Arc<RecordingPush>) {
    let mut f = Fixture::new().await;
    f.exec(
        r#"INSERT INTO "User" ("id", "email") VALUES (7, 'b@example.test');
           INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (2, 1, 'owner'), (2, 7, 'agent');
           UPDATE "Conversation" SET "assignedUserId" = 1 WHERE "id" = 6;"#,
    )
    .await;
    let rows: Vec<(i64, String)> = tokens.iter().map(|(u, t)| (*u, (*t).to_owned())).collect();
    f.sql(move |tx| {
        for (user, token) in &rows {
            tx.execute(
                r#"INSERT INTO "UserDeviceToken" ("userId", "platform", "token") VALUES (?1, 'ios', ?2)"#,
                (user, token),
            )?;
        }
        Ok(())
    })
    .await;
    let push = Arc::new(RecordingPush {
        chunk: 2,
        tickets: Mutex::new(tickets),
        ..Default::default()
    });
    f.deps.push = Some(push.clone());
    (f, push)
}

async fn tokens(f: &Fixture) -> Option<String> {
    f.text(r#"SELECT group_concat("token", ',') FROM (SELECT "token" FROM "UserDeviceToken" ORDER BY "id")"#).await
}

fn not_registered() -> Value {
    json!({ "status": "error", "message": "not registered", "details": { "error": "DeviceNotRegistered" } })
}

#[tokio::test]
async fn sends_a_message_with_non_empty_title_and_body() {
    let (f, push) = fixture(&[(1, T1)], vec![]).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    let sent = push.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0][0]["title"].clone(), sent[0][0]["body"].clone()),
        (json!("Ada Lovelace"), json!("Hello"))
    );
    assert_eq!(
        sent[0][0]["data"],
        json!({ "workspaceId": "2", "conversationId": "6", "messageId": "m-1" })
    );
}

#[tokio::test]
async fn prunes_a_token_whose_ticket_reports_device_not_registered() {
    let (f, _) = fixture(&[(1, T1)], vec![Ok(vec![not_registered()])]).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert_eq!(tokens(&f).await, None);
}

#[tokio::test]
async fn filters_out_and_prunes_an_invalid_format_token_without_sending_it() {
    let (f, push) = fixture(&[(1, "placeholder-legacy-token")], vec![]).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert_eq!(tokens(&f).await, None);
    assert!(push.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn correlates_tickets_to_tokens_by_index_within_each_chunk() {
    let tickets = vec![
        Ok(vec![
            json!({ "status": "ok", "id": "r1" }),
            not_registered(),
        ]),
        Ok(vec![not_registered()]),
    ];
    let (f, push) = fixture(&[(1, T1), (1, T2), (1, T3)], tickets).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert_eq!(push.sent.lock().unwrap().len(), 2);
    assert_eq!(tokens(&f).await.as_deref(), Some(T1));
}

#[tokio::test]
async fn a_throwing_chunk_does_not_prevent_the_other_chunk_from_sending() {
    let tickets = vec![
        Err("network error".to_owned()),
        Ok(vec![json!({ "status": "ok", "id": "r1" })]),
    ];
    let (f, push) = fixture(&[(1, T1), (1, T2), (1, T3)], tickets).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert_eq!(push.sent.lock().unwrap().len(), 2);
    // Rust addition: all chunks failing fails the job ("All N Expo push chunk(s) failed").
    let (f, _) = fixture(&[(1, T1)], vec![Err("network error".to_owned())]).await;
    assert!(
        send_push_for_notification_job(&f.deps, &incoming(json!({})))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn returns_early_when_push_is_disabled_or_there_are_no_tokens() {
    let (mut f, push) = fixture(&[(1, T1)], vec![]).await;
    f.deps.push = None;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert!(push.sent.lock().unwrap().is_empty());
    let (f, push) = fixture(&[], vec![]).await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "messageText": "Hello" })))
        .await
        .unwrap();
    assert!(push.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn degrades_to_default_locale_content_when_the_workspace_language_is_unknown() {
    // The TS mock returns no workspace; a Rust conversation cannot outlive its
    // workspace (cascade), so the closest case is an unknown language.
    let (f, push) = fixture(&[(1, T1)], vec![]).await;
    f.exec(r#"UPDATE "Workspace" SET "language" = 'xx' WHERE "id" = 2"#)
        .await;
    send_push_for_notification_job(&f.deps, &incoming(json!({ "contentType": "location" })))
        .await
        .unwrap();
    assert_eq!(
        push.sent.lock().unwrap()[0][0]["body"],
        json!("Shared a location")
    );
}

#[tokio::test]
async fn excludes_the_acting_user_from_incoming_message_recipients() {
    let (f, push) = fixture(&[(1, T1), (7, T2)], vec![]).await;
    f.exec(r#"UPDATE "Conversation" SET "assignedUserId" = NULL WHERE "id" = 6"#)
        .await;
    send_push_for_notification_job(
        &f.deps,
        &incoming(json!({ "messageText": "Hello", "excludeUserId": "1" })),
    )
    .await
    .unwrap();
    let sent = push.sent.lock().unwrap().clone();
    assert_eq!(
        sent.iter()
            .flatten()
            .map(|m| m["to"].clone())
            .collect::<Vec<_>>(),
        vec![json!(T2)]
    );
}

#[tokio::test]
async fn assigned_job_notifies_the_assignee_only() {
    let (f, push) = fixture(&[(1, T1), (7, T2)], vec![]).await;
    let job = json!({ "type": "notifyConversationAssigned", "data": { "workspaceId": "2", "conversationId": "6", "assignedUserId": "7" } });
    handle(f.deps.clone(), crate::steps::test_support::job(job))
        .await
        .unwrap();
    let sent = push.sent.lock().unwrap().clone();
    assert_eq!(sent[0][0]["to"], json!(T2));
    assert_eq!(
        sent[0][0]["body"],
        json!("You were assigned a conversation")
    );
}
