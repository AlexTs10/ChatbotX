//! Port of `apps/builder/__tests__/contacts-refresh-profile-public-api.test.ts`
//! (P3.6-D2, slice inboxes / ads / refresh). Adapted: the mocked
//! `refreshContactProfile` becomes the real one over seeded rows. The
//! messenger inbox has no integration, so the profile fetch fails and the
//! result is `failed` (the TS case returns a mocked `unavailable`).

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn post_v1_contacts_identifier_refresh_profile_resolves_the_identifier_then_delegates_to_refreshcontactprofile()
 {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (5, ?1, 'Page', 'messenger', 'page-5')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (123, ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (50, 123, 123, 5, 'messenger', 'direct', 'psid-123')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &f,
        "contacts.refreshProfile",
        WS,
        &[("identifier", "id:123")],
        None,
        Some(json!({ "contactInboxId": "50" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "status": "failed" }));

    let (status, body) = call(
        &f,
        "contacts.refreshProfile",
        WS,
        &[("identifier", "id:999")],
        None,
        Some(json!({ "contactInboxId": "50" })),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Contact not found"));
}
