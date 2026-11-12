//! Port of `packages/database/__tests__/integration/connection-repository.test.ts` (P3.2).
//! Runs against a migrated SQLite temp file (fresh file per test). The constraint checks
//! insert rows directly, as the TS does through Drizzle.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::connection::{self as c, ConnectionListInput, ConnectionPatch};
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, DbError};
use repo_common::{int, ok};

const A: i64 = 2001;
const B: i64 = 2002;

async fn db() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, B).await;
    db
}

/// `seedIntegrationConnection` (googleSheets integration + Connection `id`).
fn conn_row(
    tx: &Connection,
    id: i64,
    ws: i64,
    source: &str,
    display: &str,
    status: &str,
    reason: Option<&str>,
    disconnected: Option<i64>,
) -> Result<i64, DbError> {
    repo_common::insert_integration(tx, id + 100_000, ws)?;
    ins!(tx, "Connection", {
        "id" => id, "workspaceId" => ws, "provider" => "googleSheets", "kind" => "integration",
        "sourceId" => source.to_owned(), "displayName" => display.to_owned(), "status" => status.to_owned(),
        "statusReason" => reason.map(str::to_owned), "disconnectedAt" => disconnected, "integrationId" => id + 100_000,
    })?;
    Ok(id)
}

fn connected(tx: &Connection, id: i64, ws: i64, source: &str) -> Result<i64, DbError> {
    conn_row(tx, id, ws, source, source, "connected", None, None)
}

async fn fails_with(
    db: &Db,
    code: &str,
    f: impl FnOnce(&Connection) -> Result<usize, DbError> + Send + 'static,
) {
    let err = db.write(move |tx| f(tx)).await.expect_err("must fail");
    assert_eq!(err.sqlstate(), Some(code), "{err}");
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:111 "connectionRepository against Postgres > keeps id and provider-source lookups within the requested workspace"
#[tokio::test]
async fn connection_repository_against_postgres_keeps_id_and_provider_source_lookups_within_the_requested_workspace()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| connected(tx, 1, A, "account-1")).await;
    assert_eq!(
        ok(repo::read(&db, |x| c::find_by_id_for_workspace(x, 1, A)).await).map(|r| r.id),
        Some(1)
    );
    assert!(ok(repo::read(&db, |x| c::find_by_id_for_workspace(x, 1, B)).await).is_none());
    assert!(
        ok(repo::read(&db, |x| c::find_by_provider_source_id(
            x,
            B,
            "googleSheets",
            "account-1"
        ))
        .await)
        .is_none()
    );
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:144 "connectionRepository against Postgres > does not update or lock a connection outside its workspace"
#[tokio::test]
async fn connection_repository_against_postgres_does_not_update_or_lock_a_connection_outside_its_workspace()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| connected(tx, 1, A, "account-1")).await;
    let patch = ConnectionPatch {
        display_name: Some("cross-workspace update".into()),
        ..Default::default()
    };
    assert!(ok(repo::write(&db, move |tx| c::update(tx, 1, B, &patch)).await).is_none());
    assert_eq!(
        ok(repo::read(&db, |x| c::find_by_id(x, 1)).await)
            .map(|r| r.display_name)
            .as_deref(),
        Some("account-1")
    );
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:168 "connectionRepository against Postgres > lists and counts only the requested workspace in public order"
#[tokio::test]
async fn connection_repository_against_postgres_lists_and_counts_only_the_requested_workspace_in_public_order()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        conn_row(tx, 1, A, "account-z", "Zulu", "connected", None, None)?;
        conn_row(tx, 2, A, "account-a", "Alpha", "connected", None, None)?;
        conn_row(
            tx,
            3,
            B,
            "account-a",
            "Other workspace",
            "connected",
            None,
            None,
        )?;
        Ok(())
    })
    .await;
    let input = ConnectionListInput {
        workspace_id: A,
        page: Some(1),
        per_page: Some(1),
        ..Default::default()
    };
    let i2 = input.clone();
    let rows = ok(repo::read(&db, move |x| c::list(x, &input)).await);
    assert_eq!(
        rows.iter()
            .map(|r| r.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha"]
    );
    assert_eq!(
        ok(repo::read(&db, move |x| c::count(
            x,
            &ConnectionListInput {
                workspace_id: A,
                ..i2
            }
        ))
        .await),
        2
    );
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:199 "connectionRepository against Postgres > prefers an active match before the newest inactive cross-workspace connection"
#[tokio::test]
async fn connection_repository_against_postgres_prefers_an_active_match_before_the_newest_inactive_cross_workspace_connection()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        connected(tx, 1, A, "account-1")?;
        conn_row(
            tx,
            2,
            B,
            "account-1",
            "account-1",
            "disconnected",
            Some("manual"),
            Some(5),
        )?;
        Ok(())
    })
    .await;
    let found = ok(repo::read(&db, |x| {
        c::find_by_provider_and_source_id_any_workspace(x, "googleSheets", "account-1")
    })
    .await);
    assert_eq!(found.map(|r| r.id), Some(1));
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:224 "connectionRepository against Postgres > uses the newest row when active cross-workspace matches have equal priority"
#[tokio::test]
async fn connection_repository_against_postgres_uses_the_newest_row_when_active_cross_workspace_matches_have_equal_priority()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        connected(tx, 1, A, "account-1")?;
        connected(tx, 2, B, "account-1")?;
        Ok(())
    })
    .await;
    let found = ok(repo::read(&db, |x| {
        c::find_by_provider_and_source_id_any_workspace(x, "googleSheets", "account-1")
    })
    .await);
    assert_eq!(found.map(|r| r.id), Some(2));
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:245 "connectionRepository against Postgres > enforces the workspace-provider-source unique identity"
#[tokio::test]
async fn connection_repository_against_postgres_enforces_the_workspace_provider_source_unique_identity()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| connected(tx, 1, A, "account-1")).await;
    fails_with(&db, "23505", |tx| {
        Ok(connected(tx, 2, A, "account-1")? as usize)
    })
    .await;
}

