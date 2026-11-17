//! Q.D4 slice 1 ports:
//! - `integration/template-db.test.ts` "template lock precedence on a real
//!   database" (findBlocking): real rows; `excludeInstallationIds` belongs to
//!   the uninstall engine (not ported), so that assertion is left out.
//! - `template-service-authorization.test.ts`: softDelete / share / rotate
//!   guards, on real rows instead of the mocked repository.

mod common;

use chatbotx_business::template::permissions::Actor;
use chatbotx_business::template::{self, Blocking, installations, service};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::DbError;
use common::{OTHER_WS, WS, scalar_i64, setup, two_workspaces};
use serde_json::json;

fn flows_actor() -> Actor {
    Actor::Member(json!({ "flows": true }))
}

fn no_manage_actor() -> Actor {
    Actor::Member(json!({ "ecommerce": true }))
}

fn insert_template(tx: &Connection, id: i64, workspace_id: i64, created_at: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Template" ("id", "createdAt", "workspaceId", "tenantId", "name", "selection",
             "payload", "categoryCounts", "formatVersion", "shareToken", "defaultPermissions")
           VALUES (?1, ?3, ?2, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?2), ?4, '{}', '{}',
             '{"flows":1}', 1, ?5, '{"allowEdit":true,"allowDelete":false}')"#,
        params![id, workspace_id, created_at, format!("Template {id}"), format!("token-{id}")],
    )?;
    Ok(id)
}

fn insert_installation(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    template_id: i64,
    status: &str,
    source_deleted_at: Option<i64>,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateId", "templateName",
             "sourceWorkspaceId", "formatVersion", "status", "permissions", "sourceTemplateId",
             "sourceDeletedAt", "successfulInstallEvents")
           VALUES (?1, ?2, ?3, 'T', ?4, 1, ?5, '{}', ?3, ?6, 1)"#,
        params![id, workspace_id, template_id, OTHER_WS, status, source_deleted_at],
    )?;
    Ok(id)
}

fn track(
    tx: &Connection,
    installation: i64,
    source: &str,
    allow_edit: bool,
    was_existing: bool,
    detached_at: Option<i64>,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "TemplateInstalledResource" ("id", "installationId", "workspaceId", "category",
             "resourceKind", "resourceId", "sourceResourceId", "wasExisting", "allowEdit", "detachedAt")
           VALUES (?1, ?2, ?3, 'flows', 'flow', 9001, ?4, ?5, ?6, ?7)"#,
        params![
            chatbotx_db::repo::new_id(),
            installation,
            WS,
            source,
            was_existing,
            allow_edit,
            detached_at
        ],
    )?;
    Ok(())
}

async fn locked(db: &chatbotx_db::Db) -> Vec<String> {
    template::list_blocking_ids(db, WS, "flow", &[9001], Blocking::Edit)
        .await
        .expect("findBlocking")
}

#[tokio::test]
async fn the_most_restrictive_of_several_installations_wins() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, OTHER_WS, 1)?;
        insert_installation(tx, 11, WS, 1, "completed", None)?;
        insert_installation(tx, 12, WS, 1, "completed", None)?;
        track(tx, 11, "src-1", true, false, None)
    })
    .await;
    assert!(locked(&db).await.is_empty());
    setup(&db, |tx| track(tx, 12, "src-1", false, false, None)).await;
    assert_eq!(locked(&db).await, vec!["9001".to_string()]);
}

#[tokio::test]
async fn a_reused_detached_or_source_deleted_row_never_locks() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, OTHER_WS, 1)?;
        insert_installation(tx, 11, WS, 1, "completed", None)?;
        track(tx, 11, "src-1", false, true, None)
    })
    .await;
    assert!(locked(&db).await.is_empty());
    setup(&db, |tx| {
        insert_installation(tx, 12, WS, 1, "completed", None)?;
        track(tx, 12, "src-2", false, false, Some(5))
    })
    .await;
    assert!(locked(&db).await.is_empty());
    setup(&db, |tx| {
        insert_installation(tx, 13, WS, 1, "completed", Some(5))?;
        track(tx, 13, "src-3", false, false, None)
    })
    .await;
    assert!(locked(&db).await.is_empty());
}

#[tokio::test]
async fn undeletable_ids_read_allow_delete() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, OTHER_WS, 1)?;
        insert_installation(tx, 11, WS, 1, "completed", None)?;
        track(tx, 11, "src-1", true, false, None)?;
        tx.execute(r#"UPDATE "TemplateInstalledResource" SET "allowDelete" = 0"#, [])?;
        Ok(())
    })
    .await;
    let (locked, undeletable) = template::list_locked_resource_ids(&db, WS, "flow", &[9001, 9002])
        .await
        .expect("lists");
    assert!(locked.is_empty());
    assert_eq!(undeletable, vec!["9001".to_string()]);
    let (none, _) = template::list_locked_resource_ids(&db, WS, "flow", &[]).await.expect("empty");
    assert!(none.is_empty());
}

