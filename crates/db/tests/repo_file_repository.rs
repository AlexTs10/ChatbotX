//! Port of `packages/database/__tests__/file-repository.test.ts` (P3.2b).
//! The TS mocks `db` and checks the `eq(id)` / `eq(workspaceId)` calls; here the queries
//! run on a migrated temp file (adapted): file 81 of workspace 2001 and file 91 of another
//! workspace 2002, so both fences show in the rows.
#![allow(clippy::manual_async_fn)] // P3.2b orchestrator: style lint in locked ported test

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::file;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::types::Value;
use common::{fresh_db, seed, seed_workspace};
use repo_common::{int, ok, scalar_text, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        for (id, ws) in [(81_i64, WS), (91, OTHER)] {
            ins!(tx, "File", {
                "id" => id, "workspaceId" => ws, "userId" => 1001_i64,
                "contextType" => "generic".to_owned(), "path" => format!("public/{id}"),
                "fileName" => format!("{id}.png"), "mimeType" => "image/png".to_owned(),
            })?;
        }
        Ok(())
    })
    .await;
    db
}

fn run<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> repo::Result<T> + Send + 'static,
) -> impl std::future::Future<Output = T> + '_ {
    async move { ok(repo::read(db, f).await) }
}

async fn status(db: &Db, id: i64) -> Option<String> {
    scalar_text(
        db,
        "SELECT \"status\" FROM \"File\" WHERE \"id\" = ?1",
        vec![int(id)],
    )
    .await
}

#[tokio::test]
async fn file_repository_find_by_id_for_workspace_scopes_the_query_to_both_id_and_workspace_id() {
    let db = fixture().await;
    let row = run(&db, |c| file::find_by_id_for_workspace(c, 81, WS, None)).await;
    let row = row.expect("the workspace's own file");
    assert_eq!((row.id, row.workspace_id), (81, Some(WS)));
    assert_eq!(row.path, "public/81");
    // The same id under another workspace is not found.
    let fenced = run(&db, |c| file::find_by_id_for_workspace(c, 81, OTHER, None)).await;
    assert_eq!(fenced, None);
}

#[tokio::test]
async fn file_repository_find_by_id_for_workspace_returns_null_when_no_row_matches_e_g_a_foreign_file_id()
 {
    let db = fixture().await;
    let foreign = run(&db, |c| file::find_by_id_for_workspace(c, 91, WS, None)).await;
    assert_eq!(foreign, None);
    let missing = run(&db, |c| file::find_by_id_for_workspace(c, 999, WS, None)).await;
    assert_eq!(missing, None);
}

#[tokio::test]
async fn file_repository_update_for_workspace_scopes_the_update_to_both_id_and_workspace_id() {
    let db = fixture().await;
    let values = || vec![("status", Value::Text("uploaded".to_owned()))];
    ok(repo::write(&db, move |c| {
        file::update_for_workspace(c, 81, WS, &values())
    })
    .await);
    ok(repo::write(&db, move |c| {
        file::update_for_workspace(c, 91, WS, &values())
    })
    .await);
    assert_eq!(status(&db, 81).await.as_deref(), Some("uploaded"));
    // File 91 belongs to another workspace: untouched.
    assert_eq!(status(&db, 91).await.as_deref(), Some("pending"));
}
