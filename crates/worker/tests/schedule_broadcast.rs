//! Ports of the broadcast `schedule` job tests of `apps/worker/__tests__`
//! (Q.D5 slice 5): `enqueue-broadcast`, `prepare-broadcast`,
//! `process-broadcast-contacts`, `reconcile-broadcasts`,
//! `finalize-broadcasts`, `purge-broadcasts` and
//! `purge-draft-broadcast-recipients`. Adapted: the mocked
//! `broadcastService` becomes a migrated SQLite database and the queues a
//! recording enqueuer; the assertions are on the rows and the queued jobs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::{
    AddResult, BoxFuture, Enqueuer, JobCtx, JobError, JobOptions, JobsError, NewJob, QueueName,
    Retention,
};
use chatbotx_worker::Deps;
use chatbotx_worker::schedule::broadcast as jobs;
use serde_json::{Value, json};

const WS: i64 = 2001;
const INBOX: i64 = 81;
const FLOW: i64 = 61;
const BROADCAST: i64 = 301;

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
    bulks: Mutex<Vec<Vec<NewJob>>>,
    fail: Mutex<bool>,
}

impl Enqueuer for Recording {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let fail = *self.fail.lock().unwrap();
        if !fail {
            self.calls.lock().unwrap().push(Call {
                queue,
                name: name.into(),
                data,
                opts,
            });
        }
        Box::pin(async move {
            if fail {
                return Err(JobsError::Other("queue down".into()));
            }
            Ok(AddResult {
                job_id: "1".into(),
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        self.bulks.lock().unwrap().push(jobs);
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

impl Fixture {
    fn calls(&self) -> Vec<Call> {
        self.jobs.calls.lock().unwrap().clone()
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

    async fn int(&self, sql: &'static str) -> Option<i64> {
        self.db
            .read(move |conn| {
                Ok(conn
                    .query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
                    .ok()
                    .flatten())
            })
            .await
            .expect("read")
    }

    async fn text(&self, sql: &'static str) -> Option<String> {
        self.db
            .read(move |conn| {
                Ok(conn
                    .query_row(sql, [], |r| r.get::<_, Option<String>>(0))
                    .ok()
                    .flatten())
            })
            .await
            .expect("read")
    }
}

/// A workspace with a webchat inbox (81), a flow (61) and contacts
/// 91..=93 on it (contact inbox `contact + 100`); contacts 91 and 92 have
/// a DM (`contact + 200`).
fn seed(tx: &Connection) -> chatbotx_db::rusqlite::Result<()> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (1001, 'a@example.test')"#,
        [],
    )?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1001)"#,
        [WS],
    )?;
    tx.execute(
        r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Inbox', 'webchat', 'src')"#,
        params![INBOX, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Promo')"#,
        params![FLOW, WS],
    )?;
    for contact in [91, 92, 93] {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
            params![contact, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "sourceId", "channel", "source")
               VALUES (?1, ?2, ?2, ?3, ?4, 'webchat', 'webchat')"#,
            params![contact + 100, contact, INBOX, format!("s{contact}")],
        )?;
    }
    for contact in [91, 92] {
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "contactId", "workspaceId") VALUES (?1, ?2, ?3)"#,
            params![contact + 200, contact, WS],
        )?;
    }
    Ok(())
}

async fn fixture() -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-worker-broadcast-{}-{}",
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
    };
    Fixture {
        db,
        jobs,
        deps,
        dir,
    }
}

/// A broadcast row (`status`, flow 61 on the webchat channel) plus `extra` SQL.
async fn broadcast(f: &Fixture, status: &'static str, extra: &'static str) {
    f.db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "flowId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                   VALUES (?1, 'B', ?2, ?3, ?4, 'now', 0, 'allContacts', 'webchat')"#,
                params![BROADCAST, WS, FLOW, status],
            )?;
            tx.execute_batch(extra)?;
            Ok(())
        })
        .await
        .expect("broadcast");
}

/// Recipients for contacts 91 and 92 (unsent).
const RECIPIENTS: &str = r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId")
    VALUES (301, 91, 191, 291), (301, 92, 192, 292);"#;

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

fn removed() -> Option<Retention> {
    Some(Retention::Remove)
}

// --- enqueue-broadcast.test.ts ---

/// enqueue-broadcast.test.ts:58 "returns { scanned: 0, enqueued: 0 } without calling addBulk"
#[tokio::test]
async fn enqueue_without_due_broadcasts_adds_nothing() {
    let f = fixture().await;
    assert_eq!(
        jobs::enqueue_broadcast(&f.deps).await.unwrap(),
        json!({ "scanned": 0, "enqueued": 0 })
    );
    assert!(f.jobs.bulks.lock().unwrap().is_empty());
}

