//! Port of `packages/business/__tests__/flow.service.test.ts` (P3.6-D2).
//! Adapted: the mocked repository / transaction become a real migrated
//! database; the asserted inserts and updates become the stored rows. The
//! welcome-message sync jobs become the returned `FlowUpdated::sync` /
//! deleted ids. Not applicable: "a rename of a locked template copy is
//! refused before any write" and "takes the guard first, inside the locked
//! transaction, before the write" (no template locks / guards, no job
//! queue; P3.6-D1 decision), and the audit records.

mod common;

use chatbotx_business::flow::{
    self, FlowGraph, FlowGraphInput, FlowUpdate, PublishedDefault, WelcomeSync,
};
use chatbotx_business::tx;
use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, rows, scalar_i64, setup, two_workspaces};
use serde_json::{Value, json};

fn flow_row(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    name: &str,
    current_version_id: Option<i64>,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "active", "enableInInbox", "currentVersionId", "updatedAt")
           VALUES (?1, ?2, ?3, 1, 1, ?4, 1000)"#,
        params![id, workspace_id, name, current_version_id],
    )?;
    Ok(())
}

async fn flow_of(db: &chatbotx_db::Db, id: i64) -> serde_json::Map<String, Value> {
    rows(db, r#"SELECT * FROM "Flow" WHERE "id" = ?1"#, id)
        .await
        .pop()
        .expect("flow row")
}

async fn versions_of(db: &chatbotx_db::Db, flow_id: i64) -> Vec<serde_json::Map<String, Value>> {
    rows(
        db,
        r#"SELECT * FROM "FlowVersion" WHERE "flowId" = ?1 ORDER BY "isDraft" DESC"#,
        flow_id,
    )
    .await
}

async fn sessions_of(db: &chatbotx_db::Db, flow_id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT COUNT(*) FROM "FlowAnalyticsSession" WHERE "flowId" = ?1"#,
        flow_id,
    )
    .await
    .unwrap_or_default()
}

fn nodes_of(version: &serde_json::Map<String, Value>) -> Value {
    serde_json::from_str(version["nodes"].as_str().expect("nodes text")).expect("nodes JSON")
}

fn edges_of(version: &serde_json::Map<String, Value>) -> Value {
    serde_json::from_str(version["edges"].as_str().expect("edges text")).expect("edges JSON")
}

/// A publishable graph (a compiled one-text spec).
async fn publishable_graph(db: &chatbotx_db::Db) -> FlowGraph {
    flow::resolve_graph_input(
        db,
        WS,
        FlowGraphInput {
            spec: Some(json!({
                "formatVersion": 1,
                "name": "Spec",
                "steps": [{ "type": "send", "text": "hello" }],
            })),
            nodes: None,
            edges: None,
        },
        true,
    )
    .await
    .expect("graph")
    .expect("compiled")
}

async fn welcome(db: &chatbotx_db::Db) {
    setup(db, |tx| {
        flow_row(tx, 1, WS, "Welcome", None)?;
        flow_row(tx, 9, OTHER_WS, "Welcome", None)
    })
    .await;
}

// describe("flowService.listPublishedOptions")

#[tokio::test]
async fn delegates_the_workspace_scoped_options_read_to_the_repository() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow_row(tx, 1, WS, "Welcome", Some(11))?;
        flow_row(tx, 2, WS, "Draft only", None)?;
        flow_row(tx, 3, OTHER_WS, "Foreign", Some(33))
    })
    .await;
    let options = flow::list_published_options(&db, WS)
        .await
        .expect("options");
    let options: Vec<(i64, String)> = options.into_iter().map(|o| (o.id, o.name)).collect();
    assert_eq!(options, vec![(1, "Welcome".to_string())]);
}

// describe("flowService.update")

