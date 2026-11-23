//! Ports of `apps/worker/__tests__/scan-abandoned-carts.test.ts`,
//! `purge-workspaces.test.ts` and the repair job of `send-email.test.ts`
//! (`recordEmailTrackingTransition`) (Q.P6-U5). Adapted: the mocked
//! `webshopService` / `workspaceService` become real rows; the distributed
//! lock is gone (one worker process); "reminds" = the `sendFlow` job of the
//! store's abandoned-cart flow.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobCtx, JobOptions, JobsError, NewJob, QueueName,
};
use chatbotx_worker::Deps;
use serde_json::{Value, json};

const WS: i64 = 2001;
/// Scheduled for deletion later (blocked, not due).
const BLOCKED_WS: i64 = 2002;
/// Scheduled for deletion in the past (due for the purge).
const DUE_WS: i64 = 2003;
const FLOW: i64 = 61;

#[derive(Debug, Clone)]
struct Call {
    queue: QueueName,
    name: String,
    data: Value,
    opts: JobOptions,
}

#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<Call>>,
}

impl Enqueuer for Recording {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        self.calls.lock().unwrap().push(Call {
            queue,
            name: name.into(),
            data,
            opts,
        });
        Box::pin(async {
            Ok(AddResult {
                job_id: "1".into(),
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct Fixture {
    db: Arc<Db>,
    jobs: Arc<Recording>,
    deps: Deps,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn seed(tx: &Connection) -> chatbotx_db::rusqlite::Result<()> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (1, 'owner@example.test')"#,
        [],
    )?;
    let far = 4_000_000_000_000_i64;
    for (ws, deletion) in [(WS, None), (BLOCKED_WS, Some(far)), (DUE_WS, Some(1))] {
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "scheduledDeletionAt") VALUES (?1, 'W', 1, ?2)"#,
            params![ws, deletion],
        )?;
    }
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Come back')"#,
        params![FLOW, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (81, ?1, 'Chat', 'webchat', 'wc')"#,
        [WS],
    )?;
    for (ws, flow) in [(WS, Some(FLOW)), (BLOCKED_WS, None)] {
        tx.execute(
            r#"INSERT INTO "EcommerceSettings" ("id", "workspaceId", "orderingAvailable", "webshopProductView", "searchProducts",
                 "abandonedCartDelayMinutes", "abandonedCartFlowId", "productButtons", "productImageSize", "allowRateOrders",
                 "notifyOnCartStatusChange", "paymentMethods", "currency", "currencyPosition", "serviceFeePercent",
                 "shippingType", "shippingCountries", "deliveryCost", "checkoutMode", "minimumOrderValue",
                 "requireName", "requireEmail", "requirePhone", "requireState", "requireZip")
               VALUES (?1, ?1, 1, 'list', 1, 5, ?2, '[]', 'bigSquare', 0, 0, '[]', 'USD', 'left', 0,
                 'none', '[]', 0, 'default', 0, 0, 0, 0, 0, 0)"#,
            params![ws, flow],
        )?;
        tx.execute(
            r#"INSERT INTO "Product" ("id", "workspaceId", "name", "price") VALUES (?1, ?1, 'Mug', 10)"#,
            [ws],
        )?;
    }
    for (contact, ws) in [(91, WS), (92, WS), (93, BLOCKED_WS)] {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
            params![contact, ws],
        )?;
    }
    tx.execute(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "sourceId", "channel", "source")
           VALUES (191, 91, 91, 81, 's91', 'webchat', 'webchat')"#,
        [],
    )?;
    tx.execute(
        r#"INSERT INTO "Conversation" ("id", "contactId", "workspaceId") VALUES (291, 91, ?1)"#,
        [WS],
    )?;
    // Contact 91: left 10 minutes ago (due); 92: 1 minute ago (not yet);
    // 93: due, but its workspace is blocked.
    let now = chatbotx_db::repo::now_ms();
    for (cart, contact, ws, inbox, left) in [
        (701, 91, WS, Some(191), now - 600_000),
        (702, 92, WS, None, now - 60_000),
        (703, 93, BLOCKED_WS, None, now - 600_000),
    ] {
        tx.execute(
            r#"INSERT INTO "Cart" ("id", "workspaceId", "contactId", "contactInboxId", "items", "lastActivityAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                cart,
                ws,
                contact,
                inbox,
                json!([{ "productId": ws.to_string(), "variantId": null, "quantity": 2 }]).to_string(),
                left
            ],
        )?;
    }
    Ok(())
}

