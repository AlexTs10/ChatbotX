//! Port of `packages/business/__tests__/reflink-service.test.ts`.
//! Adapted: the mocked repository, inbox / media services and freeze check
//! become real rows; "update was called with" becomes the stored row. The lock
//! test is n-a (no template locks in the Rust port).

mod common;

use chatbotx_business::reflink::{self as svc, ReflinkData, ReflinkListInput, WidgetSettings};
use common::{
    OTHER_WS, WS, flow, inbox, media_file, reflink_row, scalar_i64, scalar_text, setup,
    two_workspaces,
};

async fn base() -> common::TestDb {
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

fn named(name: &str) -> ReflinkData {
    ReflinkData {
        name: Some(name.to_string()),
        ..ReflinkData::default()
    }
}

#[tokio::test]
async fn reflink_service_list_paginates_via_the_repository_and_computes_page_count() {
    let db = base().await;
    setup(&db, |tx| {
        for id in 50..55 {
            reflink_row(tx, id, WS, "refLink", &format!("link{id}"), 1)?;
        }
        reflink_row(tx, 60, OTHER_WS, "refLink", "foreign", 9)?;
        Ok(())
    })
    .await;
    let page = svc::list(
        &db,
        ReflinkListInput {
            workspace_id: WS,
            page: 1,
            per_page: 2,
            ..ReflinkListInput::default()
        },
    )
    .await
    .expect("list");
    assert_eq!((page.data.len(), page.page_count), (2, 3));
    assert!(page.data.iter().all(|r| r.reflink.workspace_id == WS));
}

#[tokio::test]
async fn reflink_service_find_or_fail_returns_the_row_when_found() {
    let db = base().await;
    setup(&db, |tx| reflink_row(tx, 50, WS, "refLink", "promo", 1)).await;
    let row = svc::find_or_fail(&db, WS, 50).await.expect("found");
    assert_eq!((row.id, row.name.as_str()), (50, "promo"));
}

#[tokio::test]
async fn reflink_service_find_or_fail_throws_not_found_when_no_row_matches() {
    let db = base().await;
    let error = svc::find_or_fail(&db, WS, 404).await.expect_err("missing");
    assert_eq!(error.message(), "Reflink not found");
}

#[tokio::test]
async fn reflink_service_create_inserts_scoped_to_the_workspace_with_type_ref_link() {
    let db = base().await;
    let row = svc::create(
        &db,
        WS,
        ReflinkData {
            flow_id: Some(1),
            ..named("SummerPromo")
        },
    )
    .await
    .expect("create");
    assert_eq!(
        (
            row.workspace_id,
            row.r#type.as_str(),
            row.name.as_str(),
            row.flow_id
        ),
        (WS, "refLink", "SummerPromo", 1)
    );
}

#[tokio::test]
async fn reflink_service_create_maps_a_unique_violation_on_name_into_a_field_scoped_validation_error()
 {
    let db = base().await;
    let input = ReflinkData {
        flow_id: Some(1),
        ..named("SummerPromo")
    };
    svc::create(&db, WS, input.clone()).await.expect("first");
    let error = svc::create(&db, WS, input).await.expect_err("duplicate");
    assert_eq!(
        (error.code(), error.message()),
        (Some("validation"), "Name is already taken".to_string())
    );
}

#[tokio::test]
async fn reflink_service_update_verifies_existence_then_updates() {
    let db = base().await;
    setup(&db, |tx| reflink_row(tx, 50, WS, "refLink", "OldName", 1)).await;
    let row = svc::update(&db, WS, 50, named("NewName"))
        .await
        .expect("update");
    assert_eq!((row.id, row.name.as_str(), row.flow_id), (50, "NewName", 1));
}

#[tokio::test]
async fn reflink_service_update_throws_not_found_before_updating_when_the_row_does_not_exist() {
    let db = base().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, OTHER_WS, "refLink", "Foreign", 9)
    })
    .await;
    let error = svc::update(&db, WS, 50, named("x"))
        .await
        .expect_err("foreign");
    assert_eq!(error.message(), "Reflink not found");
    let name = scalar_text(&db, r#"SELECT "name" FROM "Reflink" WHERE "id" = ?1"#, 50).await;
    assert_eq!(name.as_deref(), Some("Foreign"));
}

#[tokio::test]
async fn reflink_service_update_maps_a_unique_violation_on_name_into_a_field_scoped_validation_error()
 {
    let db = base().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "refLink", "OldName", 1)?;
        reflink_row(tx, 51, WS, "refLink", "Taken", 1)
    })
    .await;
    let error = svc::update(&db, WS, 50, named("Taken"))
        .await
        .expect_err("duplicate");
    assert_eq!(
        (error.code(), error.message()),
        (Some("validation"), "Name is already taken".to_string())
    );
}

// --- updateWidgetSettings --------------------------------------------------

fn branding() -> WidgetSettings {
    WidgetSettings {
        logo_file_id: Some(70),
        brand_name: "Shop".into(),
        brand_url: "https://shop.test".into(),
        logo_background_color: "#2563eb".into(),
        ..WidgetSettings::default()
    }
}

