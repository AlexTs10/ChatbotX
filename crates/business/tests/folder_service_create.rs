//! Port of `packages/business/__tests__/folder-service-create.test.ts`
//! (P3.6-D2 slice folders). Adapted: the mocked `folderModel.findFirst` /
//! insert become stored folders; the asserted lookup `where` becomes which
//! parents are accepted and the stored `paths`.

mod common;

use chatbotx_business::folder::create;
use common::{OTHER_WS, TestDb, WS, folder, setup, two_workspaces};
use serde_json::json;

/// Folder 11 (sequence, `WS`), 12 (tag, `WS`), 13 (tag, `OTHER_WS`).
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder(tx, 11, WS, "sequence")?;
        folder(tx, 12, WS, "tag")?;
        folder(tx, 13, OTHER_WS, "tag")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn folder_service_create_parent_lookup_the_parent_must_be_a_folder_of_the_same_workspace_and_type()
 {
    let db = db().await;
    let child = create(&db, WS, "Child", "sequence", Some(11))
        .await
        .expect("create");
    assert_eq!(child.parent_id, Some(11));
    assert_eq!(child.folder_type, "sequence");
    assert_eq!(child.workspace_id, WS);
    assert_eq!(child.paths.0, json!(["11"]));
    // A parent of the same workspace but another type is not a parent.
    let error = create(&db, WS, "Child", "tag", Some(11))
        .await
        .expect_err("type");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn folder_service_create_parent_lookup_a_parent_of_another_workspace_or_type_reads_as_missing()
 {
    let db = db().await;
    for parent in [13, 11, 999] {
        let error = create(&db, WS, "Child", "tag", Some(parent))
            .await
            .expect_err("missing parent");
        assert_eq!(error.code(), Some("notFound"), "{parent}");
        assert_eq!(error.message(), "Parent folder does not exist!", "{parent}");
    }
}
