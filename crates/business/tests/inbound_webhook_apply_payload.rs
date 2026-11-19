//! Port of `packages/business/__tests__/inbound-webhook-apply-payload.test.ts`
//! and the main cases of `inbound-webhook-resolve-inbox.test.ts`,
//! `inbound-webhook-resolve-contact.test.ts` and
//! `inbound-webhook-apply-mapped-fields.test.ts` (Q.D13a). Adapted: the
//! mocked resolvers become a migrated database, the asserted mock calls the
//! stored `Contact` / `ContactInbox` / `ContactCustomField` rows. Not
//! applicable: the distributed create lock (one write transaction) and the
//! "infrastructure failure propagates" case (no injectable failure).

mod common;

use chatbotx_business::inbound_webhook;
use chatbotx_business::inbound_webhook_apply::{Applied, apply_payload};
use chatbotx_db::rusqlite::params;
use common::{
    OTHER_WS, TestDb, WS, contact_inbox, contact_with, custom_field, custom_field_values, inbox,
    rows, scalar_i64, setup, two_workspaces,
};
use serde_json::{Value, json};

const MESSENGER: i64 = 3101;
const OFFLINE: i64 = 3102;
const FOREIGN: i64 = 3103;
const HOOK: i64 = 3201;

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, MESSENGER, WS, "messenger")?;
        inbox(tx, OFFLINE, WS, "messenger")?;
        tx.execute(
            r#"UPDATE "Inbox" SET "status" = 'disconnected' WHERE "id" = ?1"#,
            [OFFLINE],
        )?;
        inbox(tx, FOREIGN, OTHER_WS, "messenger")?;
        custom_field(tx, 100, WS, "Total", "number")?;
        Ok(())
    })
    .await;
    db
}

/// Saves the webhook config and returns the stored row.
async fn webhook(db: &TestDb, resolution: Value) -> chatbotx_db::repo::models::InboundWebhook {
    let identifiers = json!([{ "kind": "channelUserId", "jsonPath": "$.sender.id" }]);
    let mappings = json!([
        { "jsonPath": "$.name", "target": "full_name" },
        { "jsonPath": "$.total", "target": "100" },
        { "jsonPath": "$.bad", "target": "100" },
    ]);
    let resolution = (!resolution.is_null()).then(|| resolution.to_string());
    setup(db, move |tx| {
        tx.execute(r#"DELETE FROM "InboundWebhook""#, [])?;
        tx.execute(
            r#"INSERT INTO "InboundWebhook" ("id", "workspaceId", "name", "active", "token",
                 "inboxResolution", "identifiers", "fieldMappings", "actions")
               VALUES (?1, ?2, 'Hook', 1, 'iwh_test', ?3, ?4, ?5, '[]')"#,
            params![
                HOOK,
                WS,
                resolution,
                identifiers.to_string(),
                mappings.to_string()
            ],
        )?;
        Ok(())
    })
    .await;
    inbound_webhook::find_or_fail(db, WS, HOOK)
        .await
        .expect("webhook")
}

fn payload() -> Value {
    json!({
        "sender": { "id": "psid-1" },
        "name": "John Silva",
        "total": "9.5",
        "bad": "abc",
    })
}

fn by_inbox(inbox_id: i64) -> Value {
    json!({ "mode": "inbox", "inboxId": inbox_id.to_string() })
}

