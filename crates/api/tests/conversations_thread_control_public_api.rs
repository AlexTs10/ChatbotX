//! Port of `apps/builder/__tests__/conversations-thread-control-public-api.test.ts`
//! (Q.D12). Adapted: the mocked `requestConversationThreadControl` /
//! `syncConversationThreadOwner` become the real service over real rows, with the
//! Graph API answered by a local server (`ApiState.graph_url`).

mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::{get, post};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

fn reply(status: u16, value: Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::from_u16(status).unwrap(),
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

#[derive(Default)]
struct Graph {
    calls: Mutex<Vec<(String, Value)>>,
    answer: Mutex<Option<(u16, Value)>>,
}

async fn graph_post(
    State(g): State<Arc<Graph>>,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    g.calls.lock().unwrap().push((uri.path().to_owned(), body));
    let (status, answer) = g
        .answer
        .lock()
        .unwrap()
        .clone()
        .unwrap_or((200, json!({ "success": true })));
    reply(status, answer)
}

async fn graph_get(State(g): State<Arc<Graph>>, uri: axum::http::Uri) -> axum::response::Response {
    g.calls
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), Value::Null));
    reply(
        200,
        json!({ "data": [{ "thread_owner": { "app_id": "partner-app" } }] }),
    )
}

async fn start(f: &mut Fixture) -> Arc<Graph> {
    let graph = Arc::new(Graph::default());
    let app = axum::Router::new()
        .route("/v23.0/phone-1/thread_control", post(graph_post))
        .route("/v23.0/me/take_thread_control", post(graph_post))
        .route("/v23.0/me/thread_owner", get(graph_get))
        .with_state(Arc::clone(&graph));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.graph_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    graph
}

/// Contact 123: WhatsApp contact inbox 52 (inbox 6), Messenger 53 (inbox 7), webchat 50
/// (inbox 5); contact 124: WhatsApp contact inbox 54. Conversation 70 (contact 123).
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId")
                   VALUES (5, ?1, 'Web', 'webchat', 'web-5'), (6, ?1, 'Wa', 'whatsapp', 'wa-6'),
                          (7, ?1, 'Fb', 'messenger', 'fb-7')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (123, ?1, 'Ada'), (124, ?1, 'Bo')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (50, 123, 123, 5, 'webchat', 'direct', 'guest-123'),
                          (52, 123, 123, 6, 'whatsapp', 'direct', '15550001'),
                          (53, 123, 123, 7, 'messenger', 'direct', 'psid-1'),
                          (54, 124, 124, 6, 'whatsapp', 'direct', '15550002')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "createdAt", "workspaceId", "contactId") VALUES (70, 0, ?1, 123)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (101, '{"tokens":{"accessToken":"t"},"metadata":{"phoneNumber":{"id":"phone-1"}}}', 'phone-1', 'waba-1', 'b', 'Wa', ?1, 6)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (102, '{"tokens":{"accessToken":"t"},"clientId":"own-app"}', 'page-1', 'Fb', '[]', '[]', '[]', ?1, 7)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

async fn run(f: &Fixture, ws: i64, body: Value) -> (u16, Value) {
    call(
        f,
        "conversations.threadControl",
        ws,
        &[("id", "70")],
        None,
        Some(body),
    )
    .await
}

// describe("POST /v1/conversations/{id}/thread-control")

#[tokio::test]
async fn applies_the_action_to_a_conversation_of_the_tokens_workspace() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "52", "action": "take" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "applied");
    assert_eq!(body["snapshot"]["contactInboxId"], "52");
    assert_eq!(body["snapshot"]["threadControlState"], "owned");
    assert_eq!(body["snapshot"]["threadOwnerRole"], "escalation");
    assert_eq!(body["snapshot"]["threadControlLastEvent"], "taken");
    let calls = graph.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![(
            "/v23.0/phone-1/thread_control".to_owned(),
            json!({ "messaging_product": "whatsapp", "to": "15550001", "action": "take" })
        )]
    );
}

#[tokio::test]
async fn forwards_the_target_role_and_the_handover_note_on_a_pass() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(
        &f,
        WS,
        json!({ "contactInboxId": "52", "action": "pass", "targetRole": "marketing", "metadata": "Delivery issue" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["snapshot"]["threadOwnerRole"], "marketing");
    assert_eq!(
        graph.calls.lock().unwrap()[0].1,
        json!({ "messaging_product": "whatsapp", "to": "15550001", "action": "pass",
                "metadata": "Delivery issue", "control_pass": { "target_role": "marketing" } })
    );
}

#[tokio::test]
async fn a_refused_take_is_a_normal_not_escalation_result_not_an_error() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    *graph.answer.lock().unwrap() = Some((
        400,
        json!({ "error": { "message": "Only escalation may take", "code": 2_494_191 } }),
    ));
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "52", "action": "take" })).await;
    assert_eq!((status, body), (200, json!({ "status": "notEscalation" })));
}

#[tokio::test]
async fn any_other_channel_failure_is_an_error_with_the_channels_message() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    *graph.answer.lock().unwrap() = Some((
        400,
        json!({ "error": { "message": "Not the owner", "code": 10 } }),
    ));
    let (status, body) = run(
        &f,
        WS,
        json!({ "contactInboxId": "52", "action": "release" }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], "threadControlFailed");
    assert!(
        body["message"].as_str().unwrap().contains("Not the owner"),
        "{body}"
    );
}

#[tokio::test]
async fn a_contact_inbox_of_another_contact_stays_a_404() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "54", "action": "take" })).await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
    assert!(graph.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_conversation_outside_the_workspace_is_refused_before_the_channel() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(
        &f,
        OTHER_WS,
        json!({ "contactInboxId": "52", "action": "take" }),
    )
    .await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
    assert!(graph.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn target_role_and_metadata_are_refused_on_the_wrong_actions() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = run(
        &f,
        WS,
        json!({ "contactInboxId": "52", "action": "release", "targetRole": "marketing", "metadata": "x" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
}

// describe("POST /v1/conversations/{id}/thread-control with action sync")

#[tokio::test]
async fn syncs_returns_the_snapshot_and_never_asks_the_channel_to_change_ownership() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "53", "action": "sync" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["snapshot"]["threadControlState"], "standby");
    assert_eq!(body["snapshot"]["threadOwnerAppId"], "partner-app");
    let calls = graph.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "/v23.0/me/thread_owner");
}

#[tokio::test]
async fn a_channel_that_cannot_report_an_owner_leaves_the_state_unchanged_on_sync() {
    let mut f = fixture().await;
    let graph = start(&mut f).await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "52", "action": "sync" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["snapshot"]["threadControlState"], Value::Null);
    assert!(graph.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unsupported_channel_is_the_declared_thread_control_unsupported() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "50", "action": "take" })).await;
    assert_eq!(
        (status, &body["code"]),
        (400, &json!("threadControlUnsupported"))
    );
}

#[tokio::test]
async fn a_contact_inbox_of_another_contact_stays_a_404_during_sync() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = run(&f, WS, json!({ "contactInboxId": "54", "action": "sync" })).await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
}
