//! The WhatsApp coexist buffer → flush pipeline on a migrated database
//! (`coexist-whatsapp-buffer.test.ts`, `coexist-whatsapp-flush.test.ts`
//! happy paths) and the historical id factory (`bulk-historical-import.test.ts`).
use serde_json::{Value, json};

use super::*;
use crate::inbound::tests::setup;

const PHONE: &str = "phone-1";

async fn whatsapp(deps: &Deps, coexist: bool) {
    let coexist = i64::from(coexist);
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (7, 2, 'Wa', 'whatsapp', 'phone-1')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "coexistEnabled")
                   VALUES (8, '{}', 'phone-1', 'w', 'b', 'Wa', 2, 7, ?1)"#,
                [coexist],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn history(progress: i64) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "metadata": { "phone_number_id": PHONE },
        "contacts": [{ "wa_id": "849", "profile": { "name": "Ann" } }],
        "history": [{
            "metadata": { "phase": 2, "chunk_order": 1, "progress": progress },
            "threads": [{ "id": "849", "messages": [
                { "id": "m1", "from": "849", "timestamp": "1755700000", "type": "text", "text": { "body": "hi" } },
                { "id": "m2", "from": "biz", "timestamp": "1755700060", "type": "text", "text": { "body": "hello" } },
            ]}],
        }],
        "messages": [{ "id": "e", "from": "849", "type": "edit", "edit": { "original_message_id": "m1", "message": { "text": { "body": "hi!" } } } }],
    })
}