#[tokio::test]
async fn soft_delete_cannot_delete_an_already_deleted_template() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, WS, 1)?;
        insert_installation(tx, 11, OTHER_WS, 1, "completed", None)
    })
    .await;
    service::soft_delete(&db, WS, 1, &flows_actor()).await.expect("delete");
    // The installations are stamped in the same transaction.
    assert!(
        scalar_i64(&db, r#"SELECT "sourceDeletedAt" FROM "TemplateInstallation" WHERE "id" = ?1"#, 11)
            .await
            .is_some()
    );
    let error = service::soft_delete(&db, WS, 1, &flows_actor()).await.unwrap_err();
    assert_eq!(error.message(), "Template not found");
}

#[tokio::test]
async fn writes_reject_an_actor_who_cannot_manage_templates() {
    let db = two_workspaces().await;
    setup(&db, |tx| insert_template(tx, 1, WS, 1)).await;
    for actor in [no_manage_actor(), Actor::ApiToken { full: false }] {
        let errors = [
            service::soft_delete(&db, WS, 1, &actor).await.unwrap_err(),
            service::update_share_settings(&db, WS, 1, true, None, &actor)
                .await
                .unwrap_err(),
            service::rotate_share_token(&db, WS, 1, &actor).await.unwrap_err(),
        ];
        for error in errors {
            assert_eq!(error.code(), Some("templateManagePermissionDenied"));
        }
    }
}

#[tokio::test]
async fn rotate_share_token_replaces_the_token_of_a_live_template() {
    let db = two_workspaces().await;
    setup(&db, |tx| insert_template(tx, 1, WS, 1)).await;
    let rotated = service::rotate_share_token(&db, WS, 1, &flows_actor()).await.expect("rotate");
    assert_ne!(rotated.share_token, "token-1");
    let other = service::rotate_share_token(&db, OTHER_WS, 1, &flows_actor()).await.unwrap_err();
    assert_eq!(other.code(), Some("notFound"));
}

#[tokio::test]
async fn update_share_settings_keeps_or_clears_the_expiry() {
    let db = two_workspaces().await;
    setup(&db, |tx| insert_template(tx, 1, WS, 1)).await;
    let actor = flows_actor();
    let set = service::update_share_settings(&db, WS, 1, true, Some(Some(99)), &actor).await.expect("set");
    assert_eq!((set.share_enabled, set.share_expires_at), (true, Some(99)));
    let kept = service::update_share_settings(&db, WS, 1, false, None, &actor).await.expect("keep");
    assert_eq!((kept.share_enabled, kept.share_expires_at), (false, Some(99)));
    let cleared = service::update_share_settings(&db, WS, 1, true, Some(None), &actor).await.expect("clear");
    assert_eq!(cleared.share_expires_at, None);
}

#[tokio::test]
async fn list_page_pages_newest_first_with_active_install_counts() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, WS, 10)?;
        insert_template(tx, 2, WS, 20)?;
        insert_template(tx, 3, WS, 20)?;
        insert_installation(tx, 11, OTHER_WS, 3, "completed", None)?;
        insert_installation(tx, 12, OTHER_WS, 3, "uninstalled", None)
    })
    .await;
    let first = service::list_page(&db, WS, None, Some(2)).await.expect("first");
    let ids: Vec<_> = first.items.iter().map(|(t, n)| (t.id, *n)).collect();
    assert_eq!(ids, vec![(3, 1), (2, 0)]);
    let second = service::list_page(&db, WS, first.next_cursor, Some(2)).await.expect("second");
    assert_eq!(second.items.iter().map(|(t, _)| t.id).collect::<Vec<_>>(), vec![1]);
    assert_eq!(second.next_cursor, None);
    let bad = service::list_page(&db, WS, Some("100".into()), None).await.unwrap_err();
    assert_eq!(bad.code(), Some("invalidCursor"));
}

#[tokio::test]
async fn auto_update_follows_the_installation_state() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        insert_template(tx, 1, OTHER_WS, 1)?;
        insert_installation(tx, 11, WS, 1, "completed", None)?;
        insert_installation(tx, 12, WS, 1, "installing", None)?;
        insert_installation(tx, 13, WS, 1, "completed", Some(5))?;
        track(tx, 11, "src-1", true, false, None)
    })
    .await;
    let actor = flows_actor();
    installations::set_auto_update(&db, WS, 11, &actor, true).await.expect("on");
    assert_eq!(
        scalar_i64(&db, r#"SELECT "autoUpdate" FROM "TemplateInstallation" WHERE "id" = ?1"#, 11).await,
        Some(1)
    );
    let busy = installations::set_auto_update(&db, WS, 12, &actor, true).await.unwrap_err();
    assert_eq!(busy.code(), Some("templateInstallationNotReady"));
    let gone = installations::set_auto_update(&db, WS, 13, &actor, true).await.unwrap_err();
    assert_eq!(gone.code(), Some("templateInstallationNotSyncable"));
    installations::set_auto_update(&db, WS, 13, &actor, false).await.expect("off stays allowed");
    let denied = installations::set_auto_update(&db, WS, 11, &no_manage_actor(), false)
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Some("templateCategoryPermissionDenied"));
    let read_only = installations::set_auto_update(&db, WS, 11, &Actor::ApiToken { full: false }, false)
        .await
        .unwrap_err();
    assert_eq!(read_only.code(), Some("templateManagePermissionDenied"));
}
