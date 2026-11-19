//! Q.D7: the Google Ads conversion record (`record-conversion.ts`) and one
//! Data Manager delivery (`delivery.ts`) over a migrated database, a fake
//! queue and a scripted upstream.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::email_marketing::seal_auth;
use chatbotx_business::google_ads::{settings_page, update_consent};
use chatbotx_business::google_ads_api::{ADS_SCOPE, DATA_MANAGER_SCOPE};
use chatbotx_business::google_ads_conversion::{
    DeliverDeps, DeliverInput, MatchingResolver, MatchingValues, RecordInput, Recorded, deliver,
    record, retry, validate_ingest,
};
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use common::{WS, contact, contact_inbox, inbox, setup, two_workspaces};
use serde_json::{Value, json};

#[derive(Default)]
struct Queue {
    added: Mutex<Vec<(String, Value, JobOptions)>>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        assert_eq!(queue, QueueName::Integration);
        let mut added = self.added.lock().expect("lock");
        let job_id = opts.job_id.clone().unwrap_or_default();
        let created = !added
            .iter()
            .any(|(_, _, o)| o.job_id.as_deref() == Some(&job_id));
        added.push((name.to_owned(), data, opts));
        Box::pin(async move { Ok(AddResult { job_id, created }) })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }
}

#[derive(Default)]
struct Script {
    replies: Mutex<Vec<UpstreamReply>>,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().expect("lock").push(request);
        self.replies
            .lock()
            .expect("lock")
            .pop()
            .unwrap_or_else(|| Err("no scripted reply".into()))
    }
}

const CUSTOMER: &str = "1234567890";

async fn seed(db: &chatbotx_db::Db) {
    let auth = seal_auth(&json!({
        "tokens": { "accessToken": "at-1", "refreshToken": "rt-1" },
        "metadata": { "uploadMethod": "dataManager", "scope": format!("{ADS_SCOPE} {DATA_MANAGER_SCOPE}") },
    }))
    .expect("seal");
    let received = iso(now_ms() - 7 * 60 * 60 * 1000);
    setup(db, move |tx| {
        contact(tx, 3001, WS)?;
        inbox(tx, 4001, WS, "whatsapp")?;
        contact_inbox(tx, 5001, 3001, 4001, "whatsapp", "wa-1")?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "referral" = ?2 WHERE "id" = ?1"#,
            params![5001, json!({ "gclid": "gclid-abc-1234567890", "googleClickReceivedAt": received }).to_string()],
        )?;
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (6001, ?1, 'googleAds')"#,
            params![WS],
        )?;
        let actions = json!([{ "id": "55", "name": "Lead", "category": "SUBMIT_LEAD_FORM",
            "status": "ENABLED", "countingType": "ONE_PER_CLICK", "clickThroughLookbackWindowDays": 90 }]);
        tx.execute(
            r#"INSERT INTO "IntegrationGoogleAds" ("id", "createdAt", "updatedAt", "workspaceId", "integrationId",
                   "auth", "customerId", "conversionCustomerId", "conversionActions", "conversionActionsSyncedAt")
               VALUES (7001, 0, 0, ?1, 6001, ?2, ?3, ?3, ?4, 1)"#,
            params![WS, auth, CUSTOMER, actions.to_string()],
        )?;
        tx.execute(
            r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind",
                   "integrationId", "sourceId", "displayName", "status", "connectedAt")
               VALUES (8001, 0, 0, ?1, 'googleAds', 'integration', 6001, ?2, 'Shop', 'connected', 0)"#,
            params![WS, CUSTOMER],
        )?;
        Ok(())
    })
    .await;
}

