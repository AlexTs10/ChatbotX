//! Port of `packages/business/__tests__/tag-service-create.test.ts`.
//! Adapted: the mocked database becomes real `"Tag"` / `"Folder"` rows; the
//! `ensureExists` spy is the folder check's outcome. `enqueueCreate` (tag
//! sync), the `createId` spy and the template edit-lock tests are n-a.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::tag as svc;
use common::{OTHER_WS, WS, folder, scalar_i64, setup, tag, two_workspaces};

async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder(tx, 50, WS, "tag")?;
        folder(tx, 51, WS, "tag")?;
        folder(tx, 60, WS, "customField")?;
        folder(tx, 70, OTHER_WS, "tag")?;
        tag(tx, 1, WS, "vip")?;
        tx.execute(r#"UPDATE "Tag" SET "folderId" = 50 WHERE "id" = 1"#, [])?;
        Ok(())
    })
    .await;
    db
}

fn assert_code(error: BusinessError, code: &str, message: &str) {
    match error {
        BusinessError::Exception {
            code: got,
            message: got_message,
            ..
        } => {
            assert_eq!(got, code);
            assert_eq!(got_message, message);
        }
        other => panic!("unexpected {other:?}"),
    }
}

async fn tag_count(db: &chatbotx_db::Db) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "Tag" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

#[tokio::test]
async fn throws_a_validation_exception_when_name_already_exists_no_insert_no_enqueue_create() {
    let db = fixture().await;
    let error = svc::create(&db, WS, "vip", None).await.expect_err("taken");
    assert_code(error, "validation", "Name is already taken.");
    assert_eq!(tag_count(&db).await, 1);
}

#[tokio::test]
async fn calls_folder_service_ensure_exists_when_folder_id_provided() {
    let db = fixture().await;
    let created = svc::create(&db, WS, "in-folder", Some(51))
        .await
        .expect("create");
    assert_eq!(created.folder_id, Some(51));
    for folder_id in [60, 70, 999] {
        let error = svc::create(&db, WS, "bad-folder", Some(folder_id))
            .await
            .expect_err("folder check");
        assert_code(error, "notFound", "Folder not found");
    }
}

#[tokio::test]
async fn does_not_call_ensure_exists_when_folder_id_is_absent() {
    let db = fixture().await;
    let created = svc::create(&db, WS, "rooted", None).await.expect("create");
    assert_eq!(created.folder_id, None);
    assert_eq!(created.name, "rooted");
    assert_eq!(created.workspace_id, WS);
}

#[tokio::test]
async fn create_treats_folder_id_0_as_the_root() {
    let db = fixture().await;
    let created = svc::create(&db, WS, "root-zero", Some(0))
        .await
        .expect("create");
    assert_eq!(created.folder_id, None);
}

#[tokio::test]
async fn keeps_the_folder_when_folder_id_is_omitted() {
    let db = fixture().await;
    let updated = svc::update(&db, WS, 1, "vip2", None).await.expect("update");
    assert_eq!(updated.name, "vip2");
    assert_eq!(updated.folder_id, Some(50));
}

#[tokio::test]
async fn validates_and_moves_the_tag_to_the_given_folder() {
    let db = fixture().await;
    let moved = svc::update(&db, WS, 1, "vip", Some(Some(51)))
        .await
        .expect("move");
    assert_eq!(moved.folder_id, Some(51));
    let error = svc::update(&db, WS, 1, "vip", Some(Some(60)))
        .await
        .expect_err("customField folder");
    assert_code(error, "notFound", "Folder not found");
}

#[tokio::test]
async fn moves_the_tag_to_the_root_for_null_or_0() {
    let db = fixture().await;
    let root = svc::update(&db, WS, 1, "vip", Some(None))
        .await
        .expect("null");
    assert_eq!(root.folder_id, None);
    svc::update(&db, WS, 1, "vip", Some(Some(50)))
        .await
        .expect("back");
    let zero = svc::update(&db, WS, 1, "vip", Some(Some(0)))
        .await
        .expect("0");
    assert_eq!(zero.folder_id, None);
}
