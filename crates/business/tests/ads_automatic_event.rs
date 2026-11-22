//! Port (partial) of `apps/worker/__tests__/ads-automatic-event.test.ts`
//! over a migrated database and a fake queue: the `adsAutomaticEvent`
//! handler's ingestion (`ingestAutomaticEvent`) and its send enqueue. The
//! phone-number lookup and the blocked-owner guard are the worker arm.

mod common;

use std::sync::Mutex;

use chatbotx_business::ads::conversion::{
    AutomaticPayload, enqueue_send_conversion_event, ingest_automatic_event,
};
use chatbotx_business::email_marketing::seal_auth;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use common::{WS, inbox, setup, two_workspaces};
use serde_json::{Value, json};

#[derive(Default)]
struct Queue {
    added: Mutex<Vec<(String, Value, JobOptions)>>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        _: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.added
            .lock()
            .unwrap()
            .push((name.to_owned(), data, opts));
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }
}

fn payload() -> AutomaticPayload {
    AutomaticPayload::parse(&json!({
        "event_name": "LeadSubmitted",
        "id": "wamid.lead-1",
        "timestamp": "1800000000",
        "ctwa_clid": "clid-1",
    }))
    .expect("valid payload")
}

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    let auth = seal_auth(&json!({ "tokens": { "accessToken": "t" } })).unwrap();
    setup(&db, move |tx| {
        inbox(tx, 4001, WS, "whatsapp")?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
               VALUES (6001, ?1, 'phone-1', 'waba-1', 'b', 'WA', ?2, 4001)"#,
            params![auth, WS],
        )?;
        Ok(())
    })
    .await;
    db
}

/// Port of: "maps the job to ads conversion ingestion with integration attribution"
#[tokio::test]
async fn ingests_with_integration_attribution_and_enqueues_the_send() {
    let db = seeded().await;
    let event = ingest_automatic_event(&db, WS, 6001, "waba-1".into(), payload())
        .await
        .expect("ingest")
        .expect("a new row");
    assert_eq!(event.workspace_id, WS);
    assert_eq!(event.event_type, "lead");
    assert_eq!(event.source_event_id, "wamid.lead-1");
    assert_eq!(event.occurred_at, 1_800_000_000_000);
    assert_eq!(event.capi_status, "pending");
    let queue = Queue::default();
    enqueue_send_conversion_event(&queue, &event)
        .await
        .expect("enqueue");
    let added = queue.added.lock().unwrap();
    assert_eq!(added.len(), 1);
    let (name, data, opts) = &added[0];
    assert_eq!(name, "sendConversionEvent");
    assert_eq!(data["type"], "sendConversionEvent");
    assert_eq!(
        data["data"],
        json!({ "adsConversionEventId": event.id.to_string(), "workspaceId": WS.to_string() })
    );
    let job_id = opts.job_id.clone().unwrap();
    assert_eq!(job_id, format!("ads-conversion-send-{}", event.id));
    assert!(!job_id.contains(':'));
}

/// Port of: "duplicate insert returning null still completes"
#[tokio::test]
async fn a_duplicate_insert_returns_none() {
    let db = seeded().await;
    let first = ingest_automatic_event(&db, WS, 6001, "waba-1".into(), payload())
        .await
        .expect("ingest");
    assert!(first.is_some());
    let again = ingest_automatic_event(&db, WS, 6001, "waba-1".into(), payload())
        .await
        .expect("ingest");
    assert!(again.is_none());
}
