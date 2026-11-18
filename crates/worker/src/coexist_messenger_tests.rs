//! `messenger-sync.test.ts` main path: both phases in one chunk.
use std::sync::Mutex;

use chatbotx_business::connection::credentials::{UpstreamReply, UpstreamRequest};

use super::*;
use crate::contact_scan::tests::setup;

/// Answers `/conversations` and `/messages` with their own bodies.
struct Graph(Mutex<Vec<String>>, Value, Value);

impl Upstream for Graph {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl std::future::Future<Output = UpstreamReply> + Send {
        let body = if request.url.contains("/conversations?") {
            self.1.to_string()
        } else {
            self.2.to_string()
        };
        self.0.lock().unwrap().push(request.url);
        async move { Ok((200, body)) }
    }
}

#[tokio::test]
async fn a_run_imports_the_contacts_then_their_messages() {
    let (deps, _) = setup().await;
    deps.db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "IntegrationMessenger" SET "coexistEnabled" = 1 WHERE "id" = 5"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "CoexistSyncRun" ("id", "workspaceId", "integrationId", "channel", "type", "status", "triggerSource")
                   VALUES (10, 2, 5, 'messenger', 'coexist', 'init', 'connect')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
    let graph = Graph(
        Mutex::default(),
        json!({ "data": [{ "id": "t1", "updated_time": recent, "participants": { "data": [
            { "id": "p1", "name": "Page" }, { "id": "u1", "name": "Ana Lima" } ] } }] }),
        json!({ "data": [
            { "id": "m2", "message": "write me at Ana@Example.com", "from": { "id": "u1" }, "created_time": recent },
            { "id": "m1", "from": { "id": "p1" }, "created_time": recent, "attachments": { "data": [
                { "id": "a1", "mime_type": "image/png", "image_data": { "url": "https://cdn.test/a.png", "width": 4, "height": 3 } } ] } },
            { "id": "m0", "from": { "id": "u1" }, "created_time": recent },
        ] }),
    );
    let data = json!({ "runId": "10", "integrationId": "5", "workspaceId": "2" });
    run_with(&deps, &graph, &data).await.unwrap();
    assert_eq!(graph.0.lock().unwrap().len(), 3, "two walks + one thread");

    let (status, phase, contacts, messages): (String, String, i64, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "status", "messengerSyncPhase", "importedContactCount", "importedMessageCount" FROM "CoexistSyncRun" WHERE "id" = 10"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        (status.as_str(), phase.as_str(), contacts, messages),
        ("succeeded", "messages", 1, 2)
    );
    let (email, outgoing, attachments): (Option<String>, String, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT ct."email", m."senderType", (SELECT count(*) FROM "Attachment" a WHERE a."messageId" = m."id")
                   FROM "Message" m JOIN "Contact" ct ON ct."workspaceId" = 2 WHERE m."sourceId" = 'm1'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(email.as_deref(), Some("ana@example.com"));
    assert_eq!((outgoing.as_str(), attachments), ("user", 1));
}

#[test]
fn a_generic_template_becomes_a_button_card() {
    let m = json!({ "attachments": { "data": [{ "id": "a", "generic_template": {
        "title": "Pick", "cta": [{ "type": "web_url", "url": "https://x.test", "title": "Open" }, { "title": "Ok" }] } }] } });
    let (text, card) = button_template(&m).unwrap();
    assert_eq!(text, "Pick");
    assert_eq!(card["payload"]["buttons"][0]["buttonType"], "url");
    assert_eq!(card["payload"]["buttons"][1]["id"], "a-1");
    assert!(attachments_of(&m).is_empty());
}