/// enqueue-broadcast.test.ts:69, :88, :107: one bulk of `prepareBroadcast`
/// jobs, each with the broadcast id, the dedup job id and remove-on-finish.
#[tokio::test]
async fn enqueue_queues_one_prepare_job_per_due_broadcast() {
    let f = fixture().await;
    broadcast(&f, "scheduled", r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
        VALUES (302, 'C', 2001, 'scheduled', 'future', 0, 'allContacts', 'webchat'),
               (303, 'D', 2001, 'draft', 'future', 0, 'allContacts', 'webchat'),
               (304, 'E', 2001, 'scheduled', 'future', 32503680000000, 'allContacts', 'webchat');"#)
    .await;
    assert_eq!(
        jobs::enqueue_broadcast(&f.deps).await.unwrap(),
        json!({ "scanned": 2, "enqueued": 2 })
    );
    let bulks = f.jobs.bulks.lock().unwrap();
    assert_eq!(bulks.len(), 1);
    let ids: Vec<_> = bulks[0]
        .iter()
        .map(|j| j.opts.job_id.clone().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "schedule-prepare-broadcast-301",
            "schedule-prepare-broadcast-302"
        ]
    );
    let first = &bulks[0][0];
    assert_eq!(first.name, "prepareBroadcast");
    assert_eq!(
        first.data,
        json!({ "type": "prepareBroadcast", "data": { "broadcastId": "301" } })
    );
    assert_eq!(
        (first.opts.remove_on_complete, first.opts.remove_on_fail),
        (removed(), removed())
    );
}