fn input(action: &str) -> RecordInput {
    RecordInput {
        workspace_id: WS,
        contact_inbox_id: 5001,
        source: "flowStep".into(),
        scope_id: "step-1".into(),
        conversion_action_id: action.into(),
        value: Some("19.99".into()),
        currency: Some("eur".into()),
        recorded_at: now_ms(),
        dedup_mode: "click".into(),
        consent: json!({ "adUserData": { "status": "granted", "source": "fixed" },
            "adPersonalization": { "status": null, "source": "notProvided" } }),
        ..RecordInput::default()
    }
}

#[tokio::test]
async fn records_a_conversion_once_and_delivers_it_to_data_manager() {
    let db = two_workspaces().await;
    seed(&db).await;
    let queue = Queue::default();

    assert_eq!(
        record(&db, &queue, input("99")).await.expect("record"),
        Recorded::Refused("unknownConversionAction")
    );
    let Recorded::Queued(event) = record(&db, &queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    assert_eq!(event.status, "pending");
    assert_eq!(event.currency.as_deref(), Some("EUR"));
    assert!(event.transaction_id.starts_with("gads-v2-55-c-"));
    assert_eq!(event.options.as_ref().expect("options").0["version"], 1);
    // A repeat of the same click keeps the one event.
    let Recorded::Queued(again) = record(&db, &queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    assert_eq!(again.id, event.id);
    {
        let added = queue.added.lock().expect("lock");
        assert_eq!(added[0].0, "sendGoogleAdsConversion");
        assert_eq!(
            added[0].2.job_id,
            Some(format!("google-ads-send-{}-a0", event.id))
        );
        assert_eq!(added[0].2.delay_ms, Some(0));
        assert_eq!(
            added[0].1["data"]["googleAdsConversionEventId"],
            event.id.to_string()
        );
    }

    let script = Script::default();
    script
        .replies
        .lock()
        .expect("lock")
        .push(Ok((200, json!({ "requestId": "req-1" }).to_string())));
    let deps = DeliverDeps {
        db: &db,
        up: &script,
        jobs: &queue,
        events: None,
        app_url: "https://app.test",
        matching: None,
    };
    let run = DeliverInput {
        event_id: event.id,
        workspace_id: WS,
        attempt: 0,
        is_last_in_job_attempt: false,
    };
    deliver(&deps, run).await.expect("deliver");
    // A duplicate job claims nothing and sends nothing.
    deliver(&deps, run).await.expect("deliver");
    let seen = script.seen.lock().expect("lock").clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://datamanager.googleapis.com/v1/events:ingest"
    );
    let body: Value = serde_json::from_str(seen[0].body.as_deref().expect("body")).expect("json");
    assert_eq!(
        body["destinations"][0]["operatingAccount"]["accountId"],
        CUSTOMER
    );
    assert_eq!(body["destinations"][0]["productDestinationId"], "55");
    assert_eq!(
        body["events"][0]["adIdentifiers"]["gclid"],
        "gclid-abc-1234567890"
    );
    assert_eq!(body["events"][0]["conversionValue"], 19.99);
    assert_eq!(
        body["events"][0]["consent"],
        json!({ "adUserData": "CONSENT_GRANTED" })
    );

    let id = event.id;
    let (status, request, processing): (String, Option<String>, Option<String>) = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "status", "requestId", "processingStatus" FROM "GoogleAdsConversionEvent" WHERE "id" = ?1"#,
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .expect("event");
    assert_eq!(status, "sent");
    assert_eq!(request.as_deref(), Some("req-1"));
    assert_eq!(processing.as_deref(), Some("processing"));
}

/// The step's templates resolve to raw contact values.
struct Contact;

impl MatchingResolver for Contact {
    fn resolve<'a>(
        &'a self,
        _: i64,
        contact_inbox_id: i64,
        email: Option<String>,
        phone: Option<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = MatchingValues> + Send + 'a>> {
        assert_eq!(contact_inbox_id, 5001);
        assert_eq!(email.as_deref(), Some("{{contact.email}}"));
        assert_eq!(phone.as_deref(), Some("{{contact.phone}}"));
        Box::pin(async {
            Ok(Some((
                Some(" John.Doe+ads@GMail.com ".to_string()),
                Some("14155552671".to_string()),
            )))
        })
    }
}

