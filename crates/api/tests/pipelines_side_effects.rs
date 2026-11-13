//! The pipelines side effects of the original services, on the X1
//! `ApiState` services: `pipelineItemChanged` realtime frames
//! (`realtimeEffect`), the flow-trigger events (`emitPipelineItemEvent`) and
//! the audit rows (`this.audit(...)`). Rust-native tests (the TS tests mock
//! these modules).

mod handlers_common;

use std::sync::Arc;

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::events::{EventBus, RecordingSubscriber};
use chatbotx_rt::hub::EntryKind;
use chatbotx_rt::{NoPresence, NoRevokeRetry, Realtime, RealtimeDeps, RtConfig, SystemClock};
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::json;

const CONTACT: i64 = 3001;

async fn seed_contact(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
                [CONTACT, WS],
            )?;
            Ok(())
        })
        .await
        .expect("contact");
}

async fn create_pipeline(f: &Fixture) -> String {
    let (status, body) = call(
        f,
        "pipelines.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "opportunity", "name": "Sales" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

async fn create_item(f: &Fixture, pipeline_id: &str) -> String {
    let (status, body) = call(
        f,
        "pipelines.createItem",
        WS,
        &[("id", pipeline_id)],
        None,
        Some(json!({ "contactId": CONTACT.to_string(), "title": "Deal" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

#[tokio::test]
async fn item_writes_publish_pipeline_item_changed_frames() {
    let mut f = fixture().await;
    let clock: Arc<dyn chatbotx_rt::Clock> = Arc::new(SystemClock);
    let realtime = Realtime::with_deps(
        RtConfig::new("d1p-test-secret-d1p-test-secret-d1p-test"),
        RealtimeDeps {
            clock: Arc::clone(&clock),
            revoke_store: Arc::new(chatbotx_kv::DurableKv::new(Arc::clone(&f.state.db), clock)),
            presence: Arc::new(NoPresence),
            revoke_retry: Arc::new(NoRevokeRetry),
        },
    );
    f.state = f.state.clone().with_realtime(realtime.clone());
    seed_contact(&f).await;
    let pipeline_id = create_pipeline(&f).await;

    let item_id = create_item(&f, &pipeline_id).await;

    let changes: Vec<serde_json::Value> = realtime
        .hub()
        .buffered(&WS.to_string())
        .iter()
        .flat_map(|entry| match &entry.kind {
            EntryKind::WorkspaceEvents { events, .. } => events
                .iter()
                .filter(|event| event.event_type.as_str() == "pipelineItemChanged")
                .map(|event| event.data.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(
        changes,
        vec![json!({
            "pipelineId": pipeline_id,
            "itemId": item_id,
            "contactId": CONTACT.to_string(),
            "change": "created",
        })]
    );
}

#[tokio::test]
async fn item_writes_send_the_flow_trigger_events() {
    let mut f = fixture().await;
    let bus = EventBus::new();
    let recorder = RecordingSubscriber::new();
    bus.subscribe(recorder.clone());
    f.state = f.state.clone().with_events(bus);
    seed_contact(&f).await;
    let pipeline_id = create_pipeline(&f).await;

    create_item(&f, &pipeline_id).await;

    let events = recorder.trigger_events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].event_type, "pipelineItemCreated");
    assert_eq!(events[0].contact_id, CONTACT.to_string());
}

#[tokio::test]
async fn pipeline_writes_record_audit_rows() {
    let mut f = fixture().await;
    f.state = f
        .state
        .clone()
        .with_audit(AuditService::direct(Arc::clone(&f.state.db), true));
    let actor = AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ip_address: None,
        user_agent: None,
        source: Some("api-token:77".into()),
    };

    let pipeline_id = with_audit_context(actor, create_pipeline(&f)).await;

    let rows: Vec<(String, String)> = f
        .state
        .db
        .read(|conn| {
            let mut statement = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" WHERE "workspaceId" = ?1 ORDER BY "id""#,
            )?;
            let rows = statement
                .query_map([WS], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("audit rows");
    assert_eq!(
        rows,
        vec![(
            "create".to_string(),
            format!("created pipeline {pipeline_id}")
        )]
    );
}
