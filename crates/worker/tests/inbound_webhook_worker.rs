//! Ports of `apps/worker/__tests__/process-inbound-webhook-request.test.ts`
//! and `inbound-webhook-request-crons.test.ts` (Q.D13a). Adapted: the
//! mocked service and action executor become a migrated SQLite database,
//! so each "was called with" reads as the `InboundWebhookRequest` row and
//! the contact's tags. Not applicable: the log-redaction cases (no bound
//! query values in Rust errors) and the injected checkpoint / settle
//! failures (no fault injection on the real database).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{JobCtx, SystemClock};
use chatbotx_worker::Deps;
use chatbotx_worker::inbound_webhook::{process_request, redispatch, skip_blocked_request};
use serde_json::{Value, json};

const WS: i64 = 5001;
const INBOX: i64 = 5101;
const HOOK: i64 = 5201;
const REQUEST: i64 = 5301;
const TAG: i64 = 5401;

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

/// `(status, skipReason, errorCode, contactId, actionResults)` of the request.
type Row = (
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
);

impl Fixture {
    async fn request(&self) -> Row {
        self.db
            .read(|conn| {
                Ok(conn.query_row(
                    r#"SELECT "status", "skipReason", "errorCode", "contactId", "actionResults"
                       FROM "InboundWebhookRequest" WHERE "id" = ?1"#,
                    [REQUEST],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )?)
            })
            .await
            .expect("request")
    }

    async fn results(&self) -> Value {
        let (.., results) = self.request().await;
        results.map_or(Value::Null, |t| serde_json::from_str(&t).expect("json"))
    }

    async fn tag_links(&self) -> i64 {
        self.db
            .read(|conn| {
                Ok(conn.query_row(
                    r#"SELECT COUNT(*) FROM "ContactToTag" WHERE "tagId" = ?1"#,
                    [TAG],
                    |r| r.get(0),
                )?)
            })
            .await
            .expect("tags")
    }

    async fn exec(&self, sql: &'static str) {
        self.db
            .write(move |tx| {
                tx.execute_batch(sql)?;
                Ok(())
            })
            .await
            .expect("exec");
    }

    async fn run(&self) -> Result<(), chatbotx_jobs::JobError> {
        process_request(&self.deps, &job(1, 3), &data()).await
    }
}

fn data() -> Value {
    json!({
        "workspaceId": WS.to_string(),
        "inboundWebhookId": HOOK.to_string(),
        "requestId": REQUEST.to_string(),
    })
}

fn job(attempts_made: u32, attempts: u32) -> JobCtx {
    JobCtx {
        id: 9,
        job_id: format!("inbound-webhook-request-{REQUEST}"),
        data: json!({ "type": "processInboundWebhookRequest", "data": data() }),
        attempts: Some(attempts),
        attempts_made,
        ..JobCtx::default()
    }
}

async fn fixture(active: bool, actions: Value, payload: Value) -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-inbound-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    db.write(move |tx| {
        tx.execute_batch(&format!(
            r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@example.com');
               INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES ({WS}, 'WS', 1);
               INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId")
                 VALUES ({INBOX}, {WS}, 'Messenger', 'messenger', 'page-1');
               INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES ({TAG}, {WS}, 'Lead');"#
        ))?;
        tx.execute(
            r#"INSERT INTO "InboundWebhook" ("id", "workspaceId", "name", "active", "token",
                 "inboxResolution", "identifiers", "fieldMappings", "actions")
               VALUES (?1, ?2, 'Hook', ?3, 'iwh_worker', ?4, ?5, ?6, ?7)"#,
            params![
                HOOK,
                WS,
                active,
                json!({ "mode": "inbox", "inboxId": INBOX.to_string() }).to_string(),
                json!([{ "kind": "channelUserId", "jsonPath": "$.sender.id" }]).to_string(),
                json!([{ "jsonPath": "$.gender", "target": "gender" }]).to_string(),
                actions.to_string(),
            ],
        )?;
        tx.execute(
            r#"INSERT INTO "InboundWebhookRequest" ("id", "workspaceId", "inboundWebhookId", "payload")
               VALUES (?1, ?2, ?3, ?4)"#,
            params![REQUEST, WS, HOOK, payload.to_string()],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(SystemClock))
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
        events: None,
        push: None,
    };
    Fixture { db, deps, dir }
}

fn add_tag() -> Value {
    json!({ "type": "addTag", "tagIds": [TAG.to_string()] })
}

fn sender() -> Value {
    json!({ "sender": { "id": "psid-1" }, "gender": "female" })
}