/// Delivers a matching-enabled conversion; `terms` = the account's
/// `acceptedCustomerDataTerms`. Returns the Data Manager request body.
async fn matched_body(terms: Option<bool>) -> Value {
    let db = two_workspaces().await;
    seed(&db).await;
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "IntegrationGoogleAds" SET "acceptedCustomerDataTerms" = ?1"#,
            params![terms],
        )?;
        Ok(())
    })
    .await;
    let queue = Queue::default();
    let mut with_matching = input("55");
    with_matching.match_email = Some("{{contact.email}}".into());
    with_matching.match_phone = Some(" {{contact.phone}} ".into());
    let Recorded::Queued(event) = record(&db, &queue, with_matching).await.expect("record") else {
        panic!("not queued");
    };
    let options = &event.options.as_ref().expect("options").0;
    assert_eq!(options["version"], 2);
    assert_eq!(options["matching"]["status"], "enabled");

    let script = Script::default();
    script
        .replies
        .lock()
        .expect("lock")
        .push(Ok((200, json!({ "requestId": "req-2" }).to_string())));
    let deps = DeliverDeps {
        db: &db,
        up: &script,
        jobs: &queue,
        events: None,
        app_url: "https://app.test",
        matching: Some(&Contact),
    };
    let run = DeliverInput {
        event_id: event.id,
        workspace_id: WS,
        attempt: 0,
        is_last_in_job_attempt: false,
    };
    deliver(&deps, run).await.expect("deliver");
    let seen = script.seen.lock().expect("lock").clone();
    serde_json::from_str(seen[0].body.as_deref().expect("body")).expect("json")
}

#[tokio::test]
async fn customer_matching_sends_hashed_identifiers_when_consent_is_granted() {
    let body = matched_body(Some(true)).await;
    let sha = chatbotx_core::utils::crypto::sha256_hex;
    assert_eq!(
        body["events"][0]["userData"]["userIdentifiers"],
        json!([
            { "emailAddress": sha("johndoe@gmail.com") },
            { "phoneNumber": sha("+14155552671") },
        ])
    );
    assert_eq!(body["encoding"], "HEX");
}

#[tokio::test]
async fn customer_matching_is_withheld_until_the_terms_are_accepted() {
    for terms in [None, Some(false)] {
        let body = matched_body(terms).await;
        assert!(body["events"][0]["userData"].is_null(), "{terms:?}: {body}");
        assert_eq!(
            body["events"][0]["adIdentifiers"]["gclid"],
            "gclid-abc-1234567890"
        );
    }
}

// ---- housekeeping (port of a subset of `google-ads-housekeeping.test.ts`) -----

const HOUR: i64 = 60 * 60 * 1000;

/// A recorded conversion delivered to Data Manager (`sent`, `req-1`).
async fn sent_event(db: &chatbotx_db::Db, queue: &Queue) -> i64 {
    seed(db).await;
    let Recorded::Queued(event) = record(db, queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    let script = Script::default();
    script
        .replies
        .lock()
        .expect("lock")
        .push(Ok((200, json!({ "requestId": "req-1" }).to_string())));
    let deps = DeliverDeps {
        db,
        up: &script,
        jobs: queue,
        events: None,
        app_url: "https://app.test",
        matching: None,
    };
    let run = DeliverInput {
        event_id: event.id,
        workspace_id: WS,
        attempt: 0,
        is_last_in_job_attempt: false,
    };
    deliver(&deps, run).await.expect("deliver");
    event.id
}

type Row = (
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    Option<String>,
);

async fn row(db: &chatbotx_db::Db, id: i64) -> Row {
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "status", "attempt", "processingStatus", "failureStage", "error",
                      "processingAttempts", "nextProcessingCheckAt", "processingDetail"
               FROM "GoogleAdsConversionEvent" WHERE "id" = ?1"#,
            params![id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )?)
    })
    .await
    .expect("row")
}

