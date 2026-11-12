//! Port of `packages/database/__tests__/messaging-ads-connection-repository.test.ts`
//! (P3.3). The mocked `findFirst` / `findMany` / insert / update / delete become real
//! `MessagingAdsConnection` rows: workspace + integration scoping is checked with decoy
//! rows of another workspace and another integration.

mod common;
#[path = "common/repo3.rs"]
mod repo3;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::messaging_ads_connection::{
    CreateMessagingAdsConnectionInput, IntegrationRef, create, find_for_integration,
    list_for_channel, remove, update_auth, update_status,
};
use chatbotx_db::repo::models::MessagingAdsConnection;
use chatbotx_db::repo::{self, Json, find_one};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{err_message, ok, setup};
use serde_json::json;

const WS: i64 = 2001;
const OTHER: i64 = 2002;

/// Integrations of 2001: whatsapp 21, messenger 11 and 13, instagram 31; of 2002:
/// messenger 12.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, |tx| {
        repo3::integration_whatsapp(tx, 21, WS, 3201, "waba-1")?;
        repo3::integration_messenger(tx, 11, WS, 3101, "[]")?;
        repo3::integration_messenger(tx, 13, WS, 3103, "[]")?;
        repo3::integration_messenger(tx, 12, OTHER, 3102, "[]")?;
        repo3::integration_instagram(tx, 31, WS, 3301)?;
        Ok(())
    })
    .await;
    db
}

fn ref_whatsapp(ws: i64, id: i64) -> IntegrationRef {
    IntegrationRef {
        workspace_id: ws,
        integration_whatsapp_id: Some(id),
        ..Default::default()
    }
}
fn ref_messenger(ws: i64, id: i64) -> IntegrationRef {
    IntegrationRef {
        workspace_id: ws,
        integration_messenger_id: Some(id),
        ..Default::default()
    }
}
fn ref_instagram(ws: i64, id: i64) -> IntegrationRef {
    IntegrationRef {
        workspace_id: ws,
        integration_instagram_id: Some(id),
        ..Default::default()
    }
}

async fn add(
    db: &common::TestDb,
    id: i64,
    channel: &str,
    integration: IntegrationRef,
) -> MessagingAdsConnection {
    let input = CreateMessagingAdsConnectionInput {
        id,
        integration,
        channel: channel.to_owned(),
        auth: Json(json!({"accessToken": "token"})),
    };
    ok(repo::write(db, move |tx| create(tx, &input)).await)
}

