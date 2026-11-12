//! Port of `packages/database/__tests__/integration/resource-deletion-log-repository.test.ts`
//! (P3.3). Each rolled-back Postgres transaction becomes a fresh migrated SQLite file.

mod common;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::models::ResourceDeletionLog;
use chatbotx_db::repo::resource_deletion_log::{
    ResourceDeletionLogInsert, ResourceKey, cancel_pending_by_resource, complete_pending_by_id,
    find_pending_id_by_resource, insert,
};
use chatbotx_db::repo::{self, find_many, find_one};
use common::fresh_db;
use repo_common::{ms, ok};

fn build_row(workspace_id: i64, resource_id: &str, status: &str) -> ResourceDeletionLogInsert {
    ResourceDeletionLogInsert {
        workspace_id,
        resource_type: "workspace".to_owned(),
        resource_id: resource_id.to_owned(),
        resource_subtype: None,
        resource_name: None,
        status: status.to_owned(),
        requested_by_user_id: None,
        request_source: None,
        ip_address: None,
        user_agent: None,
        requested_at: ms("2026-01-01T00:00:00.000Z"),
        completed_at: None,
        canceled_at: None,
    }
}

async fn add(db: &common::TestDb, row: ResourceDeletionLogInsert) {
    ok(repo::write(db, move |tx| insert(tx, &row)).await);
}

fn key(workspace_id: i64, resource_id: &str) -> ResourceKey {
    ResourceKey {
        workspace_id,
        resource_type: "workspace".to_owned(),
        resource_id: resource_id.to_owned(),
    }
}

/// Port of: packages/database/__tests__/integration/resource-deletion-log-repository.test.ts:73 "resource deletion log repository against Postgres > findPendingIdByResource returns the newest pending row and ignores other statuses"
#[tokio::test]
async fn resource_deletion_log_repository_against_postgres_find_pending_id_by_resource_returns_the_newest_pending_row_and_ignores_other_statuses()
 {
    let db = fresh_db().await;
    let ws = repo::new_id();
    let rid = ws.to_string();
    add(&db, build_row(ws, &rid, "canceled")).await;
    add(&db, build_row(ws, &rid, "pending")).await;
    add(&db, build_row(ws, &rid, "pending")).await;
    let pending: Vec<ResourceDeletionLog> = ok(repo::read(&db, move |c| {
        find_many(
            c,
            "WHERE \"workspaceId\" = ?1 AND \"status\" = 'pending'",
            &[&ws],
        )
    })
    .await);
    let newest = pending.iter().map(|r| r.id).max().expect("pending rows");
    let k = key(ws, &rid);
    let found = ok(repo::read(&db, move |c| find_pending_id_by_resource(c, &k)).await);
    assert_eq!(found, Some(newest));
}

/// Port of: packages/database/__tests__/integration/resource-deletion-log-repository.test.ts:112 "resource deletion log repository against Postgres > completePendingById completes only a pending row, once"
#[tokio::test]
async fn resource_deletion_log_repository_against_postgres_complete_pending_by_id_completes_only_a_pending_row_once()
 {
    let db = fresh_db().await;
    let ws = repo::new_id();
    let rid = ws.to_string();
    let completed_at = ms("2026-02-01T00:00:00.000Z");
    add(&db, build_row(ws, &rid, "pending")).await;
    let k = key(ws, &rid);
    let pending_id =
        ok(repo::read(&db, move |c| find_pending_id_by_resource(c, &k)).await).expect("pending id");
    let first = ok(repo::write(&db, move |tx| {
        complete_pending_by_id(tx, pending_id, completed_at)
    })
    .await);
    let repeated = ok(repo::write(&db, move |tx| {
        complete_pending_by_id(tx, pending_id, repo::now_ms())
    })
    .await);
    assert_eq!(first, 1);
    assert_eq!(repeated, 0);
    let row: ResourceDeletionLog =
        ok(repo::read(&db, move |c| find_one(c, "\"id\" = ?1", &[&pending_id])).await)
            .expect("row");
    assert_eq!(row.status, "completed");
    assert_eq!(row.completed_at, Some(completed_at));
}

/// Port of: packages/database/__tests__/integration/resource-deletion-log-repository.test.ts:151 "resource deletion log repository against Postgres > cancelPendingByResource is scoped by workspace and leaves other statuses alone"
#[tokio::test]
async fn resource_deletion_log_repository_against_postgres_cancel_pending_by_resource_is_scoped_by_workspace_and_leaves_other_statuses_alone()
 {
    let db = fresh_db().await;
    let ws = repo::new_id();
    let other = repo::new_id();
    let rid = repo::new_id().to_string();
    let canceled_at = ms("2026-03-01T00:00:00.000Z");
    add(&db, build_row(ws, &rid, "pending")).await;
    add(&db, build_row(other, &rid, "pending")).await;
    add(&db, build_row(ws, &rid, "completed")).await;
    let k = key(ws, &rid);
    let updated = ok(repo::write(&db, move |tx| {
        cancel_pending_by_resource(tx, &k, canceled_at)
    })
    .await);
    assert_eq!(updated, 1);
    let r = rid.clone();
    let rows: Vec<ResourceDeletionLog> = ok(repo::read(&db, move |c| {
        find_many(c, "WHERE \"resourceId\" = ?1", &[&r])
    })
    .await);
    let mut by_ws: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "{}:{}",
                if row.workspace_id == ws {
                    "own"
                } else {
                    "other"
                },
                row.status
            )
        })
        .collect();
    by_ws.sort();
    assert_eq!(
        by_ws,
        vec!["other:pending", "own:canceled", "own:completed"]
    );
}