#[tokio::test]
async fn does_nothing_when_the_request_is_finished() {
    let f = fixture(true, json!([add_tag()]), sender()).await;
    f.exec(r#"UPDATE "InboundWebhookRequest" SET "status" = 'processed'"#)
        .await;
    f.run().await.expect("run");
    assert_eq!(f.request().await.0, "processed");
    assert_eq!(f.tag_links().await, 0);
}

#[tokio::test]
async fn an_inactive_webhook_closes_the_request_as_skipped_without_applying_anything() {
    let f = fixture(false, json!([add_tag()]), sender()).await;
    f.run().await.expect("run");
    let (status, reason, _, contact, _) = f.request().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("skipped", Some("webhookInactive"))
    );
    assert_eq!(contact, None);
    assert_eq!(f.tag_links().await, 0);
}

#[tokio::test]
async fn a_payload_that_cannot_be_applied_records_why_and_runs_no_action() {
    let f = fixture(true, json!([add_tag()]), json!({ "nobody": true })).await;
    f.run().await.expect("run");
    let (status, reason, ..) = f.request().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("skipped", Some("missingIdentifier"))
    );
    assert_eq!(f.tag_links().await, 0);
}

#[tokio::test]
async fn applies_the_payload_runs_each_action_and_completes_as_processed() {
    let f = fixture(true, json!([add_tag()]), sender()).await;
    f.run().await.expect("run");
    let (status, _, _, contact, _) = f.request().await;
    assert_eq!(status, "processed");
    assert!(contact.is_some());
    assert_eq!(f.tag_links().await, 1);
    assert_eq!(
        f.results().await,
        json!([{ "index": 0, "type": "addTag", "status": "succeeded" }])
    );
}

#[tokio::test]
async fn a_webhook_with_no_actions_still_completes() {
    let f = fixture(true, json!([]), sender()).await;
    f.run().await.expect("run");
    assert_eq!(f.request().await.0, "processed");
}

#[tokio::test]
async fn an_action_type_outside_the_allowlist_is_skipped_never_executed() {
    let f = fixture(true, json!([{ "type": "sendEmail" }, add_tag()]), sender()).await;
    f.run().await.expect("run");
    assert_eq!(f.request().await.0, "completedWithErrors");
    assert_eq!(
        f.results().await,
        json!([
            { "index": 0, "type": "sendEmail", "status": "skipped", "reason": "unsupportedAction" },
            { "index": 1, "type": "addTag", "status": "succeeded" },
        ])
    );
    assert_eq!(f.tag_links().await, 1);
}

#[tokio::test]
async fn rejected_field_values_make_the_request_completed_with_errors() {
    let f = fixture(
        true,
        json!([]),
        json!({ "sender": { "id": "psid-1" }, "gender": "robot" }),
    )
    .await;
    f.run().await.expect("run");
    assert_eq!(f.request().await.0, "completedWithErrors");
}

#[tokio::test]
async fn a_stored_result_only_counts_for_the_action_still_at_its_position() {
    let f = fixture(true, json!([add_tag()]), sender()).await;
    // An earlier attempt recorded a different action at index 0.
    f.exec(
        r#"UPDATE "InboundWebhookRequest" SET "actionResults" = '[{"index":0,"type":"removeTag","status":"succeeded"}]'"#,
    )
    .await;
    f.run().await.expect("run");
    assert_eq!(f.tag_links().await, 1);
    assert_eq!(
        f.results().await,
        json!([{ "index": 0, "type": "addTag", "status": "succeeded" }])
    );
}

#[tokio::test]
async fn a_blocked_workspace_closes_the_request_as_skipped() {
    let f = fixture(true, json!([add_tag()]), sender()).await;
    skip_blocked_request(&f.deps, &data()).await.expect("skip");
    let (status, reason, ..) = f.request().await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("skipped", Some("workspaceBlocked"))
    );
    assert_eq!(f.tag_links().await, 0);
}

#[tokio::test]
async fn redispatch_abandons_old_requests_and_requeues_stale_ones() {
    let f = fixture(true, json!([]), sender()).await;
    // Unclaimed for 10 minutes: stale, so re-enqueued.
    f.exec(
        r#"UPDATE "InboundWebhookRequest" SET "createdAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) - 600000"#,
    )
    .await;
    redispatch(&f.deps).await.expect("redispatch");
    let queued: i64 =
        f.db.read(|conn| {
            Ok(conn.query_row(
                r#"SELECT COUNT(*) FROM "jobs" WHERE "name" = 'processInboundWebhookRequest'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap_or_default();
    assert!(queued >= 1, "re-enqueued: {queued}");
    assert_eq!(f.request().await.0, "received");
    // Older than a day: closed as failed (abandoned).
    f.exec(
        r#"UPDATE "InboundWebhookRequest" SET "createdAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) - 90000000"#,
    )
    .await;
    redispatch(&f.deps).await.expect("redispatch");
    assert_eq!(f.request().await.0, "failed");
}
