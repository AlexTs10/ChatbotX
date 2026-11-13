//! Port of `packages/database/__tests__/integration/inbox-repository.test.ts` (P3.2b).
//! Real SQL on a migrated temp file (one fresh file per test instead of a rolled-back
//! transaction).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::inbox;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use common::{fresh_db, seed_workspace};
use repo_common::{ok, setup};

const OWNER: i64 = 1001;
const OTHER_OWNER: i64 = 1002;

/// `seedInbox`: a webchat inbox (`status` / `disconnectReason` as given).
fn seed_inbox(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    status: &str,
    reason: Option<&str>,
) -> Result<usize, chatbotx_db::DbError> {
    ins!(tx, "Inbox", {
        "id" => id, "workspaceId" => workspace_id, "name" => format!("inbox-{id}"),
        "channel" => "webchat".to_owned(), "sourceId" => format!("src-{id}"),
        "status" => status.to_owned(), "disconnectReason" => reason.map(str::to_owned),
        "disconnectedAt" => reason.map(|_| 1_780_000_000_000_i64),
    })
}

#[tokio::test]
async fn list_tenant_suspended_without_connection_by_owner_returns_only_tenant_suspended_inbox_rows_with_no_connection_row_scoped_to_the_owner()
 {
    let db = fresh_db().await;
    // Workspaces A (2001) and B (2003) of OWNER, 2002 of another owner.
    seed_workspace(&db, OWNER, 2001).await;
    seed_workspace(&db, OTHER_OWNER, 2002).await;
    setup(&db, |tx| {
        ins!(tx, "Workspace", { "id" => 2003_i64, "name" => "B".to_owned(), "ownerId" => OWNER })?;
        let suspended = Some("tenant_suspended");
        // Eligible: tenant_suspended, disconnected, no Connection row.
        seed_inbox(tx, 11, 2001, "disconnected", suspended)?;
        seed_inbox(tx, 12, 2003, "disconnected", suspended)?;
        // Already backfilled: has a Connection row.
        seed_inbox(tx, 13, 2001, "disconnected", suspended)?;
        ins!(tx, "Connection", {
            "id" => 51_i64, "workspaceId" => 2001_i64, "provider" => "webchat".to_owned(),
            "kind" => "channel".to_owned(), "channel" => "webchat".to_owned(), "inboxId" => 13_i64,
            "sourceId" => "src-13".to_owned(), "displayName" => "backfilled".to_owned(),
            "status" => "paused".to_owned(), "statusReason" => "tenant_suspended".to_owned(),
        })?;
        // Still connected; disconnected for another reason; another owner's.
        seed_inbox(tx, 14, 2001, "connected", None)?;
        seed_inbox(tx, 15, 2001, "disconnected", Some("manual"))?;
        seed_inbox(tx, 16, 2002, "disconnected", suspended)?;
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| {
        inbox::list_tenant_suspended_without_connection_by_owner(c, OWNER)
    })
    .await);
    let mut ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![11, 12]);
}
