//! Port of `packages/database/__tests__/integration-whatsapp-mark-token-refresh-error.test.ts`
//! (P3.2b). The TS mocks `and` / `eq` and checks the set object; here the update runs on a
//! migrated temp file (adapted): the stored row and a row of another workspace.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::repo::integration_whatsapp as iw;
use repo11::{OTHER_WS, WS, exec, read, two_ws_db, write};

async fn whatsapp_db() -> common::TestDb {
    let db = two_ws_db().await;
    exec(
        &db,
        r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (3101, 'wa', 'whatsapp', 'phone-1', 2001);
           INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
           VALUES (101, '{}', 'phone-1', 'waba-1', 'business', 'WhatsApp', 2001, 3101);"#,
    )
    .await;
    db
}

async fn token_refresh_error(db: &chatbotx_db::Db) -> Option<String> {
    read(db, |c| {
        Ok(c.query_row(
            r#"SELECT "tokenRefreshError" FROM "IntegrationWhatsapp" WHERE "id" = 101"#,
            [],
            |r| r.get::<_, Option<String>>(0),
        )?)
    })
    .await
}

#[tokio::test]
async fn integration_whatsapp_repository_mark_token_refresh_error_scopes_the_update_by_both_id_and_workspace_id_and_returns_the_row_s_phone_number_id()
 {
    let db = whatsapp_db().await;
    let phone = write(&db, |c| iw::mark_token_refresh_error(c, 101, WS, "boom")).await;
    assert_eq!(phone.as_deref(), Some("phone-1"));
    // The set is `{ tokenRefreshError: error }`.
    assert_eq!(token_refresh_error(&db).await.as_deref(), Some("boom"));
}

#[tokio::test]
async fn integration_whatsapp_repository_mark_token_refresh_error_returns_undefined_when_no_row_matches_the_id_workspace_id_pair_a_forged_integration_id_from_another_tenant()
 {
    let db = whatsapp_db().await;
    let phone = write(&db, |c| {
        iw::mark_token_refresh_error(c, 101, OTHER_WS, "boom")
    })
    .await;
    assert_eq!(phone, None);
    assert_eq!(token_refresh_error(&db).await, None);
}
