//! Port of `packages/database/__tests__/messaging-ad-operation-repository.test.ts` (P3.3).
//! The mocked insert/update chains become real `MessagingAdOperation` rows; the asserted
//! `values` / `set` arguments are read back from the stored row.

mod common;
#[path = "common/repo3.rs"]
mod repo3;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::messaging_ad_operation::{
    CreateMessagingAdOperationInput, UpdateCreateProgressInput, create, update_create_progress,
};
use chatbotx_db::repo::{self, Json};
use common::{fresh_db, seed};
use repo_common::{err_message, ok, setup};
use serde_json::json;

const WS: i64 = 2001;

fn ad_input() -> Json {
    Json(json!({
        "adAccountId": "act_9",
        "campaign": {"name": "My ad", "specialAdCategories": ["NONE"]},
        "adSet": {"dailyBudgetMinorUnits": 2000, "targeting": {"countries": ["US"]}},
        "creative": {
            "media": {"kind": "image", "imageHash": "hash", "link": "https://x.com"},
            "welcomeMessage": {"type": "default"}
        }
    }))
}

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo3::integration_whatsapp(tx, 21, WS, 3201, "waba-1")?;
        repo3::integration_messenger(tx, 11, WS, 3101, "[]")?;
        Ok(())
    })
    .await;
    db
}

fn whatsapp_op(id: i64) -> CreateMessagingAdOperationInput {
    CreateMessagingAdOperationInput {
        id,
        workspace_id: WS,
        channel: "whatsapp".to_owned(),
        integration_whatsapp_id: Some(21),
        integration_messenger_id: None,
        integration_instagram_id: None,
        ad_account_id: "act_9".to_owned(),
        name: "My ad".to_owned(),
        input: ad_input(),
        created_by: None,
    }
}

/// Port of: packages/database/__tests__/messaging-ad-operation-repository.test.ts:49 "messagingAdOperationRepository.create > inserts the caller-supplied operationId and the input snapshot"
#[tokio::test]
async fn messaging_ad_operation_repository_create_inserts_the_caller_supplied_operation_id_and_the_input_snapshot()
 {
    let db = seeded().await;
    let row = ok(repo::write(&db, |tx| create(tx, &whatsapp_op(777))).await);
    assert_eq!(row.id, 777);
    assert_eq!(row.workspace_id, WS);
    assert_eq!(row.input, ad_input());
    assert_eq!(row.integration_whatsapp_id, Some(21));
}

/// Port of: packages/database/__tests__/messaging-ad-operation-repository.test.ts:81 "messagingAdOperationRepository.create > throws when the insert returns no row"
///
/// A real insert either returns its row or fails: the failing insert here is a second
/// operation with the same id.
#[tokio::test]
async fn messaging_ad_operation_repository_create_throws_when_the_insert_returns_no_row() {
    let db = seeded().await;
    ok(repo::write(&db, |tx| create(tx, &whatsapp_op(777))).await);
    let second = repo::write(&db, |tx| {
        let mut op = whatsapp_op(777);
        op.channel = "messenger".to_owned();
        op.integration_whatsapp_id = None;
        op.integration_messenger_id = Some(11);
        create(tx, &op)
    })
    .await;
    assert!(!err_message(second).is_empty());
}

/// Port of: packages/database/__tests__/messaging-ad-operation-repository.test.ts:111 "messagingAdOperationRepository.updateCreateProgress > only sets the meta ids that are provided"
#[tokio::test]
async fn messaging_ad_operation_repository_update_create_progress_only_sets_the_meta_ids_that_are_provided()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| create(tx, &whatsapp_op(777))).await);
    ok(repo::write(&db, |tx| {
        update_create_progress(
            tx,
            &UpdateCreateProgressInput {
                id: 777,
                workspace_id: WS,
                create_state: "adSetCreated".to_owned(),
                meta_ad_set_id: Some("set_0".to_owned()),
                ..Default::default()
            },
        )
    })
    .await);
    let row = ok(repo::write(&db, |tx| {
        update_create_progress(
            tx,
            &UpdateCreateProgressInput {
                id: 777,
                workspace_id: WS,
                create_state: "campaignCreated".to_owned(),
                meta_campaign_id: Some("camp_1".to_owned()),
                ..Default::default()
            },
        )
    })
    .await)
    .expect("row");
    assert_eq!(row.create_state, "campaignCreated");
    assert_eq!(row.meta_campaign_id.as_deref(), Some("camp_1"));
    // Not provided: the earlier value stays.
    assert_eq!(row.meta_ad_set_id.as_deref(), Some("set_0"));
    assert_eq!(row.meta_ad_id, None);
}
