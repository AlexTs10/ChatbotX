//! Q.P7-V4: ports `apps/worker/__tests__/template-flow-response.test.ts`
//! (`captureTemplateFlowResponse`). The mocked resolver and service run for
//! real on seeded rows: the stored custom field value and the
//! `WhatsappFlow.completedCount` show whether the response was applied.
//! Also the producer: the WhatsApp parser's `templateFlowToken`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_core::flow_config::steps::encode_template_flow_token;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_worker::Deps;
use chatbotx_worker::channels::whatsapp::parse_incoming;
use chatbotx_worker::template_flow_response::capture;
use serde_json::{Value, json};

const WS: i64 = 9101;
const OTHER_WS: i64 = 9102;
const BROADCAST: &str = "11612473309626368";

struct Fixture {
    db: Arc<Db>,
    deps: Deps,
    dir: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Contact 10 (conversation 20, WhatsApp contact inbox 30 on inbox 40) of
/// `WS`; conversation 21 of `OTHER_WS`; field 50 `email`; integration 60 with
/// Flow `wa-flow-1`; broadcast `BROADCAST` whose first button is `button`.
async fn fixture(button: Value) -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-template-flow-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    let template = json!({ "button": [button] }).to_string();
    db.write(move |tx| {
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1, 'o@example.test')"#, [])?;
        for ws in [WS, OTHER_WS] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
                [ws],
            )?;
        }
        tx.execute(r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (10, ?1), (11, ?2)"#, params![WS, OTHER_WS])?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (40, ?1, 'Wa', 'whatsapp', 'p-1')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
               VALUES (30, 10, 10, 40, 'whatsapp', 'direct', '84900000000')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (20, ?1, 10), (21, ?2, 11)"#,
            params![WS, OTHER_WS],
        )?;
        tx.execute(
            r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type") VALUES (50, ?1, 'email', 'shortText')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
               VALUES (60, '{}', 'p-1', 'w-1', 'b-1', 'Wa', ?1, 40)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "WhatsappFlow" ("id", "name", "integrationWhatsappId", "sourceId", "status") VALUES (70, 'Form', 60, 'wa-flow-1', 'PUBLISHED')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "integrationWhatsappId", "templateId", "templateData", "status", "schedulesType", "schedulesAt", "subaction", "channel")
               VALUES (?1, 'B', ?2, 60, 1, ?3, 'sent', 'now', 0, 'sendTemplate', 'whatsapp')"#,
            params![BROADCAST.parse::<i64>().unwrap(), WS, template],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(chatbotx_jobs::SystemClock))
            .await
            .expect("store"),
    );
    let deps = Deps {
        db: db.clone(),
        jobs: store.clone(),
        waiter: store,
        realtime: None,
        channels: chatbotx_worker::Channels::new(),
        app_url: "http://app.test".into(),
        variables: None,
        ai: Arc::new(chatbotx_worker::ai::Ai::live()),
        files: None,
        webhook_http: Arc::new(chatbotx_worker::webhook::RecordingWebhookHttp::default()),
        channel_http: None,
        events: None,
        push: None,
    };
    Fixture { db, deps, dir }
}

fn flow_button(mappings: Value) -> Value {
    json!({ "sub_type": "flow", "index": 0, "flowSourceId": "wa-flow-1", "fieldMappings": mappings })
}

fn token() -> String {
    encode_template_flow_token(
        &json!({ "origin": "b", "broadcastId": BROADCAST, "buttonIndex": 0 }),
    )
}

fn job(token: &str, conversation: i64) -> Value {
    json!({
        "workspaceId": WS.to_string(),
        "conversationId": { "id": conversation.to_string() },
        "contactInboxId": { "id": "30" },
        "messageId": "11612473309626999",
        "templateFlowToken": token,
        "flowResponse": { "email": "user@example.com", "phone": "+84900000000" },
    })
}

/// `(stored email value, completedCount)`.
async fn outcome(fx: &Fixture) -> (Option<String>, i64) {
    fx.db
        .read(|conn| {
            let value = conn
                .query_row(
                    r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 10 AND "customFieldId" = 50"#,
                    [],
                    |r| r.get(0),
                )
                .ok();
            let count = conn.query_row(
                r#"SELECT "completedCount" FROM "WhatsappFlow" WHERE "id" = 70"#,
                [],
                |r| r.get(0),
            )?;
            Ok((value, count))
        })
        .await
        .unwrap()
}

fn email_mapping() -> Value {
    json!([{ "paramKey": "email", "customFieldId": "50" }])
}

#[tokio::test]
async fn applies_the_response_to_the_resolved_flow_button() {
    let fx = fixture(flow_button(email_mapping())).await;
    capture(&fx.deps, &job(&token(), 20)).await.unwrap();
    assert_eq!(outcome(&fx).await, (Some("user@example.com".into()), 1));
}

#[tokio::test]
async fn hands_the_stored_files_of_the_response_to_the_service() {
    let fx = fixture(flow_button(email_mapping())).await;
    let mut data = job(&token(), 20);
    data["flowFiles"] = json!({ "email": [{ "mediaId": "1", "fileName": "a.pdf",
        "mimeType": "application/pdf", "size": 10, "url": "https://storage.example.com/a.pdf" }] });
    capture(&fx.deps, &data).await.unwrap();
    assert_eq!(
        outcome(&fx).await,
        (Some("https://storage.example.com/a.pdf".into()), 1)
    );
}

#[tokio::test]
async fn applies_with_no_field_mappings_when_the_button_has_none() {
    let fx = fixture(flow_button(Value::Null)).await;
    capture(&fx.deps, &job(&token(), 20)).await.unwrap();
    assert_eq!(outcome(&fx).await, (None, 1));
}

#[tokio::test]
async fn skips_an_invalid_token_a_foreign_conversation_and_an_unresolved_button() {
    let fx = fixture(flow_button(email_mapping())).await;
    capture(&fx.deps, &job("not-a-template-token", 20))
        .await
        .unwrap();
    // Conversation 21 belongs to another workspace.
    capture(&fx.deps, &job(&token(), 21)).await.unwrap();
    assert_eq!(outcome(&fx).await, (None, 0));

    let fx = fixture(json!({ "sub_type": "quick_reply", "index": 0, "payload": "p" })).await;
    capture(&fx.deps, &job(&token(), 20)).await.unwrap();
    assert_eq!(outcome(&fx).await, (None, 0));
}

fn nfm_reply(flow_token: &str) -> chatbotx_worker::channel::Received {
    let response = json!({ "flow_token": flow_token, "email": "user@example.com" }).to_string();
    parse_incoming(
        &json!({ "phoneID": "p-1", "from": "84900000000", "name": "A", "message": {
        "id": "wamid.1", "type": "interactive",
        "interactive": { "type": "nfm_reply", "nfm_reply": { "name": "flow", "body": "Sent", "response_json": response } },
    } }),
    )
}

#[test]
fn the_parser_reads_a_template_flow_token_only_for_template_tokens() {
    let template = nfm_reply(&token());
    assert_eq!(template.template_flow_token, Some(token()));
    assert_eq!(template.postback_action, None);
    // A "Send WhatsApp Flow" step's button payload is a postback instead.
    let step = nfm_reply("11612473309626368::11612473309626371");
    assert_eq!(step.template_flow_token, None);
    assert!(step.postback_action.is_some());
    assert_eq!(nfm_reply("a-random-token").template_flow_token, None);
}
