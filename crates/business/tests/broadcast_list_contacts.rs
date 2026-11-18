//! Port of `broadcast-service-list-contacts.test.ts` (Q.W2). Adapted: the
//! mocked analytics and contact-inbox lookups become stored rows; "no longer
//! resolves" is a contact inbox whose contact is in another workspace (the
//! foreign keys keep a deleted one from staying on the recipient row).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::broadcast::list_contacts_page;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, setup, two_workspaces};

#[tokio::test]
async fn lists_recipients_by_event_type() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
               VALUES (501, 'b', ?1, 'sent', 'now', 0, 'allContacts', 'omnichannel')"#,
            params![WS],
        )?;
        common::inbox(tx, 81, WS, "whatsapp")?;
        common::contact(tx, 91, WS)?;
        common::contact(tx, 92, WS)?;
        common::contact(tx, 93, OTHER_WS)?;
        common::contact_inbox(tx, 191, 91, 81, "whatsapp", "src-1")?;
        common::contact_inbox(tx, 192, 92, 81, "whatsapp", "src-2")?;
        common::contact_inbox(tx, 193, 93, 81, "whatsapp", "src-3")?;
        common::conversation(tx, 95, WS, 91)?;
        tx.execute_batch(
            r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "sent", "deliveredAt", "failedAt", "errorContent") VALUES
                 (501, 91, 191, 95, 1, 1767225600000, NULL, NULL),
                 (501, 92, 192, 95, 1, NULL, 1767312000000, 'boom'),
                 (501, 93, 193, 95, 1, 1767100000000, NULL, NULL);"#,
        )?;
        Ok(())
    })
    .await;

    // Not found in this workspace.
    let other = list_contacts_page(&db, OTHER_WS, 501, "message:sent".into(), 1, 20).await;
    assert!(
        matches!(other, Err(BusinessError::Exception { ref message, .. }) if message == "Broadcast not found")
    );

    // No matching recipient.
    let empty = list_contacts_page(&db, WS, 501, "flow:clicked".into(), 1, 20)
        .await
        .unwrap();
    assert_eq!(
        empty,
        serde_json::json!({ "data": [], "total": 0, "pageCount": 0 })
    );

    // Joined rows (newest delivered first, failed one last); the contact
    // without a conversation gets "", the foreign contact inbox is dropped
    // but still counted.
    let page = list_contacts_page(&db, WS, 501, "message:sent".into(), 1, 2)
        .await
        .unwrap();
    assert_eq!(page["total"], 3);
    assert_eq!(page["pageCount"], 2);
    let data = page["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["contactId"], "91");
    assert_eq!(data[0]["contactInboxId"], "191");
    assert_eq!(data[0]["conversationId"], "95");
    assert_eq!(data[0]["firstName"], "Ada");
    assert_eq!(data[0]["sourceId"], "src-1");
    assert_eq!(data[0]["channel"], "whatsapp");
    assert_eq!(data[0]["occurredAt"], "2026-01-01T00:00:00.000Z");

    let failed = list_contacts_page(&db, WS, 501, "message:failed".into(), 1, 20)
        .await
        .unwrap();
    let row = &failed["data"][0];
    assert_eq!(row["contactId"], "92");
    assert_eq!(row["conversationId"], "");
    assert_eq!(row["errorContent"], "boom");
    assert_eq!(row["occurredAt"], "2026-01-02T00:00:00.000Z");
}
