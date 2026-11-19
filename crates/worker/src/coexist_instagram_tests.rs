//! `coexist-instagram-sync.test.ts`: the native and the Facebook-linked
//! pulls over a scripted Graph.
use std::sync::Mutex;

use chatbotx_business::connection::credentials::{
    UpstreamHeadersReply, UpstreamReply, UpstreamRequest,
};

use super::*;
use crate::contact_scan::tests::setup;

/// Answers by URL: conversations, the user node, else the messages edge.
struct Graph {
    seen: Mutex<Vec<String>>,
    conversations: Value,
    messages: Value,
    /// The messages page after the `c2` cursor (`messages.after(c2)`).
    older: Value,
    usage: Option<&'static str>,
}

impl Upstream for Graph {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl std::future::Future<Output = UpstreamReply> + Send {
        let reply = self.reply(&request.url);
        self.seen.lock().unwrap().push(request.url);
        async move { Ok(reply) }
    }

    fn send_with_headers(
        &self,
        request: UpstreamRequest,
    ) -> impl std::future::Future<Output = UpstreamHeadersReply> + Send {
        let (status, body) = self.reply(&request.url);
        self.seen.lock().unwrap().push(request.url);
        let headers = self
            .usage
            .map(|u| vec![("x-app-usage".to_owned(), u.to_owned())])
            .unwrap_or_default();
        async move { Ok((status, body, headers)) }
    }
}

impl Graph {
    fn reply(&self, url: &str) -> (u16, String) {
        if url.contains("/conversations?") {
            (200, self.conversations.to_string())
        } else if url.contains("fields=id%2Cname%2Cusername") {
            (
                200,
                json!({ "id": "u1", "name": "Ana Maria Lima" }).to_string(),
            )
        } else if url.contains("after%28c2%29") {
            (200, self.older.to_string())
        } else {
            (200, self.messages.to_string())
        }
    }
}

