//! Port of `packages/database/__tests__/contact-repository-backdate-created-at.test.ts`
//! (P3.2). The rendered UPDATE becomes its effect on real rows (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact::backdate_created_at;
use chatbotx_db::repo::{self};
use repo_common::{int, ms, ok, scalar_i64};

/// Port of: packages/database/__tests__/contact-repository-backdate-created-at.test.ts:14 "contactRepository.backdateCreatedAt > is a no-op with no rows"
#[tokio::test]
async fn contact_repository_backdate_created_at_is_a_no_op_with_no_rows() {
    let db = common::fresh_db().await;
    assert!(ok(repo::write(&db, |tx| backdate_created_at(tx, &[])).await).is_empty());
}

/// Port of: packages/database/__tests__/contact-repository-backdate-created-at.test.ts:29 "contactRepository.backdateCreatedAt > moves createdAt back with LEAST in one workspace-scoped statement and returns the changed ids"
#[tokio::test]
async fn contact_repository_backdate_created_at_moves_created_at_back_with_least_in_one_workspace_scoped_statement_and_returns_the_changed_ids()
 {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, 2002).await;
    let now = ms("2025-06-01T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for (id, ws) in [(1_i64, 2001_i64), (2, 2001), (3, 2002)] {
            ins!(tx, "Contact", { "id" => id, "workspaceId" => ws, "createdAt" => now })?;
        }
        tx.execute(
            r#"UPDATE "Contact" SET "createdAt" = ?1 WHERE "id" = 2"#,
            [ms("2025-01-15T00:00:00.000Z")],
        )?;
        Ok(())
    })
    .await;
    let first = ms("2025-01-01T00:00:00.000Z");
    let second = ms("2025-02-01T00:00:00.000Z");
    let changed = ok(repo::write(&db, move |tx| {
        backdate_created_at(tx, &[(1, 2001, first), (2, 2001, second), (3, 2001, first)])
    })
    .await);
    assert_eq!(changed, vec![1]);
    let at = |id| {
        scalar_i64(
            &db,
            r#"SELECT "createdAt" FROM "Contact" WHERE "id" = ?1"#,
            vec![int(id)],
        )
    };
    assert_eq!(at(1).await, Some(first));
    // Never forward; another workspace's contact is untouched.
    assert_eq!(at(2).await, Some(ms("2025-01-15T00:00:00.000Z")));
    assert_eq!(at(3).await, Some(now));
}
