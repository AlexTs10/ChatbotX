//! Q.D8: the template guards of `couponService.updateTopic` (`withEditGuard`)
//! and `deleteTopic` (`withDeleteGuard`) on real rows.

mod common;

use chatbotx_business::coupon;
use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, setup, two_workspaces};

fn installed_topic(
    tx: &Connection,
    topic_id: i64,
    allow_edit: bool,
    allow_delete: bool,
    outcome: Option<&str>,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Template" ("id", "createdAt", "workspaceId", "tenantId", "name", "selection",
             "payload", "categoryCounts", "formatVersion", "shareToken", "defaultPermissions")
           VALUES (7, 1, ?1, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1), 'T', '{}', '{}',
             '{}', 1, 'token-7', '{}')"#,
        params![OTHER_WS],
    )?;
    tx.execute(
        r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateId", "templateName",
             "sourceWorkspaceId", "formatVersion", "status", "permissions", "sourceTemplateId",
             "successfulInstallEvents")
           VALUES (8, ?1, 7, 'T', ?2, 1, 'completed', '{}', 7, 1)"#,
        params![WS, OTHER_WS],
    )?;
    tx.execute(
        r#"INSERT INTO "TemplateInstalledResource" ("id", "installationId", "workspaceId", "category",
             "resourceKind", "resourceId", "sourceResourceId", "allowEdit", "allowDelete", "uninstallOutcome")
           VALUES (9, 8, ?1, 'couponTopics', 'couponTopic', ?2, 'src', ?3, ?4, ?5)"#,
        params![WS, topic_id, allow_edit, allow_delete, outcome],
    )?;
    Ok(())
}

fn code(error: &chatbotx_business::BusinessError) -> Option<&str> {
    error.code()
}

#[tokio::test]
async fn locked_topic_refuses_content_edits_and_delete() {
    let test = two_workspaces().await;
    let db = &*test;
    let topic = coupon::create_topic(db, WS, "Spring".into(), None, None)
        .await
        .expect("create");
    setup(db, move |tx| {
        installed_topic(tx, topic.id, false, false, None)
    })
    .await;

    let renamed = coupon::update_topic(db, WS, topic.id, Some("Summer".into()), None, None).await;
    assert_eq!(
        renamed.as_ref().err().and_then(code),
        Some("templateAllowEditViolation")
    );
    // The same name re-submitted is no change: the form saves.
    coupon::update_topic(
        db,
        WS,
        topic.id,
        Some("Spring".into()),
        Some(Some(String::new())),
        None,
    )
    .await
    .expect("unchanged save");

    coupon::set_topic_status(db, WS, topic.id, "archived")
        .await
        .expect("archive");
    let deleted = coupon::delete_topic(db, WS, topic.id).await;
    assert_eq!(
        deleted.as_ref().err().and_then(code),
        Some("templateAllowDeleteViolation")
    );
}

#[tokio::test]
async fn topic_being_uninstalled_refuses_every_write() {
    let test = two_workspaces().await;
    let db = &*test;
    let topic = coupon::create_topic(db, WS, "Spring".into(), None, None)
        .await
        .expect("create");
    setup(db, move |tx| {
        installed_topic(tx, topic.id, true, true, Some("deleting"))
    })
    .await;
    let unchanged = coupon::update_topic(db, WS, topic.id, Some("Spring".into()), None, None).await;
    assert_eq!(
        unchanged.as_ref().err().and_then(code),
        Some("templateUninstallInProgress")
    );
}

#[tokio::test]
async fn editable_template_copy_saves_and_deletes() {
    let test = two_workspaces().await;
    let db = &*test;
    let topic = coupon::create_topic(db, WS, "Spring".into(), None, None)
        .await
        .expect("create");
    setup(db, move |tx| {
        installed_topic(tx, topic.id, true, true, None)
    })
    .await;
    let updated = coupon::update_topic(db, WS, topic.id, Some("Summer".into()), None, None)
        .await
        .expect("edit allowed");
    assert_eq!(updated.name, "Summer");
    coupon::set_topic_status(db, WS, topic.id, "archived")
        .await
        .expect("archive");
    coupon::delete_topic(db, WS, topic.id)
        .await
        .expect("delete allowed");
}