async fn run_row(deps: &Deps) -> (String, i64, Option<String>) {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "status", "importedMessageCount", "currentStep" FROM "CoexistSyncRun" WHERE "integrationId" = 8"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn open_run(deps: &Deps) {
    write(deps, |c| {
        runs::create_run(
            c,
            &CreateRunInput {
                workspace_id: 2,
                integration_id: 8,
                channel: "whatsapp",
                trigger_source: "popup-enable",
            },
        )
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_staged_history_is_imported_patched_and_closes_the_run_on_metas_terminal_signal() {
    let (deps, jobs) = setup().await;
    whatsapp(&deps, true).await;
    open_run(&deps).await;
    let data = json!({ "phoneNumberId": PHONE, "payload": history(100) });
    buffer(&deps, &data).await.unwrap();
    buffer(&deps, &data).await.unwrap(); // a Meta redelivery stages nothing new
    let flushes: Vec<_> = jobs
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|j| j.1 == FLUSH_JOB)
        .map(|j| j.3.clone())
        .collect();
    assert_eq!(
        flushes[0].job_id.as_deref(),
        Some("coexist-flush-v2-phone-1")
    );
    assert_eq!(flushes[0].delay_ms, Some(60_000));

    flush(&deps, &json!({ "phoneNumberId": PHONE }))
        .await
        .unwrap();
    assert_eq!(
        run_row(&deps).await,
        ("succeeded".into(), 2, Some("done".into()))
    );
    let rows: Vec<(String, String, String, Option<String>, i64)> = deps
        .db
        .read(|c| {
            let mut stmt = c.prepare(
                r#"SELECT "sourceId", "messageType", "text", "contentAttributes", "createdAt" FROM "Message" ORDER BY "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str(), rows[0].2.as_str()),
        ("m1", "incoming", "hi!")
    );
    assert!(
        rows[0]
            .3
            .as_deref()
            .unwrap_or_default()
            .contains("\"edited\":true")
    );
    assert_eq!(
        rows[0].4, 1_755_700_000_000,
        "the message keeps Meta's time"
    );
    assert_eq!(rows[1].1, "outgoing");
    let pending: i64 = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT count(*) FROM "WhatsappCoexistStaging" WHERE "processedAt" IS NULL"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(pending, 0);
}

#[tokio::test]
async fn a_drained_run_waits_until_meta_says_it_is_done() {
    let (deps, _jobs) = setup().await;
    whatsapp(&deps, true).await;
    open_run(&deps).await;
    buffer(
        &deps,
        &json!({ "phoneNumberId": PHONE, "payload": history(40) }),
    )
    .await
    .unwrap();
    flush(&deps, &json!({ "phoneNumberId": PHONE }))
        .await
        .unwrap();
    assert_eq!(
        run_row(&deps).await,
        ("waiting".into(), 2, Some("waiting for history".into()))
    );
}

#[tokio::test]
async fn coexist_off_stages_but_does_not_flush() {
    let (deps, jobs) = setup().await;
    whatsapp(&deps, false).await;
    buffer(
        &deps,
        &json!({ "phoneNumberId": PHONE, "payload": history(100) }),
    )
    .await
    .unwrap();
    assert!(jobs.0.lock().unwrap().iter().all(|j| j.1 != FLUSH_JOB));
    // Unknown numbers are dropped before staging.
    buffer(
        &deps,
        &json!({ "phoneNumberId": "other", "payload": history(100) }),
    )
    .await
    .unwrap();
    let staged: i64 = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT count(*) FROM "WhatsappCoexistStaging""#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(staged, 1);
}

#[test]
fn historical_ids_follow_the_message_time_and_re_mint_on_a_collision() {
    let mut ids = HistoricalIds::default();
    let at = 1_755_700_000_000;
    let a = ids.mint(at, "m1", 9).unwrap();
    assert_eq!(a >> 14, at - COEXIST_EPOCH_MS);
    assert_eq!(a & DISAMBIG_MASK, hash_key("9:m1"));
    let again = ids.mint(at, "m1", 9).unwrap();
    assert_ne!(a, again, "a used id is never handed out twice");
    assert!(ids.mint(at + 1, "m0", 9).unwrap() > a);
    assert!(ids.mint(COEXIST_EPOCH_MS - 1, "x", 9).is_err());
    // FNV-1a 32-bit of the empty string, masked.
    assert_eq!(hash_key(""), 2_166_136_261 & DISAMBIG_MASK);
}

#[test]
fn history_echo_and_state_sync_changes_go_to_the_buffer() {
    let body = json!({ "object": "whatsapp_business_account", "entry": [{ "id": "waba", "changes": [
        { "field": "history", "value": { "metadata": { "phone_number_id": PHONE }, "history": [] } },
        { "field": "smb_message_echoes", "value": { "metadata": { "phone_number_id": PHONE }, "message_echoes": [] } },
        { "field": "smb_app_state_sync", "value": { "metadata": { "phone_number_id": PHONE }, "state_sync": [] } },
        { "field": "history", "value": { "history": [] } },
        { "field": "smb_message_echoes", "value": { "metadata": { "phone_number_id": PHONE } } },
    ]}]});
    let jobs = crate::channels::whatsapp_webhook::webhook_jobs(&body, Some("pinned-elsewhere"));
    assert_eq!(jobs.len(), 3);
    assert!(
        jobs.iter()
            .all(|j| j.name == BUFFER_JOB && j.job_id.starts_with("wa-coexist-phone-1-"))
    );
    assert_eq!(jobs[0].data["data"]["payload"]["history"], json!([]));
}

#[tokio::test]
async fn a_synced_media_message_queues_one_attachment_download() {
    let (deps, jobs) = setup().await;
    whatsapp(&deps, true).await;
    open_run(&deps).await;
    let mut payload = history(100);
    payload["history"][0]["threads"][0]["messages"] = json!([
        { "id": "m3", "from": "849", "timestamp": "1755700000", "type": "image", "image": { "id": "media-1", "mime_type": "image/jpeg" } },
    ]);
    payload["messages"] = json!([]);
    buffer(
        &deps,
        &json!({ "phoneNumberId": PHONE, "payload": payload }),
    )
    .await
    .unwrap();
    flush(&deps, &json!({ "phoneNumberId": PHONE }))
        .await
        .unwrap();
    let downloads: Vec<_> = jobs
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|j| j.1 == crate::coexist_attachment::JOB)
        .map(|j| (j.0, j.3.job_id.clone()))
        .collect();
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].0, QueueName::Low);
    assert!(downloads[0].1.as_deref().unwrap().starts_with("att-"));
}