/// enqueue-broadcast.test.ts:128 "passes the current startOfMinute as
/// dueAt": a row at the minute's start is due, one a millisecond later waits.
#[tokio::test]
async fn enqueue_uses_the_start_of_the_minute_as_due_at() {
    let f = fixture().await;
    let start = |ms: i64| ms - ms.rem_euclid(60_000);
    let minute = start(chatbotx_db::repo::now_ms());
    broadcast(&f, "scheduled", "").await;
    f.db
        .write(move |tx| {
            tx.execute(r#"UPDATE "Broadcast" SET "schedulesAt" = ?1"#, [minute])?;
            tx.execute(
                r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                   VALUES (302, 'C', 2001, 'scheduled', 'future', ?1, 'allContacts', 'webchat')"#,
                [minute + 1],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let out = jobs::enqueue_broadcast(&f.deps).await.unwrap();
    // A new minute began during the call: both rows are due then.
    if start(chatbotx_db::repo::now_ms()) == minute {
        assert_eq!(out, json!({ "scanned": 1, "enqueued": 1 }));
    }
}

/// enqueue-broadcast.test.ts:139, :154: chunks of 500 (1001 rows → 500, 500, 1).
#[tokio::test]
async fn enqueue_splits_into_chunks_of_500() {
    let f = fixture().await;
    f.db
        .write(|tx| {
            for id in 1..=1001 {
                tx.execute(
                    r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                       VALUES (?1, 'B', 2001, 'scheduled', 'now', 0, 'allContacts', 'webchat')"#,
                    [id],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    jobs::enqueue_broadcast(&f.deps).await.unwrap();
    let sizes: Vec<usize> = f.jobs.bulks.lock().unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, [500, 500, 1]);
}

// --- prepare-broadcast.test.ts ---

/// prepare-broadcast.test.ts:115 "returns without db writes or queue enqueues when the broadcast is missing"
#[tokio::test]
async fn prepare_skips_a_missing_or_unscheduled_broadcast() {
    let f = fixture().await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    broadcast(&f, "draft", "").await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("draft")
    );
    assert!(f.calls().is_empty());
}

/// prepare-broadcast.test.ts:126 "returns without db writes or queue enqueues when the workspace is frozen"
#[tokio::test]
async fn prepare_skips_a_frozen_workspace() {
    let f = fixture().await;
    broadcast(
        &f,
        "scheduled",
        r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1;"#,
    )
    .await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("scheduled")
    );
    assert!(f.calls().is_empty());
}

/// prepare-broadcast.test.ts:328, :682, :725: one recipient per DM, the
/// row promoted to `sending` with the count, one `sendBroadcast` job.
#[tokio::test]
async fn prepare_writes_the_audience_and_queues_the_send() {
    let f = fixture().await;
    broadcast(&f, "scheduled", "").await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.text(r#"SELECT group_concat("contactId" || ':' || "contactInboxId" || ':' || "conversationId", ',') FROM "ContactOnBroadcast""#)
            .await
            .as_deref(),
        Some("91:191:291,92:192:292")
    );
    assert_eq!(
        f.text(r#"SELECT "status" || ':' || "contactCount" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("sending:2")
    );
    let calls = f.calls();
    assert_eq!(calls.len(), 1);
    let send = &calls[0];
    assert_eq!(
        (send.queue, send.name.as_str()),
        (QueueName::Schedule, "sendBroadcast")
    );
    assert_eq!(
        send.data,
        json!({ "type": "sendBroadcast", "data": { "broadcastId": "301" } })
    );
    assert_eq!(send.opts.job_id.as_deref(), Some("broadcast-send-301"));
    assert_eq!(
        (
            send.opts.attempts,
            send.opts.remove_on_complete,
            send.opts.remove_on_fail
        ),
        (Some(1), removed(), removed())
    );
}

/// prepare-broadcast.test.ts:168 "passes the resolved audience range ...":
/// positions 2..=2 reach the second contact inbox only.
#[tokio::test]
async fn prepare_keeps_to_the_audience_range() {
    let f = fixture().await;
    broadcast(
        &f,
        "scheduled",
        r#"UPDATE "Broadcast" SET "audienceRangeStart" = 2, "audienceRangeEnd" = 2;"#,
    )
    .await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.text(r#"SELECT group_concat("contactId") FROM "ContactOnBroadcast""#)
            .await
            .as_deref(),
        Some("92")
    );
    assert_eq!(
        f.int(r#"SELECT "contactCount" FROM "Broadcast""#).await,
        Some(1)
    );
}

/// prepare-broadcast.test.ts:290 "scopes a targets-mode broadcast whose
/// pages were all deleted to nobody, never the whole channel", and :635
/// "marks sent with contactCount zero and does not enqueue when the audience is empty".
#[tokio::test]
async fn prepare_of_a_targets_broadcast_without_pages_reaches_nobody() {
    let f = fixture().await;
    broadcast(
        &f,
        "scheduled",
        r#"UPDATE "Broadcast" SET "targetMode" = 'targets';"#,
    )
    .await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
    assert_eq!(
        f.text(r#"SELECT "status" || ':' || "contactCount" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("sent:0")
    );
    assert!(f.calls().is_empty());
}

/// prepare-broadcast.test.ts:197 "loads the target rows and forwards their inbox ids for a multi-page broadcast"
#[tokio::test]
async fn prepare_of_a_targets_broadcast_reaches_its_pages_only() {
    let f = fixture().await;
    broadcast(&f, "scheduled", r#"
        INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (82, 2001, 'Other', 'webchat', 'src2');
        UPDATE "ContactInbox" SET "inboxId" = 82 WHERE "id" = 192;
        UPDATE "Broadcast" SET "targetMode" = 'targets';
        INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId", "flowId") VALUES (301, 82, 61);"#).await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.text(r#"SELECT group_concat("contactId") FROM "ContactOnBroadcast""#)
            .await
            .as_deref(),
        Some("92")
    );
}

/// prepare-broadcast.test.ts:307 "fails closed for invalid persisted channel and subaction values"
#[tokio::test]
async fn prepare_fails_closed_for_an_unknown_channel() {
    let f = fixture().await;
    broadcast(
        &f,
        "scheduled",
        r#"UPDATE "Broadcast" SET "channel" = 'bogus';"#,
    )
    .await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("sent")
    );
}

/// prepare-broadcast.test.ts:652 "counts the rows a failed earlier run of
/// this prepare already wrote" and :386 "counts the recipients stored, not
/// the audience rows offered".
#[tokio::test]
async fn prepare_counts_rows_an_earlier_run_wrote_once() {
    let f = fixture().await;
    broadcast(&f, "scheduled", r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId") VALUES (301, 91, 191, 291);"#).await;
    jobs::prepare_broadcast(&f.deps, BROADCAST).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(2)
    );
    assert_eq!(
        f.int(r#"SELECT "contactCount" FROM "Broadcast""#).await,
        Some(2)
    );
}

/// prepare-broadcast.test.ts:328 (the "created on demand" part):
/// `findOrCreateDMByContactIds` opens the missing DM once.
#[tokio::test]
async fn find_or_create_dms_opens_a_missing_dm_once() {
    let f = fixture().await;
    let send = chatbotx_business::broadcast::send::find_or_create_dms;
    let dms = send(&f.db, WS, vec![91, 93, 93]).await.unwrap();
    let mut contacts: Vec<i64> = dms.iter().map(|c| c.contact_id).collect();
    contacts.sort_unstable();
    assert_eq!(contacts, [91, 93]);
    send(&f.db, WS, vec![93]).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "Conversation" WHERE "contactId" = 93"#)
            .await,
        Some(1)
    );
}

/// prepare-broadcast.test.ts:516, :558, :699 (the epoch guard): a stale
/// run neither keeps preparing nor promotes a newer schedule's row.
#[tokio::test]
async fn the_promotion_is_pinned_to_the_dispatch_epoch() {
    use chatbotx_business::broadcast::send;
    let f = fixture().await;
    broadcast(
        &f,
        "scheduled",
        r#"UPDATE "Broadcast" SET "resumeCount" = 1;"#,
    )
    .await;
    assert!(!send::is_still_preparing(&f.db, BROADCAST, 0).await.unwrap());
    assert!(send::is_still_preparing(&f.db, BROADCAST, 1).await.unwrap());
    assert!(
        !send::promote_after_prepare(&f.db, BROADCAST, "sending", 2, 0)
            .await
            .unwrap()
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("scheduled")
    );
    assert!(
        send::promote_after_prepare(&f.db, BROADCAST, "sending", 2, 1)
            .await
            .unwrap()
    );
}

/// prepare-broadcast.test.ts:222 "derives Messenger template integration
/// id": the template's page, scoped to the workspace.
#[tokio::test]
async fn the_template_page_is_scoped_to_the_workspace() {
    use chatbotx_business::broadcast::send;
    let f = fixture().await;
    f.exec(r#"
        INSERT INTO "User" ("id", "email") VALUES (1002, 'b@example.test');
        INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2002, 'Other', 1002);
        INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (75, 2001, 'Page', 'messenger', 's75');
        INSERT INTO "IntegrationMessenger" ("id", "workspaceId", "inboxId", "name", "pageId", "auth", "conversationStarters", "persistentMenus", "personas")
            VALUES (76, 2001, 75, 'Page', 'p', '{}', '[]', '[]', '[]');
        INSERT INTO "MessengerMessageTemplate" ("id", "integrationMessengerId", "name", "sourceId", "language", "category", "status")
            VALUES (77, 76, 'T', 's', 'en', 'UTILITY', 'APPROVED');"#).await;
    assert_eq!(
        send::resolve_template_integration_messenger_id(&f.db, WS, 77)
            .await
            .unwrap(),
        Some(76)
    );
    assert_eq!(
        send::resolve_template_integration_messenger_id(&f.db, 2002, 77)
            .await
            .unwrap(),
        None
    );
}

// --- process-broadcast-contacts.test.ts ---

/// process-broadcast-contacts.test.ts:175 "returns { processed: 0 } without any db updates or queue adds"
#[tokio::test]
async fn send_without_a_sending_broadcast_does_nothing() {
    let f = fixture().await;
    broadcast(&f, "scheduled", RECIPIENTS).await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert!(f.calls().is_empty());
}

/// process-broadcast-contacts.test.ts:201 "returns { processed: 0 } and enqueues nothing when the owner is blocked"
#[tokio::test]
async fn send_for_a_blocked_workspace_does_nothing() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1;"#)
        .await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert!(f.calls().is_empty());
    assert_eq!(
        f.int(r#"SELECT "handoffCompletedAt" FROM "Broadcast""#)
            .await,
        None
    );
}

/// process-broadcast-contacts.test.ts:219 "stamps hand-off completion instead of a terminal status"
#[tokio::test]
async fn send_with_no_pending_recipient_stamps_the_hand_off() {
    let f = fixture().await;
    broadcast(&f, "sending", "").await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert!(
        f.int(r#"SELECT "handoffCompletedAt" FROM "Broadcast""#)
            .await
            .is_some()
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("sending")
    );
}

/// process-broadcast-contacts.test.ts:235, :268, :528, :546, :591, :975:
/// one `sendFlow` job per recipient (no chat job), each marked sent, and the
/// hand-off stamped for a partial batch.
#[tokio::test]
async fn send_hands_each_recipient_to_its_flow_job() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 2 })
    );
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    let flow = &calls[0];
    assert_eq!(
        (flow.queue, flow.name.as_str()),
        (QueueName::Integration, "sendFlow")
    );
    assert_eq!(
        flow.data,
        json!({ "type": "sendFlow", "data": {
            "flowId": "61", "conversationId": "291", "contactInboxId": "191",
            "initialBroadcastDispatch": true,
            "metadata": { "type": "broadcast", "broadcastId": "301", "contactInboxId": "191" },
        }})
    );
    assert_eq!(
        flow.opts.job_id.as_deref(),
        Some("broadcast-send-contact-301-91-flow-r0")
    );
    assert_eq!(flow.opts.priority, Some(5));
    assert_eq!(
        flow.opts.remove_on_complete,
        Some(Retention::KeepAge {
            age_secs: 3600,
            count: Some(100_000)
        })
    );
    assert!(
        calls
            .iter()
            .all(|c| !c.opts.job_id.clone().unwrap().contains(':'))
    );
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "sent" = 1"#)
            .await,
        Some(2)
    );
    assert!(
        f.int(r#"SELECT "handoffCompletedAt" FROM "Broadcast""#)
            .await
            .is_some()
    );
}

/// process-broadcast-contacts.test.ts:473, :486, :567: the broadcast's own
/// send rate is the batch; a full batch leaves the hand-off to the cron.
#[tokio::test]
async fn a_full_batch_of_the_send_rate_leaves_the_rest_to_the_cron() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "sendRatePerMinute" = 1;"#)
        .await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 1 })
    );
    assert_eq!(
        f.int(r#"SELECT "handoffCompletedAt" FROM "Broadcast""#)
            .await,
        None
    );
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "sent" = 0"#)
            .await,
        Some(1)
    );
}