fn channel_conn(tx: &Connection, id: i64, inbox: i64, source: &str) -> Result<usize, DbError> {
    ins!(tx, "Connection", { "id" => id, "workspaceId" => A, "provider" => "telegram", "kind" => "channel", "channel" => "telegram", "inboxId" => inbox, "sourceId" => source.to_owned(), "displayName" => source.to_owned() })
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:263 "connectionRepository against Postgres > enforces unique ownership of an inbox by a connection"
#[tokio::test]
async fn connection_repository_against_postgres_enforces_unique_ownership_of_an_inbox_by_a_connection()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3100, A, "telegram", "connection-repository-test")?;
        channel_conn(tx, 1, 3100, "bot-1")
    })
    .await;
    fails_with(&db, "23505", |tx| channel_conn(tx, 2, 3100, "bot-2")).await;
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:300 "connectionRepository against Postgres > deletes a pending reconnect session when its inbox connection is deleted"
#[tokio::test]
async fn connection_repository_against_postgres_deletes_a_pending_reconnect_session_when_its_inbox_connection_is_deleted()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3100, A, "telegram", "reconnect-cascade-test")?;
        channel_conn(tx, 1, 3100, "bot-reconnect")?;
        ins!(tx, "ConnectSession", { "id" => 9_i64, "workspaceId" => A, "provider" => "telegram", "purpose" => "reconnect", "targetConnectionId" => 1_i64, "actorUserId" => 1001_i64, "stateNonceHash" => "reconnect-1", "expiresAt" => chatbotx_db::repo::now_ms() + 600_000 })
    })
    .await;
    repo_common::handover::exec(
        &db,
        r#"DELETE FROM "Inbox" WHERE "id" = ?1"#,
        vec![int(3100)],
    )
    .await;
    assert_eq!(
        repo_common::scalar_i64(
            &db,
            r#"SELECT count(*) FROM "ConnectSession" WHERE "id" = 9"#,
            vec![]
        )
        .await,
        Some(0)
    );
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:347 "connectionRepository against Postgres > enforces unique ownership of an integration by a connection"
#[tokio::test]
async fn connection_repository_against_postgres_enforces_unique_ownership_of_an_integration_by_a_connection()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_integration(tx, 50, A)?;
        ins!(tx, "Connection", { "id" => 1_i64, "workspaceId" => A, "provider" => "googleSheets", "kind" => "integration", "sourceId" => "account-1", "displayName" => "a", "integrationId" => 50_i64 })
    })
    .await;
    fails_with(&db, "23505", |tx| ins!(tx, "Connection", { "id" => 2_i64, "workspaceId" => A, "provider" => "googleSheets", "kind" => "integration", "sourceId" => "account-2", "displayName" => "b", "integrationId" => 50_i64 })).await;
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:371 "connectionRepository against Postgres > rejects connection rows that violate the kind or connected-state invariants"
#[tokio::test]
async fn connection_repository_against_postgres_rejects_connection_rows_that_violate_the_kind_or_connected_state_invariants()
 {
    let db = db().await;
    fails_with(&db, "23514", |tx| ins!(tx, "Connection", { "id" => 1_i64, "workspaceId" => A, "provider" => "googleSheets", "kind" => "integration", "sourceId" => "missing-integration", "displayName" => "x" })).await;
    repo_common::setup(&db, |tx| repo_common::insert_integration(tx, 50, A)).await;
    type Row = (
        &'static str,
        &'static str,
        Option<&'static str>,
        Option<i64>,
    );
    let cases: [Row; 5] = [
        ("sub_connection", "connected", None, None),
        ("integration", "connected", Some("manual"), None),
        ("integration", "needs_reauth", None, None),
        ("integration", "disconnected", Some("manual"), None),
        ("integration", "needs_reauth", Some("manual"), Some(5)),
    ];
    for (i, (kind, status, reason, disconnected)) in cases.into_iter().enumerate() {
        let id = 10 + i as i64;
        fails_with(&db, "23514", move |tx| {
            ins!(tx, "Connection", { "id" => id, "workspaceId" => A, "provider" => "googleSheets", "kind" => kind, "integrationId" => 50_i64, "sourceId" => format!("case-{id}"), "displayName" => "x", "status" => status, "statusReason" => reason, "disconnectedAt" => disconnected })
        })
        .await;
    }
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:489 "connectionRepository against Postgres > cascades deleting an integration to its Connection row"
#[tokio::test]
async fn connection_repository_against_postgres_cascades_deleting_an_integration_to_its_connection_row()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| connected(tx, 1, A, "integration-cascade")).await;
    repo_common::handover::exec(
        &db,
        r#"DELETE FROM "Integration" WHERE "id" = ?1"#,
        vec![int(100_001)],
    )
    .await;
    assert!(ok(repo::read(&db, |x| c::find_by_id(x, 1)).await).is_none());
}

/// Port of: packages/database/__tests__/integration/connection-repository.test.ts:511 "connectionRepository against Postgres > listPausedByOwner returns paused connections across the owner's workspaces only"
#[tokio::test]
async fn connection_repository_against_postgres_list_paused_by_owner_returns_paused_connections_across_the_owner_s_workspaces_only()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "Workspace", { "id" => 2003_i64, "name" => "paused-owner-b", "ownerId" => 1001_i64 })?;
        conn_row(tx, 1, A, "paused-a", "a", "paused", Some("trial_expired"), None)?;
        conn_row(tx, 2, 2003, "paused-b", "b", "paused", Some("trial_expired"), None)?;
        connected(tx, 3, A, "connected-a")?;
        conn_row(tx, 4, B, "paused-other", "o", "paused", Some("trial_expired"), None)?;
        Ok(())
    })
    .await;
    let mut ids: Vec<i64> = ok(repo::read(&db, |x| c::list_paused_by_owner(x, 1001)).await)
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
}
