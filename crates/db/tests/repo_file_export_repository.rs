//! Port of `packages/database/__tests__/file-export-repository.test.ts` (P3.2b).
//! The TS renders the Drizzle `where` with the Postgres dialect and checks its text and
//! params; here the queries run on a migrated temp file (adapted) and the rows show the
//! scope, the order, the `limit + 1` and the `(createdAt, id)` keyset. `createdAt` holds
//! unix ms, so `createdAtCursor` is the `toISOString()` text (no microseconds); a cursor
//! with microseconds is read at ms precision.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::file::{
    self, ExportFileCursor, ListExportsCreatedBeforeInput, ListExportsForWorkspaceInput,
};
use chatbotx_db::repo::{self, iso};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{int, ms, ok, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const OWNER: i64 = 1001;
const T0: &str = "2026-10-19T10:00:00.000Z";
const T_OTHER: &str = "2026-10-19T12:00:00.000Z";
const T1: &str = "2026-10-20T00:00:00.123Z";
const T2: &str = "2026-10-20T10:00:00.000Z";
const T3: &str = "2026-10-21T10:00:00.000Z";

/// Contacts exports of WS: 83 (T3, owner), 82 (T2, token: no user), 87 and 81 (T1,
/// owner), 86 (T0, owner). Not in scope: 84 (a generic upload), 85 (another export kind),
/// 91 (a contacts export of OTHER at `T_OTHER`).
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        tx.execute(
            "UPDATE \"User\" SET \"name\" = 'Owner' WHERE \"id\" = ?1",
            [OWNER],
        )?;
        for (id, ws, user, context, sub, at) in [
            (81_i64, WS, Some(OWNER), "export", "contacts", T1),
            (82, WS, None, "export", "contacts", T2),
            (83, WS, Some(OWNER), "export", "contacts", T3),
            (84, WS, Some(OWNER), "generic", "contacts", T0),
            (85, WS, Some(OWNER), "export", "pipelineItems", T0),
            (86, WS, Some(OWNER), "export", "contacts", T0),
            (87, WS, Some(OWNER), "export", "contacts", T1),
            (91, OTHER, None, "export", "contacts", T_OTHER),
        ] {
            ins!(tx, "File", {
                "id" => id, "workspaceId" => ws, "userId" => user,
                "contextType" => context.to_owned(), "subType" => sub.to_owned(),
                "path" => format!("exports/{id}.csv"), "fileName" => format!("{id}.csv"),
                "mimeType" => "text/csv".to_owned(), "createdAt" => ms(at),
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

fn for_workspace(
    user_id: Option<i64>,
    limit: i64,
    cursor: Option<ExportFileCursor>,
) -> ListExportsForWorkspaceInput {
    ListExportsForWorkspaceInput {
        workspace_id: WS,
        sub_type: "contacts".to_owned(),
        user_id,
        limit,
        cursor,
    }
}

fn created_before(limit: i64, after: Option<ExportFileCursor>) -> ListExportsCreatedBeforeInput {
    ListExportsCreatedBeforeInput {
        sub_type: "contacts".to_owned(),
        before: ms(T2),
        limit,
        after,
    }
}

#[tokio::test]
async fn file_repository_list_exports_for_workspace_lists_one_workspace_s_exports_of_one_kind_newest_first_one_row_past_the_page()
 {
    let db = fixture().await;
    let rows = run(&db, |c| {
        file::list_exports_for_workspace(c, &for_workspace(None, 2, None))
    })
    .await;
    // `limit + 1` rows, newest first, ties by id descending.
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![83, 82, 87]
    );
    // The requester's name comes from a left join: a token export has none.
    assert_eq!(rows[0].requested_by_name.as_deref(), Some("Owner"));
    assert_eq!(rows[1].user_id, None);
    assert_eq!(rows[1].requested_by_name, None);
    // The cursor column is the row's `createdAt` in UTC.
    assert_eq!(rows[0].created_at_cursor, iso(ms(T3)));
    assert_eq!(rows[0].created_at_cursor, T3);
    assert_eq!(rows[2].created_at_cursor, T1);
}

#[tokio::test]
async fn file_repository_list_exports_for_workspace_narrows_to_the_creator_and_pages_by_created_at_id()
 {
    let db = fixture().await;
    let cursor = ExportFileCursor {
        created_at: "2026-10-20T00:00:00.123456Z".to_owned(),
        id: 87,
    };
    let rows = run(&db, move |c| {
        file::list_exports_for_workspace(c, &for_workspace(Some(OWNER), 20, Some(cursor)))
    })
    .await;
    // Compared as a row: 81 has the cursor's createdAt and a smaller id; 82 (token) and 83
    // (before the cursor) are out.
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![81, 86]);
}

#[tokio::test]
async fn file_repository_list_exports_created_before_reads_only_export_rows_of_one_kind_older_than_the_cutoff_oldest_first_bounded()
 {
    let db = fixture().await;
    let rows = run(&db, |c| {
        file::list_exports_created_before(c, &created_before(500, None))
    })
    .await;
    // Every workspace; never the generic upload (84) or the other export kind (85).
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![86, 91, 81, 87]
    );
    assert_eq!(rows[1].workspace_id, Some(OTHER));
    assert_eq!(rows[1].path, "exports/91.csv");
    assert_eq!(rows[1].created_at_cursor, T_OTHER);
    let bounded = run(&db, |c| {
        file::list_exports_created_before(c, &created_before(2, None))
    })
    .await;
    assert_eq!(
        bounded.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![86, 91]
    );
}

#[tokio::test]
async fn file_repository_list_exports_created_before_resumes_past_the_previous_batch_oldest_first()
{
    let db = fixture().await;
    let after = ExportFileCursor {
        created_at: "2026-10-19T12:00:00.000000Z".to_owned(),
        id: 91,
    };
    let rows = run(&db, move |c| {
        file::list_exports_created_before(c, &created_before(500, Some(after)))
    })
    .await;
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![81, 87]);
}

#[tokio::test]
async fn file_repository_delete_by_id_deletes_exactly_the_given_row() {
    let db = fixture().await;
    ok(repo::write(&db, |c| file::delete_by_id(c, 81)).await);
    let count = |id: i64| {
        let db = &db;
        async move {
            scalar_i64(
                db,
                "SELECT count(*) FROM \"File\" WHERE \"id\" = ?1",
                vec![int(id)],
            )
            .await
        }
    };
    assert_eq!(count(81).await, Some(0));
    assert_eq!(count(87).await, Some(1));
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM \"File\"", vec![]).await,
        Some(7)
    );
}