async fn poll_with(
    db: &chatbotx_db::Db,
    queue: &Queue,
    reply: Option<Value>,
    now: i64,
) -> Vec<UpstreamRequest> {
    let script = Script::default();
    if let Some(reply) = reply {
        script
            .replies
            .lock()
            .expect("lock")
            .push(Ok((200, reply.to_string())));
    }
    let deps = DeliverDeps {
        db,
        up: &script,
        jobs: queue,
        events: None,
        app_url: "https://app.test",
        matching: None,
    };
    chatbotx_business::google_ads_housekeeping::poll_processing_status(&deps, now)
        .await
        .expect("poll");
    script.seen.lock().expect("lock").clone()
}

fn one_status(status: &str, reasons: &[&str]) -> Value {
    let errors: Vec<Value> = reasons
        .iter()
        .map(|r| json!({ "reason": r, "recordCount": "1" }))
        .collect();
    json!({ "requestStatusPerDestination": [{
        "requestStatus": status,
        "eventsIngestionStatus": { "recordCount": "1" },
        "errorInfo": { "errorCounts": errors },
    }] })
}

/// Port of: "pollGoogleAdsProcessingStatus > SUCCESS marks the event processed and stops polling"
#[tokio::test]
async fn poll_success_marks_the_event_processed_and_stops_polling() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let now = now_ms() + 4 * HOUR;
    let seen = poll_with(&db, &queue, Some(one_status("SUCCESS", &[])), now).await;
    assert_eq!(
        seen[0].url,
        "https://datamanager.googleapis.com/v1/requestStatus:retrieve?requestId=req-1"
    );
    assert_eq!(seen[0].method, "GET");
    let (status, _, processing, _, _, attempts, next, detail) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("processed", Some("success"))
    );
    assert_eq!((attempts, next), (1, None));
    let detail: Value = serde_json::from_str(&detail.expect("detail")).expect("json");
    assert_eq!(detail["recordCount"], 1);
    // Not due any more: no second call.
    assert!(
        poll_with(&db, &queue, None, now + 30 * 24 * HOUR)
            .await
            .is_empty()
    );
}

/// Port of: "a duplicate-only failure finishes processed with duplicateRecovery, fenced by the poll"
#[tokio::test]
async fn poll_duplicate_only_failure_finishes_processed_with_duplicate_recovery() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let reply = one_status("FAILED", &["PROCESSING_ERROR_REASON_DUPLICATE_GCLID"]);
    poll_with(&db, &queue, Some(reply), now_ms() + 4 * HOUR).await;
    let (status, _, processing, _, _, _, _, detail) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("processed", Some("success"))
    );
    let detail: Value = serde_json::from_str(&detail.expect("detail")).expect("json");
    assert_eq!(detail["duplicateRecovery"], true);
    assert_eq!(detail["errorCounts"], json!([]));
}

/// Port of: "FAILED marks the event failed(processing) and logs a provider error" (+ "open-ended Google reasons are bounded to enum shapes")
#[tokio::test]
async fn poll_failed_marks_the_event_failed_with_bounded_reasons() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let reply = one_status(
        "FAILED",
        &["PROCESSING_ERROR_REASON_INVALID_GCLID", "gclid=abc leaked"],
    );
    poll_with(&db, &queue, Some(reply), now_ms() + 4 * HOUR).await;
    let (status, _, processing, stage, error, _, next, detail) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("failed", Some("failed"))
    );
    assert_eq!(stage.as_deref(), Some("processing"));
    assert_eq!(
        error.as_deref(),
        Some("PROCESSING_ERROR_REASON_INVALID_GCLID, UNKNOWN")
    );
    assert_eq!(next, None);
    assert!(!detail.expect("detail").contains("gclid=abc"));
}