/// process-broadcast-contacts.test.ts:458 "fetches only unsent contacts that are not terminal-failed"
#[tokio::test]
async fn send_skips_sent_and_failed_recipients() {
    let f = fixture().await;
    broadcast(&f, "sending", r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "failedAt")
        VALUES (301, 91, 191, 291, 1, NULL), (301, 92, 192, 292, 0, 5);"#).await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert!(f.calls().is_empty());
}

/// process-broadcast-contacts.test.ts:305, :343: a WhatsApp template job
/// carries the conversation, the contact inbox and the stored data unchanged.
#[tokio::test]
async fn send_hands_a_whatsapp_template_to_the_chat_queue() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(
        r#"UPDATE "Broadcast" SET "flowId" = NULL, "channel" = 'whatsapp', "templateId" = 73,
        "templateData" = '{"header":{"type":"text"},"paymentRequest":{"amount":5}}';"#,
    )
    .await;
    jobs::process_broadcast_contacts(&f.deps, BROADCAST)
        .await
        .unwrap();
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    let chat = &calls[0];
    assert_eq!(
        (chat.queue, chat.name.as_str()),
        (QueueName::Chat, "sendWhatsappTemplateMessage")
    );
    let data = &chat.data["data"];
    assert_eq!(data["templateId"], json!("73"));
    assert_eq!(data["broadcastId"], json!("301"));
    assert_eq!(
        data["templateData"],
        json!({ "header": { "type": "text" }, "paymentRequest": { "amount": 5 } })
    );
    assert_eq!(data["conversation"]["id"], json!("291"));
    assert_eq!(data["contactInbox"]["id"], json!("191"));
    assert_eq!(
        data["metadata"],
        json!({ "type": "broadcast", "broadcastId": "301", "contactInboxId": "191" })
    );
    assert_eq!(
        chat.opts.job_id.as_deref(),
        Some("broadcast-send-contact-301-91-template-r0")
    );
}

