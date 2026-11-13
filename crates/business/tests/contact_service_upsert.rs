//! Port of `packages/business/__tests__/contact-service-upsert.test.ts`
//! (P3.6-D2). Adapted: the mocked inserts and quota helper become stored
//! rows. Not applicable: "threads the newly-created ContactInbox id into
//! emitContactCreated" (no event bus).

mod common;

use chatbotx_business::contact::{UpsertData, upsert_by_identifier};
use common::{TestDb, WS, inbox, rows, scalar_i64, setup, two_workspaces};

const WEBCHAT: i64 = 3001;

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, WEBCHAT, WS, "webchat")?;
        Ok(())
    })
    .await;
    db
}

fn ada() -> UpsertData {
    UpsertData {
        first_name: Some("Ada".to_string()),
        ..UpsertData::default()
    }
}

#[tokio::test]
async fn writes_the_caller_provided_source_to_new_contact_inbox_rows() {
    let db = db().await;
    let outcome = upsert_by_identifier(&db, WS, "email:ada@example.com", ada(), "api", None)
        .await
        .expect("upserted");
    assert!(outcome.is_new);
    let inboxes = rows(
        &db,
        r#"SELECT "channel", "source", "inboxId" FROM "ContactInbox" WHERE "contactId" = ?1"#,
        outcome.contact.id,
    )
    .await;
    assert_eq!(inboxes.len(), 1);
    assert_eq!(inboxes[0]["channel"], "webchat");
    assert_eq!(inboxes[0]["source"], "api");
    assert_eq!(inboxes[0]["inboxId"], WEBCHAT);
}

#[tokio::test]
async fn creates_the_contact_via_the_no_mac_path_never_the_mac_gated_one() {
    let db = db().await;
    let outcome = upsert_by_identifier(&db, WS, "email:ada@example.com", ada(), "api", None)
        .await
        .expect("upserted");
    assert_eq!(outcome.contact.workspace_id, WS);
    assert_eq!(outcome.contact.email.as_deref(), Some("ada@example.com"));
    assert_eq!(outcome.contact.first_name.as_deref(), Some("Ada"));
    let conversations = scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "Conversation" WHERE "contactId" = ?1"#,
        outcome.contact.id,
    )
    .await;
    assert_eq!(conversations, Some(1));
    // No monthly-active-contact row (the MAC-gated path would record one).
    let mac = scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "ContactActiveMonthly" WHERE "contactId" = ?1"#,
        outcome.contact.id,
    )
    .await;
    assert_eq!(mac, Some(0));
}