async fn fixture() -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-housekeeping-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    db.write(|tx| Ok(seed(tx)?)).await.expect("seed");
    let store = Arc::new(
        chatbotx_jobs::JobStore::open(db.clone(), Arc::new(chatbotx_jobs::SystemClock))
            .await
            .expect("store"),
    );
    let jobs = Arc::new(Recording::default());
    let deps = Deps {
        db: db.clone(),
        jobs: jobs.clone(),
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
    Fixture {
        db,
        jobs,
        deps,
        dir,
    }
}

fn job(data: Value) -> JobCtx {
    JobCtx {
        id: 1,
        job_id: "j".into(),
        name: data["type"].as_str().unwrap_or("").into(),
        data,
        attempts: None,
        attempts_made: 0,
        attempts_started: 1,
        priority: 0,
        lease_token: Some("token".into()),
    }
}

fn send_flows(f: &Fixture) -> Vec<Call> {
    f.jobs
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.name == "sendFlow")
        .cloned()
        .collect()
}

async fn notified(f: &Fixture) -> Vec<i64> {
    f.db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "id" FROM "Cart" WHERE "abandonedNotifiedAt" IS NOT NULL ORDER BY "id""#,
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    })
    .await
    .unwrap()
}

// ---- scan-abandoned-carts.test.ts ---------------------------------------------

#[tokio::test]
async fn reminds_every_due_cart_once_and_skips_a_blocked_owners_cart() {
    let f = fixture().await;
    let out = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "scanAbandonedCarts", "data": {} })),
    )
    .await
    .unwrap();
    assert_eq!(out, json!({ "reminded": 1 }));
    // The blocked owner's cart is claimed too (and skipped).
    assert_eq!(notified(&f).await, [701, 703]);
    let flows = send_flows(&f);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].queue, QueueName::Integration);
    assert_eq!(
        flows[0].data["data"],
        json!({ "conversationId": "291", "contactInboxId": "191", "flowId": FLOW.to_string() })
    );
    assert!(
        flows[0]
            .opts
            .job_id
            .as_deref()
            .unwrap()
            .starts_with("abandoned-cart-flow-701-")
    );
    // A second run finds nothing new.
    let again = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "scanAbandonedCarts", "data": {} })),
    )
    .await
    .unwrap();
    assert_eq!(again, json!({ "reminded": 0 }));
    assert_eq!(send_flows(&f).len(), 1);
}

// ---- purge-workspaces.test.ts ---------------------------------------------------

#[tokio::test]
async fn purges_the_due_workspaces_only() {
    let f = fixture().await;
    chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "purgeWorkspaces", "data": {} })),
    )
    .await
    .unwrap();
    let left: Vec<i64> =
        f.db.read(|conn| {
            let mut stmt = conn.prepare(r#"SELECT "id" FROM "Workspace" ORDER BY "id""#)?;
            let ids = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<i64>, _>>()?;
            Ok(ids)
        })
        .await
        .unwrap();
    assert_eq!(left, [WS, BLOCKED_WS]);
    assert!(
        chatbotx_worker::schedule::PORTED.contains(&"purgeWorkspaces")
            && chatbotx_worker::schedule::PORTED.contains(&"scanAbandonedCarts")
    );
}

// ---- send-email.test.ts (the repair job) ----------------------------------------

#[tokio::test]
async fn the_repair_job_retries_only_the_tracking_transition() {
    use chatbotx_business::email_tracking::{NewRecipient, create_recipient};
    let f = fixture().await;
    let token = create_recipient(
        &f.db,
        NewRecipient {
            workspace_id: WS,
            email: "ada@example.com".into(),
            contact_id: Some(91),
            ..NewRecipient::default()
        },
    )
    .await
    .unwrap();
    let repair = |phase: &str| {
        job(
            json!({ "type": "recordEmailTrackingTransition", "data": { "phase": phase, "token": token, "workspaceId": WS.to_string() } }),
        )
    };
    chatbotx_worker::integration::handle(f.deps.clone(), repair("delivered"))
        .await
        .unwrap();
    chatbotx_worker::integration::handle(f.deps.clone(), repair("opened"))
        .await
        .unwrap();
    let t = token.clone();
    let row: (bool, i64) = f
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "deliveredAt" IS NOT NULL, "seenCount" FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
                [t],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(row, (true, 1));
    // A topic-less email awards no leaderboard and sends nothing.
    assert!(f.jobs.calls.lock().unwrap().is_empty());
    // A bad phase is not retried.
    let bad = chatbotx_worker::integration::handle(f.deps.clone(), repair("bounced")).await;
    assert!(bad.is_err());
}