/// Port of: "a transient Google failure is redriven as a new generation after 6h"
#[tokio::test]
async fn poll_transient_failure_is_redriven_as_a_new_generation_after_six_hours() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let reply = one_status("FAILED", &["PROCESSING_ERROR_REASON_TOO_RECENT_CLICK"]);
    poll_with(&db, &queue, Some(reply), now_ms() + 4 * HOUR).await;
    let (status, attempt, ..) = row(&db, id).await;
    assert_eq!((status.as_str(), attempt), ("pending", 1));
    let added = queue.added.lock().expect("lock");
    let last = added.last().expect("job");
    assert_eq!(last.2.job_id, Some(format!("google-ads-send-{id}-a1")));
    assert_eq!(last.2.delay_ms, Some(6 * 60 * 60 * 1000));
}

/// Port of: "PROCESSING stays sent and backs off by the poll count" (+ "an unrecognised response stays sent as unknown")
#[tokio::test]
async fn poll_processing_stays_sent_and_backs_off() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let now = now_ms() + 4 * HOUR;
    poll_with(&db, &queue, Some(one_status("PROCESSING", &[])), now).await;
    let (status, _, processing, _, _, attempts, next, _) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("sent", Some("processing"))
    );
    assert_eq!((attempts, next), (1, Some(now + 3 * HOUR)));
    let later = now + 3 * HOUR;
    poll_with(
        &db,
        &queue,
        Some(json!({ "requestStatusPerDestination": [] })),
        later,
    )
    .await;
    let (status, _, processing, _, _, attempts, next, _) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("sent", Some("unknown"))
    );
    assert_eq!((attempts, next), (2, Some(later + 6 * HOUR)));
}

/// Port of: "times out a request unresolved for more than 7 days"
#[tokio::test]
async fn poll_times_out_a_request_unresolved_for_more_than_seven_days() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    let seen = poll_with(&db, &queue, None, now_ms() + 8 * 24 * HOUR).await;
    assert!(seen.is_empty());
    let (status, _, processing, stage, ..) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("failed", Some("timed_out"))
    );
    assert_eq!(stage.as_deref(), Some("timeout"));
}

/// Port of: "sweepStrandedGoogleAdsEvents > redrives a pending event whose job is dead, delayed by the click age" (+ "redrives a sending event with a stale claim")
#[tokio::test]
async fn sweep_redrives_dead_pending_and_stale_sending_events() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    seed(&db).await;
    let Recorded::Queued(event) = record(&db, &queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    let (id, now) = (event.id, now_ms());
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "GoogleAdsConversionEvent" SET "updatedAt" = ?2 WHERE "id" = ?1"#,
            params![id, now - HOUR],
        )?;
        Ok(())
    })
    .await;
    let script = Script::default();
    let deps = DeliverDeps {
        db: &db,
        up: &script,
        jobs: &queue,
        events: None,
        app_url: "https://app.test",
        matching: None,
    };
    // The fake queue knows no job, so the generation's job is dead.
    chatbotx_business::google_ads_housekeeping::sweep_stranded(&deps, now, 200)
        .await
        .expect("sweep");
    let (status, attempt, ..) = row(&db, id).await;
    assert_eq!((status.as_str(), attempt), ("pending", 1));
    {
        let added = queue.added.lock().expect("lock");
        let last = added.last().expect("job");
        assert_eq!(last.2.job_id, Some(format!("google-ads-send-{id}-a1")));
        // The click is 7 h old: past the 6 h gate.
        assert_eq!(last.2.delay_ms, Some(0));
    }
    // A crashed worker's lease.
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "GoogleAdsConversionEvent" SET "status" = 'sending', "claimToken" = 'dead',
                   "claimedAt" = ?2 WHERE "id" = ?1"#,
            params![id, now - HOUR],
        )?;
        Ok(())
    })
    .await;
    chatbotx_business::google_ads_housekeeping::sweep_stranded(&deps, now, 200)
        .await
        .expect("sweep");
    let (status, attempt, ..) = row(&db, id).await;
    assert_eq!((status.as_str(), attempt), ("pending", 2));
    assert!(script.seen.lock().expect("lock").is_empty());
}

