//! Port of `apps/worker/__tests__/analytics-dashboard-listener.test.ts` and
//! the `onClicked` / ref-link handlers of `packages/analytics`
//! (`magic-link-analytics.service.test.ts`, `ref-link-analytics.service.test.ts`).
//! Adapted: the mocked sub-handlers become the real inserts into a migrated
//! database; "the failed stream ids" are the entry ids `record_dashboard`
//! returns (a failure is a row the table rejects).

use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_db::Db;
use chatbotx_jobs::events::event_bus_message_id_key;
use chatbotx_worker::analytics_events::{
    magic_link_clicked, record_clicks, record_dashboard, ref_link_opened,
};
use serde_json::{Value, json};

const WS: i64 = 7001;

async fn db() -> Db {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-analytics-events-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@example.test')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "MagicLink" ("id", "workspaceId", "name", "url") VALUES (81, ?1, 'promo', 'https://x.test')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (92, 'Welcome', ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Reflink" ("id", "workspaceId", "name", "flowId") VALUES (91, ?1, 'promo', 92)"#,
            [WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    db
}

fn with_entry(mut payload: Value, entry: &str) -> Value {
    payload[event_bus_message_id_key()] = json!(entry);
    payload
}

async fn count(db: &Db, sql: &'static str) -> i64 {
    db.read(move |c| Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?))
        .await
        .expect("count")
}

#[tokio::test]
async fn returns_only_the_failed_sub_handler_entry_ids_from_a_mixed_batch() {
    let db = db().await;
    let contact = with_entry(
        json!({ "eventType": "contact:created", "workspaceId": WS.to_string(), "contactId": "5",
                "occurredAt": "2026-10-01T00:00:00Z", "channel": "messenger" }),
        "1-0",
    );
    // A type the table's CHECK rejects: that group fails alone.
    let conversation = with_entry(
        json!({ "eventType": "conversation:bogus", "workspaceId": WS.to_string(), "conversationId": "6",
                "occurredAt": "2026-10-01T00:00:00Z" }),
        "2-0",
    );
    let failed = record_dashboard(&db, vec![contact.clone(), conversation]).await;
    assert_eq!(failed, ["2-0"]);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AnalyticsContactEvent" WHERE "eventType" = 'contact_created'"#
        )
        .await,
        1
    );
    // A redelivery of the same entry is deduplicated on (occurredAt, eventId).
    assert!(record_dashboard(&db, vec![contact]).await.is_empty());
    assert_eq!(
        count(&db, r#"SELECT COUNT(*) FROM "AnalyticsContactEvent""#).await,
        1
    );
}

#[tokio::test]
async fn routes_agent_replied_to_its_table_and_drops_an_empty_window() {
    let db = db().await;
    let ok = json!({ "eventType": "agent:replied", "eventId": "e-1", "workspaceId": WS.to_string(),
        "conversationId": "2", "contactId": "3", "adminId": "1",
        "occurredAt": "2026-10-01T00:01:00Z", "awaitingSince": "2026-10-01T00:00:00Z",
        "measuredFrom": "2026-10-01T00:00:00Z", "responseSeconds": 60, "isFirstResponse": true });
    let mut empty = ok.clone();
    empty["eventId"] = json!("e-2");
    empty["measuredFrom"] = ok["occurredAt"].clone();
    assert!(record_dashboard(&db, vec![ok, empty]).await.is_empty());
    assert_eq!(
        count(&db, r#"SELECT COUNT(*) FROM "AnalyticsAgentResponseEvent""#).await,
        1
    );
}

#[tokio::test]
async fn bot_messages_count_toward_the_workspace_usage() {
    let db = db().await;
    let sent = json!({ "eventType": "message:bot_sent", "workspaceId": WS.to_string(), "contactId": "5",
        "occurredAt": "2026-10-01T00:00:00Z", "senderType": "bot" });
    let mut second = sent.clone();
    second["eventId"] = json!("m-2");
    assert!(record_dashboard(&db, vec![sent, second]).await.is_empty());
    assert_eq!(
        count(&db, r#"SELECT COUNT(*) FROM "AnalyticsMessageEvent""#).await,
        2
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COALESCE(SUM("botMessagesUsed"), 0) FROM "WorkspaceUsage""#
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COALESCE(SUM("count"), 0) FROM "WorkspaceBotMessageUsage""#
        )
        .await,
        2
    );
}

fn click(action: Value) -> Value {
    json!({
        "context": { "workspaceId": WS.to_string(), "contactId": "5", "contactInboxId": "6" },
        "action": action,
        "occurredAt": "2026-10-01T00:00:00.750Z",
    })
}

#[tokio::test]
async fn a_magic_link_click_is_one_stat_at_the_second() {
    let db = db().await;
    let payloads = vec![
        click(json!({ "magicLinkId": "81", "clickType": "magic_link" })),
        // Not a magic-link click: skipped.
        click(json!({ "magicLinkId": "81", "clickType": "button" })),
    ];
    let failed = record_clicks(&db, "magic-link-stats", magic_link_clicked, payloads).await;
    assert!(failed.is_empty());
    let at = count(&db, r#"SELECT "occurredAt" FROM "MagicLinkStat""#).await;
    assert_eq!(at % 1000, 0);
    assert_eq!(
        count(&db, r#"SELECT COUNT(*) FROM "MagicLinkStat""#).await,
        1
    );
}

#[tokio::test]
async fn a_ref_link_open_is_one_stat() {
    let db = db().await;
    let payloads = vec![click(json!({ "refId": "91" })), click(json!({}))];
    assert!(
        record_clicks(&db, "ref-link-stats", ref_link_opened, payloads)
            .await
            .is_empty()
    );
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "RefLinkStat""#).await, 1);
}