/// process-broadcast-contacts.test.ts:377, :401, :423: a Messenger template
/// job gets the buttons apart, and no `templateData` without other fields.
#[tokio::test]
async fn send_separates_messenger_buttons_from_the_template_data() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(
        r#"UPDATE "Broadcast" SET "flowId" = NULL, "channel" = 'messenger', "templateId" = 77,
        "templateData" = '{"buttons":[{"id":"b1","label":"Go"}]}';"#,
    )
    .await;
    jobs::process_broadcast_contacts(&f.deps, BROADCAST)
        .await
        .unwrap();
    let chat = &f.calls()[0];
    assert_eq!(chat.name, "sendMessengerTemplateMessage");
    assert_eq!(
        chat.data["data"]["buttons"],
        json!([{ "id": "b1", "label": "Go" }])
    );
    assert!(chat.data["data"].get("templateData").is_none());
    f.exec(r#"UPDATE "ContactOnBroadcast" SET "sent" = 0; UPDATE "Broadcast" SET "templateData" = '{"name":"x","buttons":[]}';"#).await;
    f.exec(r#"DELETE FROM "kv_entry";"#).await;
    jobs::process_broadcast_contacts(&f.deps, BROADCAST)
        .await
        .unwrap();
    let chat = f.calls().pop().unwrap();
    assert_eq!(chat.data["data"]["templateData"], json!({ "name": "x" }));
    assert_eq!(chat.data["data"]["buttons"], json!([]));
}

/// process-broadcast-contacts.test.ts:664, :756, :788, :813, :855: each page
/// sends its own flow; a page without one fails with the original reason.
#[tokio::test]
async fn a_targets_broadcast_sends_each_page_its_own_flow() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"
        INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (82, 2001, 'Other', 'webchat', 'src2');
        INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (62, 2001, 'Other flow');
        UPDATE "ContactInbox" SET "inboxId" = 82 WHERE "id" = 192;
        UPDATE "Broadcast" SET "targetMode" = 'targets', "flowId" = NULL;
        INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId", "flowId") VALUES (301, 81, 62), (301, 82, NULL);"#).await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 1 })
    );
    let calls = f.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].data["data"]["flowId"], json!("62"));
    assert_eq!(
        f.text(r#"SELECT "errorContent" FROM "ContactOnBroadcast" WHERE "contactId" = 92 AND "failedAt" IS NOT NULL AND "sent" = 0"#).await.as_deref(),
        Some("no flow selected for the contact's page")
    );
}