async fn row(db: &common::TestDb, id: i64) -> Option<MessagingAdsConnection> {
    ok(repo::read(db, move |c| find_one(c, "\"id\" = ?1", &[&id])).await)
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:61 "messagingAdsConnectionRepository.findForIntegration > scopes the lookup by workspaceId AND the matching integration FK"
#[tokio::test]
async fn messaging_ads_connection_repository_find_for_integration_scopes_the_lookup_by_workspace_id_and_the_matching_integration_fk()
 {
    let db = seeded().await;
    add(&db, 1, "whatsapp", ref_whatsapp(WS, 21)).await;
    add(&db, 2, "messenger", ref_messenger(WS, 11)).await;
    let found = ok(repo::read(&db, |c| find_for_integration(c, ref_whatsapp(WS, 21))).await);
    assert_eq!(found.map(|r| r.id), Some(1));
    // The integration id with another workspace finds nothing.
    let wrong_ws = ok(repo::read(&db, |c| find_for_integration(c, ref_whatsapp(OTHER, 21))).await);
    assert_eq!(wrong_ws, None);
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:75 "messagingAdsConnectionRepository.findForIntegration > does not add a filter for an integration FK that was not supplied"
#[tokio::test]
async fn messaging_ads_connection_repository_find_for_integration_does_not_add_a_filter_for_an_integration_fk_that_was_not_supplied()
 {
    let db = seeded().await;
    add(&db, 2, "messenger", ref_messenger(WS, 11)).await;
    // Only the messenger FK is filtered: the row (whatsapp/instagram FKs NULL) matches.
    let found = ok(repo::read(&db, |c| find_for_integration(c, ref_messenger(WS, 11))).await);
    assert_eq!(found.map(|r| r.id), Some(2));
    let other_page = ok(repo::read(&db, |c| find_for_integration(c, ref_messenger(WS, 13))).await);
    assert_eq!(other_page, None);
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:90 "messagingAdsConnectionRepository.findForIntegration > returns null on a miss instead of undefined"
#[tokio::test]
async fn messaging_ads_connection_repository_find_for_integration_returns_null_on_a_miss_instead_of_undefined()
 {
    let db = seeded().await;
    let found = ok(repo::read(&db, |c| find_for_integration(c, ref_instagram(WS, 31))).await);
    assert_eq!(found, None);
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:103 "messagingAdsConnectionRepository.listForChannel > scopes by workspaceId AND channel, and only returns active rows"
#[tokio::test]
async fn messaging_ads_connection_repository_list_for_channel_scopes_by_workspace_id_and_channel_and_only_returns_active_rows()
 {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    add(&db, 2, "messenger", ref_messenger(WS, 13)).await;
    add(&db, 3, "messenger", ref_messenger(OTHER, 12)).await;
    add(&db, 4, "whatsapp", ref_whatsapp(WS, 21)).await;
    ok(repo::write(&db, |tx| {
        update_status(tx, ref_messenger(WS, 13), "invalid")
    })
    .await);
    add(&db, 5, "instagram", ref_instagram(WS, 31)).await;
    let rows = ok(repo::read(&db, |c| list_for_channel(c, WS, "messenger")).await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:118 "messagingAdsConnectionRepository.listForChannel > returns an empty array when no connection exists for the channel"
#[tokio::test]
async fn messaging_ads_connection_repository_list_for_channel_returns_an_empty_array_when_no_connection_exists_for_the_channel()
 {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    assert!(ok(repo::read(&db, |c| list_for_channel(c, WS, "instagram")).await).is_empty());
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:131 "messagingAdsConnectionRepository.create > inserts with status active and nulls the two unused integration FKs"
#[tokio::test]
async fn messaging_ads_connection_repository_create_inserts_with_status_active_and_nulls_the_two_unused_integration_fks()
 {
    let db = seeded().await;
    let created = add(&db, 1, "instagram", ref_instagram(WS, 31)).await;
    assert_eq!(created.id, 1);
    assert_eq!(created.workspace_id, WS);
    assert_eq!(created.channel, "instagram");
    assert_eq!(created.integration_instagram_id, Some(31));
    assert_eq!(created.integration_whatsapp_id, None);
    assert_eq!(created.integration_messenger_id, None);
    assert_eq!(created.status, "active");
    assert_eq!(row(&db, 1).await, Some(created));
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:158 "messagingAdsConnectionRepository.create > throws when the insert returns no row"
///
/// A real insert either returns its row or fails (same id twice).
#[tokio::test]
async fn messaging_ads_connection_repository_create_throws_when_the_insert_returns_no_row() {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    let input = CreateMessagingAdsConnectionInput {
        id: 1,
        integration: ref_messenger(WS, 13),
        channel: "messenger".to_owned(),
        auth: Json(json!({"accessToken": "token"})),
    };
    let second = repo::write(&db, move |tx| create(tx, &input)).await;
    assert!(!err_message(second).is_empty());
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:174 "messagingAdsConnectionRepository.updateAuth > scopes the update by id AND workspaceId, and resets status to active"
#[tokio::test]
async fn messaging_ads_connection_repository_update_auth_scopes_the_update_by_id_and_workspace_id_and_resets_status_to_active()
 {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    ok(repo::write(&db, |tx| {
        update_status(tx, ref_messenger(WS, 11), "invalid")
    })
    .await);
    let auth = Json(json!({"accessToken": "new-token"}));
    let updated = ok(repo::write(&db, move |tx| update_auth(tx, 1, WS, &auth)).await).expect("row");
    assert_eq!(updated.auth, Json(json!({"accessToken": "new-token"})));
    assert_eq!(updated.status, "active");
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:193 "messagingAdsConnectionRepository.updateAuth > returns null when no row matched (wrong workspace)"
#[tokio::test]
async fn messaging_ads_connection_repository_update_auth_returns_null_when_no_row_matched_wrong_workspace()
 {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    let auth = Json(json!({"accessToken": "new-token"}));
    let updated = ok(repo::write(&db, move |tx| update_auth(tx, 1, OTHER, &auth)).await);
    assert_eq!(updated, None);
    assert_eq!(
        row(&db, 1).await.map(|r| r.auth),
        Some(Json(json!({"accessToken": "token"})))
    );
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:207 "messagingAdsConnectionRepository.updateStatus > is a no-op when no connection exists for the integration"
#[tokio::test]
async fn messaging_ads_connection_repository_update_status_is_a_no_op_when_no_connection_exists_for_the_integration()
 {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    ok(repo::write(&db, |tx| update_status(tx, ref_whatsapp(WS, 21), "invalid")).await);
    assert_eq!(
        row(&db, 1).await.map(|r| r.status).as_deref(),
        Some("active")
    );
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:219 "messagingAdsConnectionRepository.updateStatus > updates the status of the resolved connection"
#[tokio::test]
async fn messaging_ads_connection_repository_update_status_updates_the_status_of_the_resolved_connection()
 {
    let db = seeded().await;
    add(&db, 1, "whatsapp", ref_whatsapp(WS, 21)).await;
    ok(repo::write(&db, |tx| update_status(tx, ref_whatsapp(WS, 21), "invalid")).await);
    assert_eq!(
        row(&db, 1).await.map(|r| r.status).as_deref(),
        Some("invalid")
    );
}

/// Port of: packages/database/__tests__/messaging-ads-connection-repository.test.ts:235 "messagingAdsConnectionRepository.remove > deletes scoped by id AND workspaceId"
#[tokio::test]
async fn messaging_ads_connection_repository_remove_deletes_scoped_by_id_and_workspace_id() {
    let db = seeded().await;
    add(&db, 1, "messenger", ref_messenger(WS, 11)).await;
    ok(repo::write(&db, |tx| remove(tx, 1, OTHER)).await);
    assert!(row(&db, 1).await.is_some());
    ok(repo::write(&db, |tx| remove(tx, 1, WS)).await);
    assert_eq!(row(&db, 1).await, None);
}