async fn block_workspace(db: &chatbotx_db::Db) {
    setup(db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
}

/// Port of: "pollGoogleAdsProcessingStatus > blocked owner reschedules as unknown without calling Google"
#[tokio::test]
async fn poll_blocked_owner_reschedules_as_unknown_without_calling_google() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    let id = sent_event(&db, &queue).await;
    block_workspace(&db).await;
    let now = now_ms() + 4 * HOUR;
    let seen = poll_with(&db, &queue, Some(one_status("SUCCESS", &[])), now).await;
    assert!(seen.is_empty());
    let (status, _, processing, _, _, attempts, next, _) = row(&db, id).await;
    assert_eq!(
        (status.as_str(), processing.as_deref()),
        ("sent", Some("unknown"))
    );
    assert_eq!((attempts, next), (1, Some(now + 3 * HOUR)));
}

/// Port of: "sweepStrandedGoogleAdsEvents > a blocked workspace's stranded sending row is released to pending on the same generation"
#[tokio::test]
async fn sweep_releases_a_blocked_workspaces_sending_row_on_the_same_generation() {
    let db = two_workspaces().await;
    let queue = Queue::default();
    seed(&db).await;
    let Recorded::Queued(event) = record(&db, &queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    let (id, now) = (event.id, now_ms());
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "GoogleAdsConversionEvent" SET "status" = 'sending', "claimToken" = 'dead',
                   "claimedAt" = ?2 WHERE "id" = ?1"#,
            params![id, now - HOUR],
        )?;
        Ok(())
    })
    .await;
    block_workspace(&db).await;
    let jobs_before = queue.added.lock().expect("lock").len();
    let script = Script::default();
    let deps = DeliverDeps {
        db: &db,
        up: &script,
        jobs: &queue,
        events: None,
        app_url: "https://app.test",
        matching: None,
    };
    chatbotx_business::google_ads_housekeeping::sweep_stranded(&deps, now, 200)
        .await
        .expect("sweep");
    let (status, attempt, ..) = row(&db, id).await;
    assert_eq!((status.as_str(), attempt), ("pending", 0));
    assert_eq!(queue.added.lock().expect("lock").len(), jobs_before);
}

/// Own (`retryGoogleAdsEventAction` / `googleAdsConversionService.retry`):
/// only a `failed` event of this workspace is redriven and queued again.
#[tokio::test]
async fn retry_redrives_only_a_failed_event_of_the_workspace() {
    let db = two_workspaces().await;
    seed(&db).await;
    let queue = Queue::default();
    let Recorded::Queued(event) = record(&db, &queue, input("55")).await.expect("record") else {
        panic!("not queued");
    };
    assert!(!retry(&db, &queue, WS, event.id).await.expect("pending"));
    let id = event.id;
    db.write(move |conn| {
        conn.execute(
            r#"UPDATE "GoogleAdsConversionEvent" SET "status" = 'failed', "error" = 'boom', "failureStage" = 'delivery' WHERE "id" = ?1"#,
            params![id],
        )?;
        Ok(())
    })
    .await
    .expect("fail it");
    assert!(
        !retry(&db, &queue, common::OTHER_WS, event.id)
            .await
            .expect("other workspace")
    );
    let before = queue.added.lock().expect("lock").len();
    assert!(retry(&db, &queue, WS, event.id).await.expect("failed"));
    assert_eq!(queue.added.lock().expect("lock").len(), before + 1);
    let status: String = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "status" FROM "GoogleAdsConversionEvent" WHERE "id" = ?1"#,
                params![id],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("status");
    assert_eq!(status, "pending");
}