/// process-broadcast-contacts.test.ts:788 "fails every contact of a
/// targets-mode broadcast whose target rows are gone".
#[tokio::test]
async fn a_targets_broadcast_without_rows_fails_every_contact() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "targetMode" = 'targets';"#)
        .await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert!(f.calls().is_empty());
    assert_eq!(
        f.text(r#"SELECT group_concat(DISTINCT "errorContent") FROM "ContactOnBroadcast" WHERE "failedAt" IS NOT NULL"#).await.as_deref(),
        Some("no flow selected for the contact's page")
    );
}

/// process-broadcast-contacts.test.ts:612, :1078: a failed queue add
/// rejects the run, marks nothing failed or sent, stamps no hand-off.
#[tokio::test]
async fn a_failed_queue_add_rejects_the_batch() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    *f.jobs.fail.lock().unwrap() = true;
    assert!(matches!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST).await,
        Err(JobError::Retryable(_))
    ));
    assert_eq!(f.int(r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "sent" = 1 OR "failedAt" IS NOT NULL"#).await, Some(0));
    assert_eq!(
        f.int(r#"SELECT "handoffCompletedAt" FROM "Broadcast""#)
            .await,
        None
    );
}

/// process-broadcast-contacts.test.ts:1100, :1129: job ids carry the
/// dispatch epoch.
#[tokio::test]
async fn downstream_job_ids_carry_the_resume_count() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "resumeCount" = 2;"#).await;
    jobs::process_broadcast_contacts(&f.deps, BROADCAST)
        .await
        .unwrap();
    assert_eq!(
        f.calls()[0].opts.job_id.as_deref(),
        Some("broadcast-send-contact-301-91-flow-r2")
    );
}

/// process-broadcast-contacts.test.ts:1156, :1170, :1196: a second run
/// inside the dispatch window hands off nothing.
#[tokio::test]
async fn the_dispatch_window_refuses_a_second_batch() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "sendRatePerMinute" = 1;"#)
        .await;
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 1 })
    );
    assert_eq!(
        jobs::process_broadcast_contacts(&f.deps, BROADCAST)
            .await
            .unwrap(),
        json!({ "processed": 0 })
    );
    assert_eq!(f.calls().len(), 1);
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "sent" = 0"#)
            .await,
        Some(1)
    );
}

/// process-broadcast-contacts.test.ts:528 (the service guard):
/// `markContactSentIfSending` does not touch a row once the broadcast left `sending`.
#[tokio::test]
async fn a_recipient_is_marked_sent_only_while_sending() {
    use chatbotx_business::broadcast::send;
    let f = fixture().await;
    broadcast(&f, "cancelled", RECIPIENTS).await;
    send::mark_contact_sent_if_sending(&f.db, BROADCAST, 91)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "sent" = 1"#)
            .await,
        Some(0)
    );
}

// --- reconcile-broadcasts.test.ts ---

/// reconcile-broadcasts.test.ts:67, :105, :112: one `sendBroadcast` per
/// sending row awaiting its hand-off; ids without ':'.
#[tokio::test]
async fn reconcile_revives_the_send_of_each_sending_broadcast() {
    let f = fixture().await;
    assert_eq!(
        jobs::reconcile_broadcasts(&f.deps).await.unwrap(),
        json!({ "reconciled": 0 })
    );
    assert!(f.calls().is_empty());
    broadcast(&f, "sending", r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "handoffCompletedAt")
        VALUES (302, 'C', 2001, 'sending', 'now', 0, 'allContacts', 'webchat', 5);"#).await;
    assert_eq!(
        jobs::reconcile_broadcasts(&f.deps).await.unwrap(),
        json!({ "reconciled": 1 })
    );
    let call = &f.calls()[0];
    assert_eq!(
        (call.queue, call.name.as_str()),
        (QueueName::Schedule, "sendBroadcast")
    );
    assert_eq!(
        call.data,
        json!({ "type": "sendBroadcast", "data": { "broadcastId": "301" } })
    );
    assert_eq!(call.opts.job_id.as_deref(), Some("broadcast-send-301"));
    assert_eq!(call.opts.attempts, Some(1));
}