async fn widget_base() -> common::TestDb {
    let db = base().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "refLink", "promo", 1)?;
        inbox(tx, 80, WS, "messenger")?;
        inbox(tx, 81, OTHER_WS, "messenger")?;
        media_file(tx, 70, WS, "image/png")?;
        media_file(tx, 71, WS, "application/pdf")?;
        media_file(tx, 72, OTHER_WS, "image/png")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn reflink_service_update_widget_settings_folds_domain_case_and_duplicates_and_keeps_only_this_workspace_s_inboxes()
 {
    let db = widget_base().await;
    let row = svc::update_widget_settings(
        &db,
        WS,
        50,
        WidgetSettings {
            authorized_domains: vec![
                "Example.COM".into(),
                "example.com".into(),
                "shop.test".into(),
            ],
            hidden_inbox_ids: vec![80, 80, 81],
            ..branding()
        },
    )
    .await
    .expect("update");
    assert_eq!(
        row.widget_authorized_domains.0,
        serde_json::json!(["example.com", "shop.test"])
    );
    assert_eq!(row.widget_hidden_inbox_ids.0, serde_json::json!(["80"]));
    assert_eq!(
        (
            row.widget_logo_file_id,
            row.widget_brand_name.as_deref(),
            row.widget_brand_url.as_deref(),
            row.widget_logo_background_color.as_deref()
        ),
        (
            Some(70),
            Some("Shop"),
            Some("https://shop.test"),
            Some("#2563eb")
        )
    );
}

#[tokio::test]
async fn reflink_service_update_widget_settings_stores_no_logo_brand_name_or_url_when_they_are_empty()
 {
    let db = widget_base().await;
    let row = svc::update_widget_settings(
        &db,
        WS,
        50,
        WidgetSettings {
            logo_background_color: "#111827".into(),
            ..WidgetSettings::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(
        (
            row.widget_logo_file_id,
            row.widget_brand_name,
            row.widget_brand_url
        ),
        (None, None, None)
    );
}

#[tokio::test]
async fn reflink_service_update_widget_settings_rejects_a_logo_file_s() {
    let db = widget_base().await;
    for file in [72, 71] {
        let error = svc::update_widget_settings(
            &db,
            WS,
            50,
            WidgetSettings {
                logo_file_id: Some(file),
                ..branding()
            },
        )
        .await
        .expect_err("bad logo");
        assert_eq!(error.message(), "Logo must be an image", "{file}");
    }
    let stored = scalar_i64(
        &db,
        r#"SELECT "widgetLogoFileId" FROM "Reflink" WHERE "id" = ?1"#,
        50,
    )
    .await;
    assert_eq!(stored, None);
}

#[tokio::test]
async fn reflink_service_update_widget_settings_a_changed_logo_is_the_installer_s_no_template_edit_guard_it_is_written()
 {
    let db = widget_base().await;
    setup(&db, |tx| {
        media_file(tx, 73, WS, "image/png")?;
        tx.execute(
            r#"UPDATE "Reflink" SET "widgetLogoFileId" = 73 WHERE "id" = 50"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let row = svc::update_widget_settings(&db, WS, 50, branding())
        .await
        .expect("update");
    assert_eq!(row.widget_logo_file_id, Some(70));
}

#[tokio::test]
async fn reflink_service_update_widget_settings_an_unchanged_logo_takes_no_guard() {
    let db = widget_base().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Reflink" SET "widgetLogoFileId" = 70 WHERE "id" = 50"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let row = svc::update_widget_settings(&db, WS, 50, branding())
        .await
        .expect("update");
    assert_eq!(row.widget_logo_file_id, Some(70));
}

#[tokio::test]
async fn reflink_service_update_widget_settings_throws_not_found_for_a_ref_link_outside_the_workspace()
 {
    let db = widget_base().await;
    let error = svc::update_widget_settings(&db, OTHER_WS, 50, WidgetSettings::default())
        .await
        .expect_err("foreign");
    assert_eq!(error.message(), "Reflink not found");
}

// --- findForWidget ------------------------------------------------------------

#[tokio::test]
async fn reflink_service_find_for_widget_returns_the_ref_link_when_its_workspace_is_active() {
    let db = base().await;
    setup(&db, |tx| reflink_row(tx, 50, WS, "refLink", "promo", 1)).await;
    let found = svc::find_for_widget(&db, 50)
        .await
        .expect("find")
        .expect("row");
    assert_eq!((found.reflink.id, found.reflink.workspace_id), (50, WS));
}

#[tokio::test]
async fn reflink_service_find_for_widget_returns_null_when_the_workspace_is_frozen_or_the_link_is_gone()
 {
    let db = base().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "refLink", "promo", 1)?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    assert!(
        svc::find_for_widget(&db, 50)
            .await
            .expect("frozen")
            .is_none()
    );
    assert!(
        svc::find_for_widget(&db, 404)
            .await
            .expect("gone")
            .is_none()
    );
}

// --- deleteInTx -----------------------------------------------------------------

async fn both_kinds() -> common::TestDb {
    let db = base().await;
    setup(&db, |tx| {
        reflink_row(tx, 50, WS, "refLink", "link", 1)?;
        reflink_row(tx, 51, WS, "qrCode", "qr_code", 1)
    })
    .await;
    db
}

#[tokio::test]
async fn reflink_service_delete_in_tx_a_given_type_scopes_the_delete_to_it_so_a_qr_code_delete_never_takes_a_ref_link()
 {
    let db = both_kinds().await;
    let deleted = chatbotx_business::tx::write(&db, |tx| {
        svc::delete_in_tx(tx, WS, &[50, 51], Some("qrCode"))
    })
    .await
    .expect("delete");
    assert_eq!(deleted, vec![51]);
}

#[tokio::test]
async fn reflink_service_delete_in_tx_without_a_type_the_uninstall_both_kinds_sharing_the_table_are_matched()
 {
    let db = both_kinds().await;
    let mut deleted =
        chatbotx_business::tx::write(&db, |tx| svc::delete_in_tx(tx, WS, &[50, 51], None))
            .await
            .expect("delete");
    deleted.sort_unstable();
    assert_eq!(deleted, vec![50, 51]);
}