/// Own (`validateGoogleAdsRequestAction` / `validateIngest`): a
/// `validateOnly` Data Manager call; a Google refusal never echoes the click id.
#[tokio::test]
async fn validate_ingest_sends_validate_only_and_redacts_the_click_id() {
    let db = two_workspaces().await;
    seed(&db).await;
    let click = "gclid-validate-123456";
    let script = Script::default();
    script.replies.lock().expect("lock").extend([
        Ok((
            400,
            json!({ "error": { "code": 400, "status": "INVALID_ARGUMENT",
                "message": format!("bad click {click}") } })
            .to_string(),
        )),
        Ok((200, json!({ "requestId": "req-v" }).to_string())),
    ]);
    let ok = validate_ingest(&db, &script, "https://app.test", WS, "55", "gclid", click)
        .await
        .expect("validate");
    assert_eq!(ok["ok"], json!(true), "{ok}");
    assert_eq!(ok["consentSummary"]["adPersonalization"], json!("omitted"));
    let body: Value = serde_json::from_str(
        script.seen.lock().expect("lock")[0]
            .body
            .as_deref()
            .expect("body"),
    )
    .expect("json");
    assert_eq!(body["validateOnly"], json!(true));
    let refused = validate_ingest(&db, &script, "https://app.test", WS, "55", "gclid", click)
        .await
        .expect("validate");
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["code"], json!("rejected"));
    assert!(!refused.to_string().contains(click), "{refused}");
    // Nothing is recorded.
    let count: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT COUNT(*) FROM "GoogleAdsConversionEvent""#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(count, 0);
    let none = validate_ingest(
        &db,
        &script,
        "https://app.test",
        common::OTHER_WS,
        "55",
        "gclid",
        click,
    )
    .await
    .expect("other");
    assert_eq!(none, json!({ "ok": false, "code": "accountNotReady" }));
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-page.test.ts:41 "passes the consent through when nothing is connected"
#[tokio::test]
async fn load_settings_page_passes_the_consent_through_when_nothing_is_connected() {
    let db = two_workspaces().await;
    let page = settings_page(&db, WS).await.expect("page");
    assert_eq!(page["setup"], Value::Null);
    assert_eq!(page["consent"]["status"], json!("absent"));
    let consent = json!({
        "adUserData": { "type": "granted" },
        "adPersonalization": { "type": "variable", "template": "{{contact.consent}}" },
    });
    update_consent(&db, WS, &consent).await.expect("save");
    let page = settings_page(&db, WS).await.expect("page");
    assert_eq!(page["consent"]["status"], json!("ok"));
    assert_eq!(
        page["consent"]["adUserData"],
        json!({ "type": "granted", "template": null })
    );
    assert_eq!(
        page["consent"]["adPersonalization"]["template"],
        json!("{{contact.consent}}")
    );
    assert!(
        update_consent(&db, WS, &json!({ "adUserData": { "type": "nope" } }))
            .await
            .is_err()
    );
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-page.test.ts:56 "reports an unreadable stored consent as invalid"
#[tokio::test]
async fn load_settings_page_reports_an_unreadable_stored_consent_as_invalid() {
    let db = two_workspaces().await;
    seed(&db).await;
    db.write(|conn| {
        conn.execute(
            r#"INSERT INTO "GoogleAdsSettings" ("workspaceId", "settings") VALUES (?1, '{"version":9}')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await
    .expect("bad doc");
    let page = settings_page(&db, WS).await.expect("page");
    assert_eq!(
        page["consent"],
        json!({ "status": "invalid", "adUserData": null, "adPersonalization": null })
    );
    // The setup view carries no `auth` / tokens.
    assert_eq!(page["setup"]["customerId"], json!(CUSTOMER));
    assert!(!page.to_string().contains("at-1"));
}
