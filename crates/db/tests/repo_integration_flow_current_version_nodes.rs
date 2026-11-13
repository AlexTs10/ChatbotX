//! Port of `packages/database/__tests__/integration/flow-current-version-nodes.test.ts`
//! (P3.2b). Real SQL on a migrated temp file, one fresh file per test.
#![allow(clippy::manual_async_fn)] // P3.2b orchestrator: style lint in locked ported test

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::flow;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};
use serde_json::{Value, json};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const PUBLISHED: i64 = 101;
const DRAFT_ONLY: i64 = 102;
const FOREIGN: i64 = 201;

fn node(id: &str, name: &str) -> Value {
    json!({ "id": id, "type": "sendMessage", "data": { "name": name } })
}

/// `createFlow`: the flow, one version per entry (`publish` = latest, else draft), and
/// `currentVersionId` = the last published one.
fn create_flow(
    tx: &Connection,
    id: i64,
    ws: i64,
    name: &str,
    versions: &[(Vec<Value>, bool)],
) -> Result<(), chatbotx_db::DbError> {
    ins!(tx, "Flow", { "id" => id, "workspaceId" => ws, "name" => name.to_owned() })?;
    let mut current = None;
    for (i, (nodes, publish)) in versions.iter().enumerate() {
        let version_id = id * 10 + i64::try_from(i).unwrap_or(0) + 1;
        let start = nodes
            .first()
            .and_then(|n| n["id"].as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(1);
        ins!(tx, "FlowVersion", {
            "id" => version_id, "workspaceId" => ws, "flowId" => id,
            "nodes" => Value::Array(nodes.clone()).to_string(), "edges" => "[]".to_owned(),
            "isDraft" => !publish, "isLatest" => *publish, "startNodeId" => start,
        })?;
        if *publish {
            current = Some(version_id);
        }
    }
    if let Some(version_id) = current {
        tx.execute(
            "UPDATE \"Flow\" SET \"currentVersionId\" = ?1 WHERE \"id\" = ?2",
            chatbotx_db::rusqlite::params![version_id, id],
        )?;
    }
    Ok(())
}

async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        create_flow(
            tx,
            PUBLISHED,
            WS,
            "Published",
            &[
                (vec![node("11", "Old start")], true),
                (vec![node("21", "Draft only")], false),
            ],
        )?;
        create_flow(
            tx,
            DRAFT_ONLY,
            WS,
            "Draft",
            &[(vec![node("31", "Never published")], false)],
        )?;
        create_flow(
            tx,
            FOREIGN,
            OTHER,
            "Foreign",
            &[(vec![node("41", "Foreign node")], true)],
        )
    })
    .await;
    db
}

fn run<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> impl std::future::Future<Output = T> + '_ {
    async move { ok(repo::read(db, f).await) }
}

fn ids(nodes: &[Value]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| n["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn returns_only_the_published_version_s_nodes_not_the_draft_s() {
    let db = fixture().await;
    let nodes = run(&db, |c| flow::find_current_version_nodes(c, WS, PUBLISHED)).await;
    assert_eq!(nodes.as_deref().map(ids), Some(vec!["11".to_owned()]));
}

#[tokio::test]
async fn a_flow_that_was_never_published_has_no_nodes() {
    let db = fixture().await;
    let nodes = run(&db, |c| flow::find_current_version_nodes(c, WS, DRAFT_ONLY)).await;
    assert_eq!(nodes, Some(vec![]));
}

#[tokio::test]
async fn another_workspace_s_flow_does_not_exist_for_this_workspace() {
    let db = fixture().await;
    let nodes = run(&db, |c| flow::find_current_version_nodes(c, WS, FOREIGN)).await;
    assert_eq!(nodes, None);
}

#[tokio::test]
async fn reads_the_nodes_of_many_flows_in_one_scoped_pass() {
    let db = fixture().await;
    let flows = run(&db, |c| {
        flow::list_current_version_nodes_by_flow_ids(c, WS, &[PUBLISHED, DRAFT_ONLY, FOREIGN, 999])
    })
    .await;
    let by_id = |id: i64| flows.iter().find(|f| f.flow_id == id);
    let published = by_id(PUBLISHED).expect("published flow");
    assert_eq!(ids(&published.nodes), vec!["11".to_owned()]);
    assert_eq!(published.flow_name, "Published");
    assert_eq!(by_id(DRAFT_ONLY).map(|f| f.nodes.clone()), Some(vec![]));
    assert!(by_id(FOREIGN).is_none());
    assert!(by_id(999).is_none());
}

#[tokio::test]
async fn asking_for_no_flows_does_no_work() {
    let db = fixture().await;
    let flows = run(&db, |c| {
        flow::list_current_version_nodes_by_flow_ids(c, WS, &[])
    })
    .await;
    assert!(flows.is_empty());
}
