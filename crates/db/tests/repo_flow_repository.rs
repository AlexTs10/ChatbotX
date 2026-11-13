//! Port of `packages/database/__tests__/flow-repository.test.ts` (P3.2b).
//! The TS mocks `db.query.flowModel` and checks the relational `where` / `with` objects;
//! here the queries run on a migrated temp file (adapted): flows of workspace 2001 and of
//! another workspace 2002, with draft, latest and older versions, so the filters show in
//! the rows.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::flow::{self, FlowListInput, FlowOption};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

/// Folder 51 (WS). Flows: 61 "Welcome" (WS, active, published 611, root folder),
/// 62 "Inactive" (WS, inactive, published 621, folder 51), 63 "Unpublished" (WS, active,
/// no published version, folder 51), 71 "Foreign" (OTHER, active, published 711).
/// Versions of 61: 611 latest, 612 draft, 613 neither.
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        ins!(tx, "Folder", {
            "id" => 51_i64, "name" => "Folder".to_owned(), "folderType" => "flow".to_owned(),
            "workspaceId" => WS, "paths" => "[]".to_owned(),
        })?;
        for (id, ws, name, active, current, folder) in [
            (61_i64, WS, "Welcome", true, Some(611_i64), None::<i64>),
            (62, WS, "Inactive", false, Some(621), Some(51)),
            (63, WS, "Unpublished", true, None, Some(51)),
            (71, OTHER, "Foreign", true, Some(711), None),
        ] {
            ins!(tx, "Flow", {
                "id" => id, "workspaceId" => ws, "name" => name.to_owned(), "active" => active,
                "currentVersionId" => current, "folderId" => folder,
            })?;
        }
        for (id, flow_id, ws, draft, latest) in [
            (611_i64, 61_i64, WS, false, true),
            (612, 61, WS, true, false),
            (613, 61, WS, false, false),
            (621, 62, WS, false, true),
            (711, 71, OTHER, false, true),
        ] {
            ins!(tx, "FlowVersion", {
                "id" => id, "workspaceId" => ws, "flowId" => flow_id, "nodes" => "[]".to_owned(),
                "edges" => "[]".to_owned(), "isDraft" => draft, "isLatest" => latest,
                "startNodeId" => 1_i64,
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

fn input(folder_id: Option<&str>) -> FlowListInput {
    FlowListInput {
        workspace_id: WS,
        folder_id: folder_id.map(str::to_owned),
        ..FlowListInput::default()
    }
}

#[tokio::test]
async fn flow_repository_list_published_options_selects_only_active_published_flows_in_the_workspace()
 {
    let db = fixture().await;
    let options = run(&db, |c| flow::list_published_options(c, WS)).await;
    assert_eq!(
        options,
        vec![FlowOption {
            id: 61,
            name: "Welcome".to_owned()
        }]
    );
}

#[tokio::test]
async fn flow_repository_list_with_versions_scopes_the_query_to_the_workspace_and_attaches_draft_latest_versions()
 {
    let db = fixture().await;
    let flows = run(&db, |c| flow::list_with_versions(c, &input(None))).await;
    let shape: Vec<(i64, Vec<i64>)> = flows
        .iter()
        .map(|f| {
            let mut v: Vec<i64> = f.flow_versions.iter().map(|v| v.id).collect();
            v.sort_unstable();
            (f.flow.id, v)
        })
        .collect();
    // Flow 71 is another workspace's; version 613 is neither draft nor latest.
    assert_eq!(
        shape,
        vec![(61, vec![611, 612]), (62, vec![621]), (63, vec![])]
    );
}

#[tokio::test]
async fn flow_repository_list_with_versions_resolves_the_root_folder_sentinel_to_is_null() {
    let db = fixture().await;
    let root = run(&db, |c| flow::list_with_versions(c, &input(Some("0")))).await;
    assert_eq!(root.iter().map(|f| f.flow.id).collect::<Vec<_>>(), vec![61]);
    let folder = run(&db, |c| flow::list_with_versions(c, &input(Some("51")))).await;
    assert_eq!(
        folder.iter().map(|f| f.flow.id).collect::<Vec<_>>(),
        vec![62, 63]
    );
}

#[tokio::test]
async fn flow_repository_count_delegates_to_db_count_with_the_same_where_shape() {
    let db = fixture().await;
    assert_eq!(run(&db, |c| flow::count(c, &input(None))).await, 3);
    let active = FlowListInput {
        active: Some(true),
        ..input(Some("51"))
    };
    let listed = run(&db, {
        let active = active.clone();
        move |c| flow::list_with_versions(c, &active)
    })
    .await;
    let counted = run(&db, move |c| flow::count(c, &active)).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(counted, 1);
}

#[tokio::test]
async fn flow_repository_find_with_versions_scopes_by_both_id_and_workspace_id() {
    let db = fixture().await;
    let found = run(&db, |c| flow::find_with_versions(c, WS, 61)).await;
    let found = found.expect("flow 61 of the workspace");
    let mut ids: Vec<i64> = found.flow_versions.iter().map(|v| v.id).collect();
    ids.sort_unstable();
    assert_eq!(found.flow.id, 61);
    assert_eq!(ids, vec![611, 612, 613]);
    let foreign = run(&db, |c| flow::find_with_versions(c, WS, 71)).await;
    assert_eq!(foreign, None);
}

#[tokio::test]
async fn flow_repository_list_ids_by_ids_returns_only_the_ids_that_exist_in_the_workspace() {
    let db = fixture().await;
    let mut ids = run(&db, |c| flow::list_ids_by_ids(c, WS, &[61, 62, 999, 71])).await;
    ids.sort_unstable();
    assert_eq!(ids, vec![61, 62]);
}
