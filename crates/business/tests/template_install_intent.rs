//! Q.D4 follow-up: the template install intent (`install-intent.ts`, K35
//! durable `take`), `stageSharedTemplateInstall` and `dispatchStagedInstalls`.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::template::install_intent::{
    clear, dispatch_staged_installs, remember, stage_shared_install, take,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_kv::{DurableKv, SystemClock};
use common::{OTHER_WS, WS, setup, two_workspaces};
use x1_support::RecordingEnqueuer;

async fn kv() -> DurableKv {
    let db = Db::open_in_memory().expect("kv db");
    db.migrate().await.expect("kv migrate");
    DurableKv::new(Arc::new(db), Arc::new(SystemClock))
}

#[tokio::test]
async fn the_intent_is_one_slot_taken_once() {
    let kv = kv().await;
    remember(&kv, 7, "old").await;
    remember(&kv, 7, "new").await;
    assert_eq!(take(&kv, 8).await, None);
    assert_eq!(take(&kv, 7).await.as_deref(), Some("new"));
    assert_eq!(take(&kv, 7).await, None);
    remember(&kv, 7, "again").await;
    clear(&kv, 7).await;
    assert_eq!(take(&kv, 7).await, None);
}

#[tokio::test]
async fn a_live_share_link_is_staged_and_dispatched() {
    let db = two_workspaces().await;
    let staged = setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Template" ("id", "workspaceId", "tenantId", "name", "selection", "payload",
                 "categoryCounts", "formatVersion", "shareToken", "shareEnabled", "defaultPermissions")
               VALUES (901, ?1, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1), 'Starter', '{}', '{}',
                 '{}', 1, 'live', 1, '{"allowEdit":true}')"#,
            params![OTHER_WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Template" ("id", "workspaceId", "tenantId", "name", "selection", "payload",
                 "categoryCounts", "formatVersion", "shareToken", "shareEnabled", "defaultPermissions")
               VALUES (902, ?1, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1), 'Off', '{}', '{}',
                 '{}', 1, 'off', 0, '{}')"#,
            params![OTHER_WS],
        )?;
        let tenant: i64 = tx.query_row(
            r#"SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1"#,
            params![WS],
            |r| r.get(0),
        )?;
        let off = stage_shared_install(tx, WS, tenant, 1001, "off")
            .map_err(|e| chatbotx_db::DbError::other(e.message()))?;
        let missing = stage_shared_install(tx, WS, tenant, 1001, "nope")
            .map_err(|e| chatbotx_db::DbError::other(e.message()))?;
        assert_eq!((off, missing), (None, None));
        Ok(stage_shared_install(tx, WS, tenant, 1001, "live")
            .map_err(|e| chatbotx_db::DbError::other(e.message()))?
            .expect("staged"))
    })
    .await;
    let row: (String, String, i64, String) = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "status", "installSource", "installedBy", "installedByKind"
                   FROM "TemplateInstallation" WHERE "id" = ?1"#,
                params![staged],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        row,
        ("pending".into(), "manual".into(), 1001, "member".into())
    );

    let jobs = RecordingEnqueuer::new();
    dispatch_staged_installs(&db, jobs.as_ref(), WS).await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "installTemplate");
    assert_eq!(calls[0].data["data"]["installationId"], staged.to_string());
    // Nothing pending elsewhere: no job.
    dispatch_staged_installs(&db, jobs.as_ref(), OTHER_WS).await;
    assert_eq!(jobs.calls().len(), 1);
}
