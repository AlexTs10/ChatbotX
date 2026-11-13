//! Port of `packages/database/__tests__/inbox-team-member-repository.test.ts` (P3.2b).
//! The TS mocks the Drizzle chain and checks the rendered join / where; here the queries
//! run on a migrated temp file (adapted): teams of workspace 2001 and of another workspace
//! 2002 with the same members, so the workspace fence is visible in the rows.
#![allow(clippy::manual_async_fn)] // P3.2b orchestrator: style lint in locked ported test

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::inbox_team_member as itm;
use chatbotx_db::repo::{self};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const USER1: i64 = 1001;
const USER2: i64 = 1002;

/// Teams: 31 "Support" and 32 "Sales" in WS, 41 "Billing" in OTHER. Members: 31 = user 1,
/// 32 = users 1 and 2, 41 = users 1 and 2.
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, USER2, OTHER).await;
    setup(&db, |tx| {
        for (id, ws, name) in [
            (31, WS, "Support"),
            (32, WS, "Sales"),
            (41, OTHER, "Billing"),
        ] {
            ins!(tx, "InboxTeam", { "id" => id, "workspaceId" => ws, "name" => name.to_owned() })?;
        }
        for (id, team, user) in [
            (1, 31, USER1),
            (2, 32, USER1),
            (3, 32, USER2),
            (4, 41, USER1),
            (5, 41, USER2),
        ] {
            ins!(tx, "InboxTeamMember", { "id" => id, "inboxTeamId" => team, "userId" => user })?;
        }
        Ok(())
    })
    .await;
    db
}

fn sorted(mut v: Vec<i64>) -> Vec<i64> {
    v.sort_unstable();
    v
}

fn run<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> repo::Result<T> + Send + 'static,
) -> impl std::future::Future<Output = T> + '_ {
    async move { ok(repo::read(db, f).await) }
}

#[tokio::test]
async fn inbox_team_member_repository_workspace_scoping_returns_only_member_ids_selected_through_the_workspace_scoped_team_join()
 {
    let db = fixture().await;
    let ids = run(&db, |c| itm::list_user_ids_by_team_id(c, WS, 32)).await;
    assert_eq!(sorted(ids), vec![USER1, USER2]);
    // A team of another workspace is fenced out by the InboxTeam join.
    let fenced = run(&db, |c| itm::list_user_ids_by_team_id(c, WS, 41)).await;
    assert!(fenced.is_empty());
}

#[tokio::test]
async fn inbox_team_member_repository_workspace_scoping_returns_member_ids_for_all_workspace_scoped_team_ids_in_one_query()
 {
    let db = fixture().await;
    let ids = run(&db, |c| {
        itm::list_user_ids_by_team_ids(c, WS, &[31, 32, 41])
    })
    .await;
    assert_eq!(sorted(ids), vec![USER1, USER1, USER2]);
    let none = run(&db, |c| itm::list_user_ids_by_team_ids(c, WS, &[])).await;
    assert!(none.is_empty());
}

#[tokio::test]
async fn inbox_team_member_repository_workspace_scoping_returns_only_team_ids_selected_through_the_workspace_scoped_member_join()
 {
    let db = fixture().await;
    let ids = run(&db, |c| itm::list_team_ids_by_user_id(c, WS, USER1)).await;
    assert_eq!(sorted(ids), vec![31, 32]);
    let other = run(&db, |c| itm::list_team_ids_by_user_id(c, OTHER, USER1)).await;
    assert_eq!(other, vec![41]);
}

#[tokio::test]
async fn inbox_team_member_repository_list_team_names_by_user_id_returns_the_team_names_of_the_user_scoped_to_the_workspace_through_the_team_join()
 {
    let db = fixture().await;
    let names = run(&db, |c| itm::list_team_names_by_user_id(c, WS, USER1)).await;
    assert_eq!(names, vec!["Sales".to_owned(), "Support".to_owned()]);
}

#[tokio::test]
async fn inbox_team_member_repository_list_team_names_by_user_id_returns_an_empty_list_for_a_user_in_no_team()
 {
    let db = fixture().await;
    let names = run(&db, |c| itm::list_team_names_by_user_id(c, WS, 999)).await;
    assert!(names.is_empty());
}