// --- finalize-broadcasts.test.ts ---

async fn finalize_case(
    f: &Fixture,
    contact_count: Option<i64>,
    outcomes: &'static str,
    handoff_age_ms: i64,
) -> String {
    let at = chatbotx_db::repo::now_ms() - handoff_age_ms;
    broadcast(f, "sending", outcomes).await;
    f.db.write(move |tx| {
        tx.execute(
            r#"UPDATE "Broadcast" SET "contactCount" = ?1, "handoffCompletedAt" = ?2"#,
            params![contact_count, at],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    jobs::finalize_broadcasts(&f.deps).await.unwrap();
    f.text(r#"SELECT "status" FROM "Broadcast""#).await.unwrap()
}

/// finalize-broadcasts.test.ts:89 "returns zero counts when nothing awaits finalization"
#[tokio::test]
async fn finalize_without_candidates_reports_zero() {
    let f = fixture().await;
    assert_eq!(
        jobs::finalize_broadcasts(&f.deps).await.unwrap(),
        json!({ "skipped": false, "finalized": 0, "failed": 0 })
    );
}

/// finalize-broadcasts.test.ts:98 "marks null/zero-contact handed-off broadcasts sent without counting outcomes"
#[tokio::test]
async fn finalize_marks_a_contactless_broadcast_sent() {
    let f = fixture().await;
    assert_eq!(finalize_case(&f, None, "", 0).await, "sent");
}

/// finalize-broadcasts.test.ts:118 "marks sent when every recipient has an outcome and not all failed"
#[tokio::test]
async fn finalize_marks_sent_when_every_recipient_has_an_outcome() {
    let f = fixture().await;
    let outcomes = r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "deliveredAt", "failedAt")
        VALUES (301, 91, 191, 291, 1, 5, NULL), (301, 92, 192, 292, 1, NULL, 5);"#;
    assert_eq!(finalize_case(&f, Some(2), outcomes, 0).await, "sent");
}

/// finalize-broadcasts.test.ts:140 "marks failed when every recipient failed"
#[tokio::test]
async fn finalize_marks_failed_when_every_recipient_failed() {
    let f = fixture().await;
    let outcomes = r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "failedAt")
        VALUES (301, 91, 191, 291, 1, 5), (301, 92, 192, 292, 1, 5);"#;
    assert_eq!(finalize_case(&f, Some(2), outcomes, 0).await, "failed");
}

/// finalize-broadcasts.test.ts:180 "waits inside the grace window when outcomes are incomplete"
/// and :192 "resolves with whatever is known once the grace window elapsed".
#[tokio::test]
async fn finalize_waits_for_outcomes_inside_the_grace_window() {
    let f = fixture().await;
    let outcomes = r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "failedAt")
        VALUES (301, 91, 191, 291, 1, 5), (301, 92, 192, 292, 1, NULL);"#;
    assert_eq!(finalize_case(&f, Some(2), outcomes, 0).await, "sending");
    f.exec(r#"UPDATE "Broadcast" SET "handoffCompletedAt" = 1;"#)
        .await;
    jobs::finalize_broadcasts(&f.deps).await.unwrap();
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("sent")
    );
}

/// finalize-broadcasts.test.ts:219 "does not count a lost race": a row no
/// longer `sending` is not finalized again.
#[tokio::test]
async fn finalize_does_not_touch_a_row_that_left_sending() {
    use chatbotx_business::broadcast::send;
    let f = fixture().await;
    broadcast(
        &f,
        "cancelled",
        r#"UPDATE "Broadcast" SET "handoffCompletedAt" = 1;"#,
    )
    .await;
    assert!(
        !send::complete_sending(&f.db, BROADCAST, "sent")
            .await
            .unwrap()
    );
    assert_eq!(
        f.text(r#"SELECT "status" FROM "Broadcast""#)
            .await
            .as_deref(),
        Some("cancelled")
    );
}

// --- purge-broadcasts.test.ts ---

/// purge-broadcasts.test.ts:133 "claims candidates under the lock and
/// hard-deletes a drained broadcast with no remaining recipients", and :242.
#[tokio::test]
async fn purge_hard_deletes_a_drained_soft_deleted_broadcast() {
    let f = fixture().await;
    jobs::purge_broadcasts(&f.deps).await.unwrap();
    broadcast(&f, "sent", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "deletedAt" = 1;"#).await;
    jobs::purge_broadcasts(&f.deps).await.unwrap();
    assert_eq!(f.int(r#"SELECT count(*) FROM "Broadcast""#).await, Some(0));
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
}

/// purge-broadcasts.test.ts (the candidate rule): a live or a sending
/// deleted broadcast is kept.
#[tokio::test]
async fn purge_keeps_live_and_sending_broadcasts() {
    let f = fixture().await;
    broadcast(&f, "sending", RECIPIENTS).await;
    f.exec(r#"UPDATE "Broadcast" SET "deletedAt" = 1;"#).await;
    jobs::purge_broadcasts(&f.deps).await.unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(2)
    );
}

// --- purge-draft-broadcast-recipients.test.ts ---

fn purge_job(attempts_started: u32, token: bool) -> JobCtx {
    let mut ctx = job(
        json!({ "type": "purgeDraftBroadcastRecipients", "data": { "workspaceId": "2001", "broadcastId": "301" } }),
    );
    ctx.attempts_started = attempts_started;
    ctx.lease_token = token.then(|| "token".into());
    ctx
}

/// purge-draft-broadcast-recipients.test.ts:46 "finishes once the draft holds nothing"
#[tokio::test]
async fn the_draft_purge_finishes_once_drained() {
    let f = fixture().await;
    broadcast(&f, "draft", RECIPIENTS).await;
    jobs::purge_draft_broadcast_recipients(&f.deps, &purge_job(1, true), WS, BROADCAST)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(0)
    );
}

/// purge-draft-broadcast-recipients.test.ts:123 "is a no-op for a blocked owner"
#[tokio::test]
async fn the_draft_purge_is_a_no_op_for_a_blocked_workspace() {
    let f = fixture().await;
    broadcast(&f, "draft", RECIPIENTS).await;
    f.exec(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1;"#)
        .await;
    jobs::purge_draft_broadcast_recipients(&f.deps, &purge_job(1, true), WS, BROADCAST)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(2)
    );
}

/// purge-draft-broadcast-recipients.test.ts:64, :90, :108: an undrained
/// pass parks the job 5 s, stops after the last pass, and cannot park
/// without the lease token. Adapted: the decision is tested alone (an
/// undrained pass needs more rows than a 30 s pass deletes).
#[test]
fn an_undrained_draft_purge_parks_until_the_last_pass() {
    assert!(matches!(
        jobs::draft_purge_park(BROADCAST, 0, true, 1000),
        Err(JobError::Delayed { run_at_ms: 6000 })
    ));
    assert!(matches!(
        jobs::draft_purge_park(BROADCAST, jobs::PURGE_DRAFT_MAX_PASSES - 1, true, 0),
        Err(JobError::Delayed { .. })
    ));
    assert_eq!(
        jobs::draft_purge_park(BROADCAST, jobs::PURGE_DRAFT_MAX_PASSES, true, 0).unwrap(),
        Value::Null
    );
    assert_eq!(
        jobs::draft_purge_park(BROADCAST, 0, false, 0).unwrap(),
        Value::Null
    );
}

/// purge-draft-broadcast-recipients.test.ts:46 (the draft guard): a row
/// that is no longer a draft keeps its recipients.
#[tokio::test]
async fn the_draft_purge_keeps_the_rows_of_a_non_draft() {
    let f = fixture().await;
    broadcast(&f, "scheduled", RECIPIENTS).await;
    jobs::purge_draft_broadcast_recipients(&f.deps, &purge_job(1, true), WS, BROADCAST)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT count(*) FROM "ContactOnBroadcast""#).await,
        Some(2)
    );
}

// --- schedule worker routing ---

/// schedule/worker.ts switch: an unported schedule job fails unrecoverably.
#[tokio::test]
async fn an_unported_schedule_job_is_refused() {
    let f = fixture().await;
    let out = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "scanAppointmentReminders", "data": {} })),
    )
    .await;
    assert!(matches!(out, Err(JobError::Unrecoverable(_))));
    let out = chatbotx_worker::schedule::handle(
        f.deps.clone(),
        job(json!({ "type": "reconcileBroadcasts", "data": {} })),
    )
    .await;
    assert_eq!(out.unwrap(), json!({ "reconciled": 0 }));
}