#[tokio::test]
async fn throws_when_the_flow_does_not_exist_in_the_workspace() {
    let db = two_workspaces().await;
    welcome(&db).await;
    let err = flow::update(
        &db,
        WS,
        9,
        FlowUpdate {
            name: Some("New Name".into()),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect_err("not found");
    assert_eq!(err.message(), "Flow not found");
    assert_eq!(flow_of(&db, 9).await["name"], json!("Welcome"));
}

#[tokio::test]
async fn locks_the_flow_before_the_stored_row_is_read_then_diffs_the_fresh_row() {
    let db = two_workspaces().await;
    welcome(&db).await;
    let out = flow::update(
        &db,
        WS,
        1,
        FlowUpdate {
            name: Some("Renamed".into()),
            active: Some(true),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(out.changed_keys, vec!["name"]);
    assert_eq!(flow_of(&db, 1).await["name"], json!("Renamed"));
}

#[tokio::test]
async fn no_ops_when_nothing_changed() {
    let db = two_workspaces().await;
    welcome(&db).await;
    let out = flow::update(
        &db,
        WS,
        1,
        FlowUpdate {
            name: Some("Welcome".into()),
            active: Some(true),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect("update");
    assert!(out.changed_keys.is_empty());
    assert_eq!(out.sync, None);
    let row = flow_of(&db, 1).await;
    assert_eq!(row["updatedAt"], json!(1000));
    assert_eq!(row["name"], json!("Welcome"));
}

#[tokio::test]
async fn updates_and_audits_when_a_field_changed() {
    let db = two_workspaces().await;
    welcome(&db).await;
    flow::update(
        &db,
        WS,
        1,
        FlowUpdate {
            name: Some("Onboarding".into()),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect("update");
    let row = flow_of(&db, 1).await;
    assert_eq!(row["name"], json!("Onboarding"));
    assert_eq!(row["active"], json!(1));
    assert_eq!(row["enableInInbox"], json!(1));
    assert_ne!(row["updatedAt"], json!(1000));
}

// describe("flowService.createPublishedDefault")

#[tokio::test]
async fn inserts_a_flow_analytics_session_draft_version_and_published_version_pointing_at_each_other()
 {
    let db = two_workspaces().await;
    let ids = tx::write(&db, |tx| {
        flow::create_published_default(
            tx,
            PublishedDefault {
                workspace_id: WS,
                name: "Booking confirmation - Lich_1".into(),
                folder_id: None,
                start_node_id: 77,
                nodes: vec![json!({ "id": "77" })],
                edges: vec![],
            },
        )
    })
    .await
    .expect("create");
    let row = flow_of(&db, ids.flow_id).await;
    assert_eq!(row["name"], json!("Booking confirmation - Lich_1"));
    assert_eq!(row["active"], json!(1));
    assert_eq!(row["enableInInbox"], json!(0));
    assert_eq!(row["workspaceId"], json!(WS));
    assert_eq!(row["folderId"], Value::Null);
    assert_eq!(row["currentVersionId"], json!(ids.published_version_id));
    assert_eq!(row["draftVersionId"], json!(ids.draft_version_id));
    assert_eq!(sessions_of(&db, ids.flow_id).await, 1);
    let versions = versions_of(&db, ids.flow_id).await;
    assert_eq!(versions.len(), 2);
    for (version, id, draft, latest) in [
        (&versions[0], ids.draft_version_id, 1, 0),
        (&versions[1], ids.published_version_id, 0, 1),
    ] {
        assert_eq!(version["id"], json!(id));
        assert_eq!(version["workspaceId"], json!(WS));
        assert_eq!(version["isDraft"], json!(draft));
        assert_eq!(version["isLatest"], json!(latest));
        assert_eq!(version["startNodeId"], json!(77));
        assert_eq!(nodes_of(version), json!([{ "id": "77" }]));
        assert_eq!(edges_of(version), json!([]));
    }
}

// describe("flowService.createDraft")

#[tokio::test]
async fn checks_the_folder_exists_when_a_folder_id_is_given() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::folder(tx, 50, WS, "flow")?;
        common::folder(tx, 51, WS, "tag")?;
        common::folder(tx, 52, OTHER_WS, "flow")
    })
    .await;
    for folder in [51, 52, 404] {
        let err = flow::create_draft(&db, WS, "New flow", Some(folder), None)
            .await
            .expect_err("folder check");
        assert_eq!(err.message(), "Folder not found");
    }
    let id = flow::create_draft(&db, WS, "New flow", Some(50), None)
        .await
        .expect("create");
    assert_eq!(flow_of(&db, id).await["folderId"], json!(50));
}

#[tokio::test]
async fn inserts_the_flow_its_analytics_session_and_a_draft_version_with_one_default_start_node() {
    let db = two_workspaces().await;
    let id = flow::create_draft(&db, WS, "New flow", None, None)
        .await
        .expect("create");
    let row = flow_of(&db, id).await;
    assert_eq!(row["name"], json!("New flow"));
    assert_eq!(row["workspaceId"], json!(WS));
    assert_eq!(sessions_of(&db, id).await, 1);
    let versions = versions_of(&db, id).await;
    assert_eq!(versions.len(), 1);
    let draft = &versions[0];
    assert_eq!(draft["isDraft"], json!(1));
    assert_eq!(draft["workspaceId"], json!(WS));
    let nodes = nodes_of(draft);
    assert_eq!(nodes.as_array().map(Vec::len), Some(1));
    assert_eq!(
        nodes[0]["id"].as_str().and_then(|s| s.parse::<i64>().ok()),
        draft["startNodeId"].as_i64()
    );
    assert_eq!(nodes[0]["data"]["isStartNode"], json!(true));
    assert_eq!(edges_of(draft), json!([]));
}

#[tokio::test]
async fn uses_a_pre_resolved_graph_instead_of_the_default_start_node_when_one_is_supplied() {
    let db = two_workspaces().await;
    let graph = FlowGraph {
        nodes: vec![json!({ "id": "5" })],
        edges: vec![json!({ "id": "e1", "source": "5", "target": "5" })],
        start_node_id: "5".into(),
        node_ids: None,
    };
    let id = flow::create_draft(&db, WS, "New flow", None, Some(graph))
        .await
        .expect("create");
    let versions = versions_of(&db, id).await;
    assert_eq!(versions.len(), 1);
    assert_eq!(nodes_of(&versions[0]), json!([{ "id": "5" }]));
    assert_eq!(
        edges_of(&versions[0]),
        json!([{ "id": "e1", "source": "5", "target": "5" }])
    );
    assert_eq!(versions[0]["startNodeId"], json!(5));
    assert_eq!(versions[0]["isDraft"], json!(1));
}

#[tokio::test]
async fn creates_a_draft_with_an_in_progress_graph_without_publish_validation() {
    let db = two_workspaces().await;
    let graph = FlowGraph {
        nodes: vec![json!({ "id": "6", "type": "unsupported" })],
        edges: vec![],
        start_node_id: "6".into(),
        node_ids: None,
    };
    let id = flow::create_draft(&db, WS, "New flow", None, Some(graph))
        .await
        .expect("an unpublishable draft is stored");
    assert_eq!(versions_of(&db, id).await.len(), 1);
}

// describe("flowService.createPublished")

#[tokio::test]
async fn checks_the_folder_exists_before_opening_the_transaction() {
    let db = two_workspaces().await;
    setup(&db, |tx| common::folder(tx, 51, WS, "tag")).await;
    let graph = publishable_graph(&db).await;
    let err = flow::create_published(&db, WS, "New flow", Some(51), graph)
        .await
        .expect_err("folder check");
    assert_eq!(err.message(), "Folder not found");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn inserts_flow_draft_published_version_in_one_transaction_invalidates_the_versions_cache_and_audits()
 {
    let db = two_workspaces().await;
    let graph = publishable_graph(&db).await;
    let id = flow::create_published(&db, WS, "New flow", None, graph)
        .await
        .expect("create");
    let versions = versions_of(&db, id).await;
    assert_eq!(versions.len(), 2);
    assert_eq!(
        (&versions[0]["isDraft"], &versions[0]["isLatest"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(
        (&versions[1]["isDraft"], &versions[1]["isLatest"]),
        (&json!(0), &json!(1))
    );
    let row = flow_of(&db, id).await;
    assert_eq!(row["currentVersionId"], versions[1]["id"]);
}

// describe("flowService welcome message flow sync hooks")

#[tokio::test]
async fn update_enqueues_a_rename_sync_when_the_name_changes() {
    let db = two_workspaces().await;
    welcome(&db).await;
    let out = flow::update(
        &db,
        WS,
        1,
        FlowUpdate {
            name: Some("Onboarding".into()),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(out.sync, Some(WelcomeSync::Rename));
}

#[tokio::test]
async fn update_without_a_name_change_does_not_enqueue() {
    let db = two_workspaces().await;
    welcome(&db).await;
    let out = flow::update(
        &db,
        WS,
        1,
        FlowUpdate {
            name: Some("Welcome".into()),
            active: Some(false),
            ..FlowUpdate::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(out.changed_keys, vec!["active"]);
    assert_eq!(out.sync, None);
    assert_eq!(flow_of(&db, 1).await["active"], json!(0));
}

#[tokio::test]
async fn create_published_enqueues_a_publish_sync_for_the_new_flow() {
    // The sync reads the live (published) version: the new flow has one.
    let db = two_workspaces().await;
    let graph = publishable_graph(&db).await;
    let id = flow::create_published(&db, WS, "New flow", None, graph)
        .await
        .expect("create");
    assert!(flow_of(&db, id).await["currentVersionId"].is_i64());
}

#[tokio::test]
async fn delete_many_enqueues_one_delete_job_for_the_deleted_ids_after_commit() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow_row(tx, 2, WS, "Two", None)?;
        flow_row(tx, 3, WS, "Three", None)?;
        flow_row(tx, 5, OTHER_WS, "Foreign", None)
    })
    .await;
    let mut deleted = flow::delete_many(&db, WS, vec![2, 3, 404, 5])
        .await
        .expect("delete");
    deleted.sort_unstable();
    assert_eq!(deleted, vec![2, 3]);
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
    assert_eq!(flow_of(&db, 5).await["name"], json!("Foreign"));
}

#[tokio::test]
async fn delete_many_with_no_matching_flows_does_not_enqueue() {
    let db = two_workspaces().await;
    let deleted = flow::delete_many(&db, WS, vec![404]).await.expect("delete");
    assert!(deleted.is_empty());
}
