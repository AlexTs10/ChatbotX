//! Port of `packages/database/__tests__/integration/workspace-repository.test.ts` (P3.3),
//! plus the workspace-fence case of `integration/contact-activity-repository.test.ts`
//! (it tests `workspaceRepository`; P3.2 left it to P3.3). Each rolled-back Postgres
//! transaction becomes a fresh migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::workspace::{find_empty_active_by_owner, lock_workspace_for_child_write};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{NOW, ok, setup};

const OWNER: i64 = 1101;
const OTHER_OWNER: i64 = 1102;

async fn db_with_owners() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => OWNER, "email" => "workspace-repository@example.test" })?;
        ins!(tx, "User", { "id" => OTHER_OWNER, "email" => "other-workspace-repository@example.test" })?;
        Ok(())
    })
    .await;
    db
}

fn workspace(
    tx: &chatbotx_db::rusqlite::Connection,
    id: i64,
    owner: i64,
    scheduled_deletion_at: Option<i64>,
) -> Result<usize, chatbotx_db::DbError> {
    ins!(tx, "Workspace", {
        "id" => id,
        "name" => format!("workspace-{id}"),
        "ownerId" => owner,
        "scheduledDeletionAt" => scheduled_deletion_at,
    })
}

fn webchat_inbox(
    tx: &chatbotx_db::rusqlite::Connection,
    id: i64,
    workspace_id: i64,
) -> Result<usize, chatbotx_db::DbError> {
    repo_common::insert_inbox(tx, id, workspace_id, "webchat", &format!("source-{id}"))
}

/// Port of: packages/database/__tests__/integration/workspace-repository.test.ts:49 "workspace repository against Postgres > findEmptyActiveByOwner skips the oldest workspace when it already has an Inbox"
#[tokio::test]
async fn workspace_repository_against_postgres_find_empty_active_by_owner_skips_the_oldest_workspace_when_it_already_has_an_inbox()
 {
    let db = db_with_owners().await;
    setup(&db, |tx| {
        workspace(tx, 2101, OWNER, None)?;
        workspace(tx, 2102, OWNER, None)?;
        webchat_inbox(tx, 3101, 2101)?;
        Ok(())
    })
    .await;
    let target = ok(repo::read(&db, |c| find_empty_active_by_owner(c, OWNER)).await);
    assert_eq!(target.map(|w| w.id), Some(2102));
}

/// Port of: packages/database/__tests__/integration/workspace-repository.test.ts:82 "workspace repository against Postgres > findEmptyActiveByOwner skips a workspace scheduled for deletion"
#[tokio::test]
async fn workspace_repository_against_postgres_find_empty_active_by_owner_skips_a_workspace_scheduled_for_deletion()
 {
    let db = db_with_owners().await;
    setup(&db, |tx| {
        workspace(tx, 2101, OWNER, Some(NOW))?;
        workspace(tx, 2102, OWNER, None)?;
        Ok(())
    })
    .await;
    let target = ok(repo::read(&db, |c| find_empty_active_by_owner(c, OWNER)).await);
    assert_eq!(target.map(|w| w.id), Some(2102));
}

/// Port of: packages/database/__tests__/integration/workspace-repository.test.ts:108 "workspace repository against Postgres > findEmptyActiveByOwner returns undefined when every live workspace has an Inbox"
#[tokio::test]
async fn workspace_repository_against_postgres_find_empty_active_by_owner_returns_undefined_when_every_live_workspace_has_an_inbox()
 {
    let db = db_with_owners().await;
    setup(&db, |tx| {
        workspace(tx, 2101, OWNER, None)?;
        workspace(tx, 2102, OWNER, None)?;
        webchat_inbox(tx, 3101, 2101)?;
        webchat_inbox(tx, 3102, 2102)?;
        Ok(())
    })
    .await;
    let target = ok(repo::read(&db, |c| find_empty_active_by_owner(c, OWNER)).await);
    assert_eq!(target, None);
}

/// Port of: packages/database/__tests__/integration/workspace-repository.test.ts:147 "workspace repository against Postgres > findEmptyActiveByOwner ignores empty workspaces belonging to another owner"
#[tokio::test]
async fn workspace_repository_against_postgres_find_empty_active_by_owner_ignores_empty_workspaces_belonging_to_another_owner()
 {
    let db = db_with_owners().await;
    setup(&db, |tx| {
        workspace(tx, 2101, OWNER, None)?;
        workspace(tx, 2102, OTHER_OWNER, None)?;
        webchat_inbox(tx, 3101, 2101)?;
        Ok(())
    })
    .await;
    let target = ok(repo::read(&db, |c| find_empty_active_by_owner(c, OWNER)).await);
    assert_eq!(target, None);
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:405 "contact activity repository > the workspace fence closes once the purge starts"
#[tokio::test]
async fn contact_activity_repository_the_workspace_fence_closes_once_the_purge_starts() {
    let db = fresh_db().await;
    let s = seed(&db).await;
    let ws = s.workspace_id;
    assert!(ok(repo::write(&db, move |tx| {
        lock_workspace_for_child_write(tx, ws)
    })
    .await));

    setup(&db, move |tx| {
        tx.execute(
            "UPDATE \"Workspace\" SET \"purgeStartedAt\" = ?1 WHERE \"id\" = ?2",
            chatbotx_db::rusqlite::params![NOW, ws],
        )?;
        Ok(())
    })
    .await;

    assert!(!ok(repo::write(&db, move |tx| {
        lock_workspace_for_child_write(tx, ws)
    })
    .await));
}
