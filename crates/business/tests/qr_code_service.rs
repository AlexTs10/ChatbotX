//! Port of `packages/business/__tests__/qr-code.service.test.ts`.
//! Adapted: the mocked database and reflink core become real `"Reflink"`
//! rows. The cache and template-lock tests are n-a (no read cache, no locks).

mod common;

use chatbotx_business::qr_code::{self as svc, ListQrCodes, QrCodeData};
use common::{OTHER_WS, WS, flow, reflink_row, scalar_i64, scalar_text, setup, two_workspaces};

const DUPLICATE: &str = "Name already exists";

fn data(name: &str, flow_id: i64) -> QrCodeData {
    QrCodeData {
        name: Some(name.to_string()),
        flow_id: Some(flow_id),
        size: Some(256),
        ..QrCodeData::default()
    }
}

async fn flows() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, 1, WS)?;
        flow(tx, 2, WS)?;
        flow(tx, 9, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

// --- create -----------------------------------------------------------------

#[tokio::test]
async fn qr_code_service_create_prefixes_the_name_with_qr_and_writes_qr_styles_from_size() {
    let db = flows().await;
    let id = svc::create(&db, WS, data("my-code", 1), DUPLICATE)
        .await
        .expect("create");
    let name = scalar_text(&db, r#"SELECT "name" FROM "Reflink" WHERE "id" = ?1"#, id).await;
    let styles = scalar_text(
        &db,
        r#"SELECT "qrStyles" FROM "Reflink" WHERE "id" = ?1"#,
        id,
    )
    .await;
    let kind = scalar_text(&db, r#"SELECT "type" FROM "Reflink" WHERE "id" = ?1"#, id).await;
    assert_eq!(
        (name.as_deref(), kind.as_deref()),
        (Some("qr_my-code"), Some("qrCode"))
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&styles.expect("qrStyles")).expect("json"),
        serde_json::json!({ "size": 256 })
    );
}

#[tokio::test]
async fn qr_code_service_create_maps_a_unique_violation_to_a_validation_exception_carrying_the_passed_message()
 {
    let db = flows().await;
    svc::create(&db, WS, data("my-code", 1), DUPLICATE)
        .await
        .expect("first");
    let error = svc::create(&db, WS, data("my-code", 1), DUPLICATE)
        .await
        .expect_err("duplicate");
    assert_eq!(
        (error.code(), error.status(), error.message()),
        (Some("validation"), Some(422), DUPLICATE.to_string())
    );
}

#[tokio::test]
async fn qr_code_service_create_returns_the_generated_id() {
    let db = flows().await;
    let id = svc::create(&db, WS, data("my-code", 1), DUPLICATE)
        .await
        .expect("create");
    let found = svc::find(&db, WS, id).await.expect("find").expect("row");
    assert_eq!(found.id, id);
}

#[tokio::test]
async fn qr_code_service_create_rejects_when_flow_id_does_not_belong_to_the_workspace() {
    let db = flows().await;
    let error = svc::create(&db, WS, data("my-code", 9), DUPLICATE)
        .await
        .expect_err("foreign flow");
    assert_eq!(
        (error.code(), error.message()),
        (Some("notFound"), "Flow not found".to_string())
    );
    let count = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "Reflink" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(count, Some(0));
}

// --- update -----------------------------------------------------------------

#[tokio::test]
async fn qr_code_service_update_returns_the_updated_row_scoped_to_workspace_id_and_the_qr_code_type()
 {
    let db = flows().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "qrCode", "qr_old", 1)?;
        reflink_row(tx, 51, WS, "refLink", "reflink", 1)?;
        Ok(())
    })
    .await;
    let rename = QrCodeData {
        name: Some("renamed".into()),
        ..QrCodeData::default()
    };
    let row = svc::update(&db, WS, 50, rename.clone(), DUPLICATE)
        .await
        .expect("update");
    assert_eq!((row.id, row.name.as_str()), (50, "qr_renamed"));
    let error = svc::update(&db, WS, 51, rename.clone(), DUPLICATE)
        .await
        .expect_err("ref link");
    assert_eq!(error.message(), "QR Code not found");
    let error = svc::update(&db, OTHER_WS, 50, rename, DUPLICATE)
        .await
        .expect_err("other workspace");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn qr_code_service_update_rejects_when_flow_id_does_not_belong_to_the_workspace() {
    let db = flows().await;
    setup(&db, |tx| reflink_row(tx, 50, WS, "qrCode", "qr_old", 1)).await;
    let error = svc::update(
        &db,
        WS,
        50,
        QrCodeData {
            flow_id: Some(9),
            ..QrCodeData::default()
        },
        DUPLICATE,
    )
    .await
    .expect_err("foreign flow");
    assert_eq!(
        (error.code(), error.message()),
        (
            Some("validation"),
            "Flow not found in this workspace".to_string()
        )
    );
    let flow_id = scalar_i64(&db, r#"SELECT "flowId" FROM "Reflink" WHERE "id" = ?1"#, 50).await;
    assert_eq!(flow_id, Some(1));
}

// --- deleteMany ---------------------------------------------------------------

#[tokio::test]
async fn qr_code_service_delete_many_deletes_through_the_shared_reflink_core_scoped_to_the_qr_code_type()
 {
    let db = flows().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "qrCode", "qr_a", 1)?;
        reflink_row(tx, 51, WS, "refLink", "b", 1)?;
        Ok(())
    })
    .await;
    svc::delete_many(&db, WS, vec![50, 51])
        .await
        .expect("delete");
    let left = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "Reflink" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    let kept = scalar_i64(&db, r#"SELECT "id" FROM "Reflink" WHERE "id" = ?1"#, 51).await;
    assert_eq!((left, kept), (Some(1), Some(51)));
}

// --- list ---------------------------------------------------------------------

#[tokio::test]
async fn qr_code_service_list_scoping_and_cache_tags_list_filters_by_workspace_id_and_the_qr_code_type_discriminator()
 {
    let db = flows().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "qrCode", "qr_a", 1)?;
        reflink_row(tx, 51, WS, "refLink", "b", 1)?;
        reflink_row(tx, 52, OTHER_WS, "qrCode", "qr_c", 9)?;
        Ok(())
    })
    .await;
    let page = svc::list(
        &db,
        ListQrCodes {
            workspace_id: WS,
            per_page: 10,
            ..ListQrCodes::default()
        },
    )
    .await
    .expect("list");
    let ids: Vec<(i64, i64)> = page
        .data
        .iter()
        .map(|r| (r.qr_code.id, r.flow.id))
        .collect();
    assert_eq!((ids, page.page_count), (vec![(50, 1)], 1));
}

// --- find -----------------------------------------------------------------------

#[tokio::test]
async fn qr_code_service_find_filters_by_workspace_id_id_and_the_qr_code_type_discriminator() {
    let db = flows().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "qrCode", "qr_a", 1)?;
        reflink_row(tx, 51, WS, "refLink", "b", 1)?;
        Ok(())
    })
    .await;
    assert!(svc::find(&db, WS, 50).await.expect("find").is_some());
    assert!(svc::find(&db, WS, 51).await.expect("find").is_none());
    assert!(svc::find(&db, OTHER_WS, 50).await.expect("find").is_none());
}