async fn contact_count(db: &TestDb) -> i64 {
    scalar_i64(
        db,
        r#"SELECT COUNT(*) FROM "Contact" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

#[tokio::test]
async fn resolves_inbox_then_contact_writes_valid_fields_and_reports_rejected_ones() {
    let db = db().await;
    let hook = webhook(&db, by_inbox(MESSENGER)).await;
    let Applied::Processed {
        inbox_id,
        contact_id,
        is_new,
        field_errors,
        ..
    } = apply_payload(&db, &hook, payload()).await.expect("apply")
    else {
        panic!("not processed");
    };
    assert_eq!(inbox_id, MESSENGER);
    assert!(is_new);
    assert_eq!(
        field_errors,
        vec![json!({ "target": "100", "code": "invalidValue" })]
    );
    // The contact is created with the mapped name (the contact-created event carries it).
    let contact = rows(
        &db,
        r#"SELECT "firstName", "lastName" FROM "Contact" WHERE "id" = ?1"#,
        contact_id,
    )
    .await;
    assert_eq!(contact[0]["firstName"], json!("John"));
    assert_eq!(contact[0]["lastName"], json!("Silva"));
    let inboxes = rows(
        &db,
        r#"SELECT "inboxId", "sourceId", "source" FROM "ContactInbox" WHERE "contactId" = ?1"#,
        contact_id,
    )
    .await;
    assert_eq!(inboxes.len(), 1);
    assert_eq!(inboxes[0]["inboxId"], json!(MESSENGER));
    assert_eq!(inboxes[0]["sourceId"], json!("psid-1"));
    let values = custom_field_values(&db, contact_id).await;
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, 100);
    assert!(values[0].1.contains("9.5"), "{values:?}");
}

#[tokio::test]
async fn is_repeatable_a_second_run_finds_the_contact_instead_of_creating_one() {
    let db = db().await;
    let hook = webhook(&db, by_inbox(MESSENGER)).await;
    let first = apply_payload(&db, &hook, payload()).await.expect("first");
    let second = apply_payload(&db, &hook, payload()).await.expect("second");
    let (
        Applied::Processed { contact_id: a, .. },
        Applied::Processed {
            contact_id: b,
            is_new,
            ..
        },
    ) = (first, second)
    else {
        panic!("not processed");
    };
    assert_eq!(a, b);
    assert!(!is_new);
    assert_eq!(contact_count(&db).await, 1);
}

#[tokio::test]
async fn finds_an_existing_contact_by_its_channel_user_id() {
    let db = db().await;
    setup(&db, |tx| {
        contact_with(tx, 4001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 4101, 4001, MESSENGER, "messenger", "psid-1")?;
        Ok(())
    })
    .await;
    let hook = webhook(&db, by_inbox(MESSENGER)).await;
    let out = apply_payload(&db, &hook, payload()).await.expect("apply");
    assert!(matches!(
        out,
        Applied::Processed {
            contact_id: 4001,
            contact_inbox_id: 4101,
            is_new: false,
            ..
        }
    ));
    assert_eq!(contact_count(&db).await, 1);
}

#[tokio::test]
async fn skips_without_touching_contacts_when_the_inbox_cannot_be_resolved() {
    let db = db().await;
    for (inbox_id, reason) in [(FOREIGN, "inboxNotFound"), (OFFLINE, "inboxDisconnected")] {
        let hook = webhook(&db, by_inbox(inbox_id)).await;
        let out = apply_payload(&db, &hook, payload()).await.expect("apply");
        assert!(
            matches!(out, Applied::Skipped { reason: r, inbox_id: None, .. } if r == reason),
            "{out:?}"
        );
    }
    assert_eq!(contact_count(&db).await, 0);
}

#[tokio::test]
async fn skips_and_writes_nothing_when_the_payload_has_no_usable_identity() {
    let db = db().await;
    let hook = webhook(&db, by_inbox(MESSENGER)).await;
    let out = apply_payload(&db, &hook, json!({ "name": "John" }))
        .await
        .expect("apply");
    assert!(matches!(
        out,
        Applied::Skipped {
            reason: "missingIdentifier",
            inbox_id: Some(MESSENGER),
            ..
        }
    ));
    assert_eq!(contact_count(&db).await, 0);
}

#[tokio::test]
async fn an_incomplete_config_is_skipped_as_inactive_without_any_lookup() {
    let db = db().await;
    let hook = webhook(&db, Value::Null).await;
    let out = apply_payload(&db, &hook, payload()).await.expect("apply");
    assert_eq!(
        out,
        Applied::Skipped {
            reason: "webhookInactive",
            inbox_id: None,
            field_errors: vec![],
        }
    );
}

#[tokio::test]
async fn payload_mode_reads_the_inbox_identifier_and_reports_a_missing_one() {
    let db = db().await;
    let hook = webhook(
        &db,
        json!({
            "mode": "payload",
            "channel": "messenger",
            "identifierKind": "pageId",
            "jsonPath": "$.entry[0].id",
        }),
    )
    .await;
    let out = apply_payload(&db, &hook, payload()).await.expect("apply");
    assert!(matches!(
        out,
        Applied::Skipped {
            reason: "missingInboxIdentifier",
            ..
        }
    ));
    let mut unknown = payload();
    unknown["entry"] = json!([{ "id": "page-unknown" }]);
    let out = apply_payload(&db, &hook, unknown).await.expect("apply");
    assert!(matches!(
        out,
        Applied::Skipped {
            reason: "inboxNotFound",
            ..
        }
    ));
}