async fn seed(deps: &Deps, kind: &str, auth: &str) {
    let (kind, auth) = (kind.to_owned(), auth.to_owned());
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (6, 2, 'Shop', 'instagram', 'ig-1')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "conversationStarters", "persistentMenus", "workspaceId", "inboxId", "type", "coexistEnabled")
                   VALUES (7, ?1, 'ig-1', 'page-1', 'Shop', 'shop', '[]', '[]', 2, 6, ?2, 1)"#,
                params![auth, kind],
            )?;
            tx.execute(
                r#"INSERT INTO "CoexistSyncRun" ("id", "workspaceId", "integrationId", "channel", "type", "status", "triggerSource")
                   VALUES (11, 2, 7, 'instagram', 'coexist', 'init', 'popup-enable')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn data() -> Value {
    json!({ "runId": "11", "integrationId": "7", "workspaceId": "2" })
}

async fn run_row(deps: &Deps) -> (String, i64, i64, i64) {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "status", "importedContactCount", "importedMessageCount", "currentPageNumber" FROM "CoexistSyncRun" WHERE "id" = 11"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn native_pull_imports_the_customer_with_the_profile_name() {
    let (deps, _) = setup().await;
    seed(
        &deps,
        "instagram",
        r#"{"tokens":{"accessToken":"ig-tok"},"metadata":{"igId":"ig-1"}}"#,
    )
    .await;
    let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
    let graph = Graph {
        seen: Mutex::default(),
        conversations: json!({ "data": [{ "id": "c1", "updated_time": recent, "participants": { "data": [
            { "id": "ig-1", "username": "shop" }, { "id": "u1", "username": "ana.l" } ] } }] }),
        messages: json!({ "id": "c1", "messages": { "data": [
            { "id": "m2", "message": "call me +4915112345678", "from": { "id": "u1" }, "created_time": recent },
            { "id": "m1", "from": { "id": "ig-1" }, "created_time": recent, "attachments": { "data": [
                { "id": "a1", "mime_type": "image/png", "image_data": { "url": "https://cdn.test/a.png" } },
                { "mime_type": "image/png", "image_data": { "url": "https://cdn.test/no-id.png" } } ] } },
            { "id": "m0", "from": { "id": "u1" }, "created_time": recent },
        ] } }),
        older: Value::Null,
        usage: Some(r#"{"call_count":10}"#),
    };
    run_with(&deps, &graph, &data()).await.unwrap();
    let seen = graph.seen.lock().unwrap().clone();
    assert!(seen[0].starts_with("https://graph.instagram.com/v22.0/ig-1/conversations?"));
    assert!(!seen[0].contains("platform="));
    assert!(
        seen.iter()
            .any(|u| u.starts_with("https://graph.instagram.com/v22.0/u1?"))
    );
    assert_eq!(run_row(&deps).await, ("succeeded".into(), 1, 2, 1));
    let (first, last, username, attachments): (String, String, String, i64) = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT ct."firstName", ct."lastName", ci."sourceUsername",
                          (SELECT count(*) FROM "Attachment")
                   FROM "ContactInbox" ci JOIN "Contact" ct ON ct."id" = ci."contactId" WHERE ci."sourceId" = 'u1'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        (
            first.as_str(),
            last.as_str(),
            username.as_str(),
            attachments
        ),
        ("Ana", "Maria Lima", "ana.l", 1)
    );
}

#[tokio::test]
async fn facebook_pull_reads_the_page_node_and_a_disabled_channel_fails_the_run() {
    let (deps, _) = setup().await;
    seed(
        &deps,
        "facebook",
        r#"{"tokens":{"accessToken":"page-tok"},"metadata":{"igId":"ig-1","pageId":"page-1"}}"#,
    )
    .await;
    let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
    let graph = Graph {
        seen: Mutex::default(),
        conversations: json!({ "data": [
            { "id": "c1", "updated_time": recent, "participants": { "data": [ { "id": "ig-1" }, { "id": "u2", "username": "bo" } ] } },
            { "id": "c2", "updated_time": recent, "participants": { "data": [ { "id": "ig-1" } ] } },
        ] }),
        messages: json!({ "id": "c1", "messages": { "data": [
            { "id": "m1", "message": "hi", "from": { "id": "u2" }, "created_time": recent } ] } }),
        older: Value::Null,
        usage: None,
    };
    run_with(&deps, &graph, &data()).await.unwrap();
    let seen = graph.seen.lock().unwrap().clone();
    assert!(seen[0].starts_with("https://graph.facebook.com/v23.0/page-1/conversations?"));
    assert!(seen[0].contains("platform=instagram"));
    // No user-node read for a Facebook-linked account.
    assert!(
        !seen
            .iter()
            .any(|u| u.contains("fields=id%2Cname%2Cusername"))
    );
    let (status, contacts, messages, _) = run_row(&deps).await;
    assert_eq!((status.as_str(), contacts, messages), ("succeeded", 1, 1));

    deps.db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "IntegrationInstagram" SET "coexistEnabled" = 0"#,
                [],
            )?;
            tx.execute(
                r#"UPDATE "CoexistSyncRun" SET "status" = 'init' WHERE "id" = 11"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    run_with(&deps, &graph, &data()).await.unwrap();
    let error: String = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "status" || ':' || "currentError" FROM "CoexistSyncRun" WHERE "id" = 11"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(error, format!("failed:{NOT_FOUND}"));
}

#[test]
fn messages_before_the_window_drop_after_the_first_hundred() {
    let old = json!({ "id": "m", "message": "x", "from": { "id": "u" }, "created_time": "2020-01-01T00:00:00Z" });
    let cutoff = now_ms() - STORE_WINDOW_MS;
    assert!(historical_message(&old, "ig", cutoff, 100).is_some());
    assert!(historical_message(&old, "ig", cutoff, 101).is_none());
    let empty = json!({ "id": "m", "from": { "id": "ig" } });
    assert!(historical_message(&empty, "ig", cutoff, 1).is_none());
    assert_eq!(
        split_display_name(Some("  Ana   Maria Lima ")),
        (Some("Ana".into()), Some("Maria Lima".into()))
    );
}

async fn marker(deps: &Deps) -> Option<i64> {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT cv."aiContextLastMessageId" FROM "Conversation" cv
                   JOIN "ContactInbox" ci ON ci."contactId" = cv."contactId" WHERE ci."sourceId" = 'u1'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn walks_all_message_pages_and_advances_the_ai_marker_by_default() {
    for ai_reads in [false, true] {
        let (deps, _) = setup().await;
        seed(
            &deps,
            "instagram",
            r#"{"tokens":{"accessToken":"t"},"metadata":{"igId":"ig-1"}}"#,
        )
        .await;
        if ai_reads {
            deps.db
                .write(|tx| {
                    tx.execute(
                        r#"UPDATE "IntegrationInstagram" SET "coexistAiReadsSyncedHistory" = 1"#,
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
        let graph = Graph {
            seen: Mutex::default(),
            conversations: json!({ "data": [{ "id": "c1", "updated_time": recent, "participants": { "data": [
                { "id": "ig-1" }, { "id": "u1", "username": "ana" } ] } }] }),
            messages: json!({ "id": "c1", "messages": {
                "data": [{ "id": "m2", "message": "two", "from": { "id": "u1" }, "created_time": recent }],
                "paging": { "cursors": { "after": "c2" }, "next": "https://graph.instagram.com/next" } } }),
            older: json!({ "id": "c1", "messages": {
                "data": [{ "id": "m1", "message": "one", "from": { "id": "ig-1" }, "created_time": recent }] } }),
            usage: None,
        };
        run_with(&deps, &graph, &data()).await.unwrap();
        let (status, _, messages, _) = run_row(&deps).await;
        assert_eq!((status.as_str(), messages), ("succeeded", 2));
        let newest: i64 = deps
            .db
            .read(|c| Ok(c.query_row(r#"SELECT max("id") FROM "Message""#, [], |r| r.get(0))?))
            .await
            .unwrap();
        // The marker keeps the newest synced message unless the AI reads history.
        assert_eq!(marker(&deps).await, (!ai_reads).then_some(newest));
    }
}

#[tokio::test]
async fn a_page_without_a_processed_conversation_keeps_the_watermark() {
    let (deps, _) = setup().await;
    seed(
        &deps,
        "instagram",
        r#"{"tokens":{"accessToken":"t"},"metadata":{"igId":"ig-1"}}"#,
    )
    .await;
    let frontier = now_ms() - 7_200_000;
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "CoexistSyncRun" SET "lastSyncedAt" = ?1 WHERE "id" = 11"#,
                [frontier],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Newer than the frontier: already synced, skipped.
    let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
    let graph = Graph {
        seen: Mutex::default(),
        conversations: json!({ "data": [{ "id": "c1", "updated_time": recent, "participants": { "data": [{ "id": "u1" }] } }] }),
        messages: Value::Null,
        older: Value::Null,
        usage: None,
    };
    run_with(&deps, &graph, &data()).await.unwrap();
    let kept: Option<i64> = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "lastSyncedAt" FROM "CoexistSyncRun" WHERE "id" = 11"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(kept, Some(frontier));
    assert_eq!(graph.seen.lock().unwrap().len(), 1, "no message fetch");
}
