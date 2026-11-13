//! P3.6-X1 own tests (not ported, not locked): the SQLite parts of the
//! event bus and the audit writer, and the bus fan-out rules.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, SYSTEM_ACTOR, with_audit_context};
use chatbotx_business::events::cache::ActiveCache;
use chatbotx_business::events::emitter::QueueEmitter;
use chatbotx_business::events::trigger::tag_applied;
use chatbotx_business::events::{
    BoxFuture, Event, EventBus, RecordingSubscriber, Subscriber, SubscriberError,
};
use chatbotx_db::Db;
use common::{scalar_i64, scalar_text, setup};
use x1_support::RecordingEnqueuer;

/// A workspace (10, owner 1) with an active trigger on `tagApplied` / tag-7,
/// an inactive one on `tagRemoved`, and a webhook on `newContact`.
struct Seeded {
    db: Arc<Db>,
    dir: std::path::PathBuf,
}

impl Drop for Seeded {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn seeded() -> Seeded {
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-x1-{}-{}",
        std::process::id(),
        chatbotx_core::utils::id::create_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    let db = Arc::new(db);
    setup(&db, |tx| {
        common::workspace(tx, 1, 10)?;
        tx.execute(
            r#"INSERT INTO "Trigger" ("id", "name", "active", "actions", "workspaceId") VALUES (100, 'on', 1, '[]', 10), (101, 'off', 0, '[]', 10)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Condition" ("id", "triggerId", "type", "sourceId") VALUES (200, 100, 'tagApplied', '7'), (201, 101, 'tagRemoved', NULL)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId") VALUES (300, 'hook', 'https://example.test', 10)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId") VALUES (202, 300, 'newContact', NULL)"#,
            [],
        )?;
        Ok(())
    })
    .await;
    Seeded { db, dir }
}

#[tokio::test]
async fn the_trigger_cache_reads_the_active_trigger_conditions() {
    let s = seeded().await;
    let cache = ActiveCache::triggers(Arc::clone(&s.db));
    assert!(
        cache
            .has_active("10", &["tagApplied"], Some("7"))
            .await
            .unwrap()
    );
    assert!(
        !cache
            .has_active("10", &["tagApplied"], Some("8"))
            .await
            .unwrap()
    );
    // The inactive trigger does not count.
    assert!(!cache.has_active("10", &["tagRemoved"], None).await.unwrap());
    assert!(
        !cache
            .has_active("11", &["tagApplied"], Some("7"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn the_webhook_cache_reads_the_webhook_conditions() {
    let s = seeded().await;
    let cache = ActiveCache::webhooks(Arc::clone(&s.db));
    assert!(cache.has_active("10", &["newContact"], None).await.unwrap());
    assert!(
        !cache
            .has_active("10", &["tagApplied"], Some("7"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_trigger_emitter_on_the_bus_queues_a_matching_event() {
    let s = seeded().await;
    let jobs = RecordingEnqueuer::new();
    let bus = EventBus::new();
    bus.subscribe(Arc::new(QueueEmitter::trigger(
        jobs.clone(),
        Arc::new(ActiveCache::triggers(Arc::clone(&s.db))),
    )));
    bus.publish(Event::Trigger(tag_applied("10", "5", "7", None)))
        .await;
    bus.publish(Event::Trigger(tag_applied("10", "5", "8", None)))
        .await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].data["data"]["eventType"], "tagApplied");
    assert_eq!(calls[0].data["data"]["channelOriginated"], false);
}

struct Failing;

impl Subscriber for Failing {
    fn name(&self) -> &str {
        "failing"
    }
    fn handle<'a>(&'a self, _event: &'a Event) -> BoxFuture<'a, Result<(), SubscriberError>> {
        Box::pin(async { Err("boom".to_string()) })
    }
}

#[tokio::test]
async fn a_failing_subscriber_does_not_stop_the_next_one() {
    let log = x1_support::RecordingLog::install();
    let bus = EventBus::new();
    let recorder = RecordingSubscriber::new();
    bus.subscribe(Arc::new(Failing));
    bus.subscribe(recorder.clone());
    bus.publish(Event::Trigger(tag_applied("10", "5", "7", None)))
        .await;
    assert_eq!(recorder.events().len(), 1);
    let entry = log
        .find("Failed to emit event: tagApplied")
        .expect("logged");
    assert_eq!(entry.field("err"), Some("boom"));
}

#[tokio::test]
async fn the_direct_audit_writer_inserts_one_row() {
    let s = seeded().await;
    let audit =
        AuditService::direct(Arc::clone(&s.db), true).with_ids(Arc::new(|| "900".to_string()));
    let actor = AuditActor {
        user_id: Some("1".into()),
        workspace_id: Some("10".into()),
        source: Some("api-token:55".into()),
        ..AuditActor::default()
    };
    with_audit_context(actor, audit.audit("create", "created pipeline 3")).await;
    // The same id again is ignored (`onConflictDoNothing`).
    with_audit_context(
        AuditActor {
            user_id: Some(SYSTEM_ACTOR.into()),
            workspace_id: Some("10".into()),
            ..AuditActor::default()
        },
        audit.audit("create", "again"),
    )
    .await;
    assert_eq!(
        scalar_text(
            &s.db,
            r#"SELECT "detail" FROM "AuditLog" WHERE "id" = ?1"#,
            900
        )
        .await,
        Some("created pipeline 3".to_string())
    );
    assert_eq!(
        scalar_text(
            &s.db,
            r#"SELECT "source" FROM "AuditLog" WHERE "id" = ?1"#,
            900
        )
        .await,
        Some("api-token:55".to_string())
    );
    assert_eq!(
        scalar_i64(
            &s.db,
            r#"SELECT "userId" FROM "AuditLog" WHERE "id" = ?1"#,
            900
        )
        .await,
        Some(1)
    );
    assert_eq!(
        scalar_i64(
            &s.db,
            r#"SELECT COUNT(*) FROM "AuditLog" WHERE "workspaceId" = ?1"#,
            10
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn the_direct_audit_writer_writes_nothing_on_the_community_edition() {
    let s = seeded().await;
    let audit = AuditService::direct(Arc::clone(&s.db), false);
    with_audit_context(
        AuditActor {
            user_id: Some("1".into()),
            workspace_id: Some("10".into()),
            ..AuditActor::default()
        },
        audit.audit("create", "created pipeline 3"),
    )
    .await;
    assert_eq!(
        scalar_i64(
            &s.db,
            r#"SELECT COUNT(*) FROM "AuditLog" WHERE "workspaceId" = ?1"#,
            10
        )
        .await,
        Some(0)
    );
}
