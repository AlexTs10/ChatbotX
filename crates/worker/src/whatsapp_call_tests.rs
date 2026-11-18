//! `whatsapp-call.test.ts` / `whatsapp-call-finalize.test.ts` main paths on a
//! migrated database.
use serde_json::{Value, json};

use super::*;
use crate::inbound::tests::setup;

async fn whatsapp(deps: &Deps) {
    deps.db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (7, 2, 'Wa', 'whatsapp', 'phone-1')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "callRecordingEnabled")
                   VALUES (8, '{}', 'phone-1', 'w', 'b', 'Wa', 2, 7, 1)"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn job(event: Value) -> Value {
    json!({ "integrationType": "whatsapp", "integrationIdentifier": "phone-1",
            "payload": { "phoneNumberId": "phone-1", "contact": { "waId": "49", "name": "Ann" }, "event": event } })
}

async fn call_row(deps: &Deps) -> (String, Option<String>, Option<i64>, Option<i64>) {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "status", "outcome", "durationSeconds", "messageId" FROM "WhatsappCall" WHERE "wacid" = 'wacid-1'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn card(deps: &Deps) -> (String, Value) {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "text", "contentAttributes" FROM "Message" WHERE "sourceId" LIKE 'wacall-%'"#,
                [],
                |r| Ok((r.get::<_, String>(0)?, serde_json::from_str(&r.get::<_, String>(1)?).unwrap())),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn an_inbound_call_rings_then_completes_with_one_activity_card() {
    let (deps, _) = setup().await;
    whatsapp(&deps).await;
    let connect = json!({ "kind": "connect", "wacid": "wacid-1", "direction": "userInitiated", "from": "49" });
    handle_event(&deps, &job(connect.clone())).await.unwrap();
    handle_event(&deps, &job(connect)).await.unwrap(); // a redelivery inserts nothing
    assert_eq!(call_row(&deps).await.0, "ringing");

    let terminate = json!({ "kind": "terminate", "wacid": "wacid-1", "direction": "userInitiated", "status": "COMPLETED",
                            "from": "49", "startTime": "1755700000", "endTime": "1755700075", "durationSeconds": 75 });
    handle_event(&deps, &job(terminate.clone())).await.unwrap();
    handle_event(&deps, &job(terminate)).await.unwrap();
    let (status, outcome, duration, message_id) = call_row(&deps).await;
    assert_eq!(
        (status.as_str(), outcome.as_deref(), duration),
        ("completed", Some("completed"), Some(75))
    );
    assert!(message_id.is_some());
    let (text, entity) = card(&deps).await;
    assert_eq!(text, "Voice call · 1:15");
    assert_eq!(entity["type"], "whatsapp_call");
    assert_eq!(
        entity["recordingRequested"], true,
        "the number records calls"
    );
    assert_eq!(entity["transcriptionRequested"], false);
    let messages: i64 = deps
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT count(*) FROM "Message" WHERE "messageType" = 'activity'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(messages, 1);
}

#[tokio::test]
async fn an_unanswered_terminate_without_a_connect_records_a_missed_call() {
    let (deps, _) = setup().await;
    whatsapp(&deps).await;
    let terminate = json!({ "kind": "terminate", "wacid": "wacid-1", "direction": "userInitiated", "status": "COMPLETED", "from": "49",
                            "errors": [{ "code": 138021, "title": "media" }] });
    handle_event(&deps, &job(terminate)).await.unwrap();
    let (status, outcome, _, _) = call_row(&deps).await;
    assert_eq!(
        (status.as_str(), outcome.as_deref()),
        ("failed", Some("failed"))
    );
    let (text, entity) = card(&deps).await;
    assert_eq!(text, "Missed voice call");
    assert_eq!(entity["failureReason"], "138021:media");

    // A late REJECTED upgrades the row and repairs the card.
    let rejected = json!({ "kind": "status", "wacid": "wacid-1", "status": "REJECTED" });
    handle_event(&deps, &job(rejected)).await.unwrap();
    assert_eq!(call_row(&deps).await.0, "rejected");
    assert_eq!(card(&deps).await.0, "Declined voice call");
}

#[tokio::test]
async fn statuses_without_a_row_and_outbound_without_a_match_never_create_rows() {
    let (deps, _) = setup().await;
    whatsapp(&deps).await;
    let ringing = json!({ "kind": "status", "wacid": "wacid-1", "status": "RINGING" });
    handle_event(&deps, &job(ringing)).await.unwrap();
    let accepted = json!({ "kind": "status", "wacid": "wacid-1", "status": "ACCEPTED" });
    assert!(
        handle_event(&deps, &job(accepted)).await.is_err(),
        "ACCEPTED retries until the row exists"
    );
    let outbound = json!({ "kind": "terminate", "wacid": "wacid-1", "direction": "businessInitiated", "status": "FAILED", "to": "49" });
    handle_event(&deps, &job(outbound)).await.unwrap();
    let rows: i64 = deps
        .db
        .read(|c| Ok(c.query_row(r#"SELECT count(*) FROM "WhatsappCall""#, [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn activity_texts() {
    let e = |status: &str, direction: &str| json!({ "status": status, "direction": direction });
    assert_eq!(
        activity_text(&e("completed", "userInitiated")),
        "Voice call"
    );
    assert_eq!(
        activity_text(&e("canceled", "businessInitiated")),
        "Cancelled call"
    );
    assert_eq!(
        activity_text(&e("failed", "businessInitiated")),
        "No answer"
    );
    assert_eq!(
        activity_text(&e("failed", "userInitiated")),
        "Missed voice call"
    );
}
