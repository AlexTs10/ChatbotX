//! Port of `packages/business/__tests__/custom-field-service-update.test.ts`.
//! Adapted: the mocked database becomes real `"CustomField"` rows; the
//! `set` spy is the stored row after the update. The name-lock and the
//! injected database error tests are n-a (no advisory locks, one SQLite
//! writer; the error is mock-only).

mod common;

use chatbotx_business::custom_field::{self as svc, UpdateCustomField};
use common::{WS, custom_field, folder, setup, two_workspaces};

async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder(tx, 50, WS, "customField")?;
        custom_field(tx, 1, WS, "a", "shortText")?;
        custom_field(tx, 2, WS, "taken", "shortText")?;
        tx.execute(
            r#"UPDATE "CustomField" SET "folderId" = 50 WHERE "id" = 1"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

fn rename(name: &str) -> UpdateCustomField {
    UpdateCustomField {
        name: Some(name.to_string()),
        ..UpdateCustomField::default()
    }
}

#[tokio::test]
async fn a_rename_onto_a_name_in_use_is_a_validation_error_on_name() {
    let db = fixture().await;
    let error = svc::update(&db, WS, "1", rename("taken"))
        .await
        .expect_err("name in use");
    match error {
        chatbotx_business::BusinessError::Exception {
            code,
            field,
            message,
            ..
        } => {
            assert_eq!(code, "validation");
            assert_eq!(field.as_deref(), Some("name"));
            assert_eq!(message, "Name is already taken");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn returns_the_updated_row() {
    let db = fixture().await;
    let updated = svc::update(&db, WS, "1", rename("b"))
        .await
        .expect("update");
    assert_eq!(updated.name, "b");
    assert_eq!(updated.id, 1);
}

#[tokio::test]
async fn keeps_the_folder_when_folder_id_is_omitted() {
    let db = fixture().await;
    let updated = svc::update(&db, WS, "1", rename("a"))
        .await
        .expect("update");
    assert_eq!(updated.folder_id, Some(50));
    let stored = svc::find_by_key_or_fail(&db, WS, "1")
        .await
        .expect("stored");
    assert_eq!(stored.folder_id, Some(50));
}

#[tokio::test]
async fn stores_the_root_for_0() {
    let db = fixture().await;
    let data = UpdateCustomField {
        folder_id: Some(Some(0)),
        ..rename("a")
    };
    let updated = svc::update(&db, WS, "1", data).await.expect("update");
    assert_eq!(updated.folder_id, None);
    let stored = svc::find_by_key_or_fail(&db, WS, "1")
        .await
        .expect("stored");
    assert_eq!(stored.folder_id, None);
}
