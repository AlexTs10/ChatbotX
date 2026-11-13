//! Port of `packages/business/__tests__/flow-duplicate.test.ts` (P3.6-D2).
//! Adapted: the mocked flow / version reads and inserts become real rows;
//! the welcome-message sync and versions-cache calls become the stored
//! published version. Not applicable: "is audited as a create" (no audit
//! records).

mod common;

use chatbotx_business::flow;
use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};
use common::{WS, rows, scalar_i64, setup, two_workspaces};
use serde_json::{Map, Value, json};

const SOURCE: i64 = 1;

fn source_flow(tx: &Connection, name: &str) -> Result<(), DbError> {
    common::folder(tx, 50, WS, "flow")?;
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "active", "enableInInbox", "folderId")
           VALUES (?1, ?2, ?3, 1, 1, 50)"#,
        params![SOURCE, WS, name],
    )?;
    Ok(())
}

/// A graph whose step jumps to a flow (`startAnotherFlow`).
fn graph_referencing(flow_ids: &[&str]) -> Value {
    let steps: Vec<Value> = flow_ids
        .iter()
        .map(|id| json!({ "stepType": "startAnotherFlow", "flowId": id }))
        .collect();
    json!([{ "id": "11", "type": "startAnotherFlow", "data": { "details": { "steps": steps } } }])
}

fn version(tx: &Connection, id: i64, nodes: &Value, is_draft: bool) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, 11)"#,
        params![id, WS, SOURCE, nodes.to_string(), is_draft, !is_draft],
    )?;
    if !is_draft {
        tx.execute(
            r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
            params![id, SOURCE],
        )?;
    }
    Ok(())
}

async fn flow_of(db: &chatbotx_db::Db, id: i64) -> Map<String, Value> {
    rows(db, r#"SELECT * FROM "Flow" WHERE "id" = ?1"#, id)
        .await
        .pop()
        .expect("flow row")
}

async fn versions_of(db: &chatbotx_db::Db, flow_id: i64) -> Vec<Map<String, Value>> {
    rows(
        db,
        r#"SELECT * FROM "FlowVersion" WHERE "flowId" = ?1 ORDER BY "isDraft" DESC"#,
        flow_id,
    )
    .await
}

async fn flow_count(db: &chatbotx_db::Db) -> i64 {
    scalar_i64(
        db,
        r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

#[tokio::test]
async fn throws_when_the_source_flow_does_not_exist() {
    let db = two_workspaces().await;
    let err = flow::duplicate(&db, WS, SOURCE).await.expect_err("missing");
    assert_eq!(err.message(), "Flow not found");
    assert_eq!(flow_count(&db).await, 0);
}

#[tokio::test]
async fn throws_when_the_source_flow_has_no_draft_version() {
    let db = two_workspaces().await;
    setup(&db, |tx| source_flow(tx, "Welcome")).await;
    let err = flow::duplicate(&db, WS, SOURCE)
        .await
        .expect_err("no draft");
    assert_eq!(err.message(), "Draft version not found");
    assert_eq!(flow_count(&db).await, 1);
}

#[tokio::test]
async fn a_draft_only_source_gives_an_inactive_draft_only_copy() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        source_flow(tx, "Welcome")?;
        version(tx, 21, &graph_referencing(&["999"]), true)
    })
    .await;
    let id = flow::duplicate(&db, WS, SOURCE).await.expect("duplicate");
    let copy = flow_of(&db, id).await;
    assert_eq!(copy["name"], json!("Welcome _copy"));
    assert_eq!(copy["active"], json!(0));
    assert_eq!(copy["currentVersionId"], Value::Null);
    assert_eq!(copy["folderId"], json!(50));
    let versions = versions_of(&db, id).await;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["isDraft"], json!(1));
}

#[tokio::test]
async fn a_published_source_gives_a_copy_with_its_own_published_version_keeping_active() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        source_flow(tx, "Welcome")?;
        version(tx, 21, &graph_referencing(&["999"]), true)?;
        version(tx, 22, &graph_referencing(&["999"]), false)
    })
    .await;
    let id = flow::duplicate(&db, WS, SOURCE).await.expect("duplicate");
    let copy = flow_of(&db, id).await;
    assert_eq!(copy["active"], json!(1));
    assert!(copy["currentVersionId"].is_i64());
    assert_ne!(copy["currentVersionId"], json!(22));
    let versions = versions_of(&db, id).await;
    let flags: Vec<(Value, Value)> = versions
        .iter()
        .map(|v| (v["isDraft"].clone(), v["isLatest"].clone()))
        .collect();
    assert_eq!(flags, vec![(json!(1), json!(0)), (json!(0), json!(1))]);
    assert_eq!(versions[1]["id"], copy["currentVersionId"]);
}

#[tokio::test]
async fn references_to_the_source_flow_itself_point_at_the_copy_others_are_untouched() {
    let db = two_workspaces().await;
    let graph = graph_referencing(&["1", "424242"]);
    let stored = graph.clone();
    setup(&db, move |tx| {
        source_flow(tx, "Welcome")?;
        version(tx, 21, &stored, true)?;
        version(tx, 22, &stored, false)
    })
    .await;
    let id = flow::duplicate(&db, WS, SOURCE).await.expect("duplicate");
    let versions = versions_of(&db, id).await;
    assert_eq!(versions.len(), 2);
    for version in versions {
        let steps = version["nodes"].as_str().expect("nodes").to_string();
        assert!(steps.contains(&format!("\"flowId\":\"{id}\"")), "{steps}");
        assert!(steps.contains("\"flowId\":\"424242\""), "{steps}");
        assert!(!steps.contains("\"flowId\":\"1\""), "{steps}");
    }
    // The source's own graph is not changed.
    for version in versions_of(&db, SOURCE).await {
        assert!(
            version["nodes"]
                .as_str()
                .expect("nodes")
                .contains("\"flowId\":\"1\"")
        );
    }
}

#[tokio::test]
async fn a_second_copy_takes_the_next_free_name() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        source_flow(tx, "Welcome")?;
        version(tx, 21, &graph_referencing(&["999"]), true)?;
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (2, ?1, 'Welcome _copy')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let id = flow::duplicate(&db, WS, SOURCE).await.expect("duplicate");
    assert_eq!(flow_of(&db, id).await["name"], json!("Welcome _copy 2"));
}

#[tokio::test]
async fn a_long_name_is_cut_so_the_copy_suffix_still_fits_the_limit() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        source_flow(tx, &"x".repeat(255))?;
        version(tx, 21, &graph_referencing(&["999"]), true)
    })
    .await;
    let id = flow::duplicate(&db, WS, SOURCE).await.expect("duplicate");
    let name = flow_of(&db, id).await["name"]
        .as_str()
        .expect("name")
        .to_string();
    assert_eq!(name.chars().count(), 255);
    assert!(name.ends_with(" _copy"));
}
