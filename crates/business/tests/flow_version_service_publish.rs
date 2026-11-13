//! Port of `packages/business/__tests__/flow-version-service-publish.test.ts`
//! (P3.6-D2). Adapted: the mocked transaction / inserts / updates become real
//! rows; the welcome-message sync after the commit becomes the live
//! (`currentVersionId`) version that `after_publish_if_live` reports. Not
//! applicable: "clears stale quick reply state for the new version without
//! failing publish when cleanup rejects" (no quick-reply cleanup jobs), the
//! cache invalidation and audit parts.

mod common;

use chatbotx_business::flow_version;
use chatbotx_core::flow_config::nodes::send_message_node_default_fn;
use chatbotx_core::flow_config::steps::send_video_step_default_fn;
use chatbotx_db::DbError;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::models::FlowVersion;
use chatbotx_db::rusqlite::{Connection, params};
use common::{WS, rows, scalar_i64, setup, two_workspaces};
use serde_json::{Map, Value, json};

const FLOW: i64 = 1;
const DRAFT: i64 = 10;

fn flow_with_draft(tx: &Connection, with_draft: bool) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Flow')"#,
        params![FLOW, WS],
    )?;
    if with_draft {
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
               VALUES (?1, ?2, ?3, '[]', '[]', 1, 0, 77)"#,
            params![DRAFT, WS, FLOW],
        )?;
    }
    Ok(())
}

fn old_published(tx: &Connection, id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, '[]', '[]', 0, 1, 77)"#,
        params![id, WS, FLOW],
    )?;
    tx.execute(
        r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
        params![id, FLOW],
    )?;
    Ok(())
}

async fn version_row(db: &chatbotx_db::Db, id: i64) -> Map<String, Value> {
    rows(db, r#"SELECT * FROM "FlowVersion" WHERE "id" = ?1"#, id)
        .await
        .pop()
        .expect("version row")
}

async fn current_version(db: &chatbotx_db::Db) -> Option<i64> {
    scalar_i64(
        db,
        r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
        FLOW,
    )
    .await
}

/// A send-message node whose channel (TikTok) does not support a video step.
fn tiktok_video_node() -> Value {
    let mut node = send_message_node_default_fn(&json!({}));
    node["data"]["details"]["beforeStep"]["channel"] = json!("tiktok");
    node["data"]["details"]["steps"] = json!([send_video_step_default_fn()]);
    node
}

fn assert_authoring_error(err: &chatbotx_business::BusinessError) {
    assert_eq!(err.code(), Some("invalidRequestData"), "{err:?}");
    assert_eq!(err.status(), Some(422));
}

// describe("flowVersionService.publish")

#[tokio::test]
async fn resets_other_latest_versions_syncs_the_draft_inserts_the_published_version_repoints_current_version_id_invalidates_cache_and_audits()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow_with_draft(tx, true)?;
        old_published(tx, 5)
    })
    .await;
    let nodes = vec![send_message_node_default_fn(&json!({}))];
    let new_id = flow_version::publish(&db, WS, FLOW, nodes.clone(), vec![])
        .await
        .expect("publish");
    // 1) the other latest version is reset.
    assert_eq!(version_row(&db, 5).await["isLatest"], json!(0));
    // 2) the draft graph is synced.
    let draft = version_row(&db, DRAFT).await;
    assert_eq!(
        draft["nodes"],
        json!(Value::Array(nodes.clone()).to_string())
    );
    assert_eq!(draft["edges"], json!("[]"));
    // 3) the new published version.
    let published = version_row(&db, new_id).await;
    assert_eq!(published["workspaceId"], json!(WS));
    assert_eq!(published["flowId"], json!(FLOW));
    assert_eq!(published["isDraft"], json!(0));
    assert_eq!(published["isLatest"], json!(1));
    assert_eq!(published["nodes"], json!(Value::Array(nodes).to_string()));
    assert_eq!(published["startNodeId"], json!(77));
    // 4) currentVersionId points at it.
    assert_eq!(current_version(&db).await, Some(new_id));
}

#[tokio::test]
async fn enqueues_a_welcome_message_flow_sync_after_the_transaction_commits() {
    let db = two_workspaces().await;
    setup(&db, |tx| flow_with_draft(tx, true)).await;
    let new_id = flow_version::publish(
        &db,
        WS,
        FLOW,
        vec![send_message_node_default_fn(&json!({}))],
        vec![],
    )
    .await
    .expect("publish");
    assert!(
        flow_version::after_publish_if_live(&db, WS, FLOW, new_id)
            .await
            .expect("live")
    );
}

#[tokio::test]
async fn rejects_a_channel_incompatible_graph_before_reading_or_mutating_the_flow() {
    // No flow exists: the graph check comes first (not a "Flow not found").
    let db = two_workspaces().await;
    let err = flow_version::publish(&db, WS, FLOW, vec![tiktok_video_node()], vec![])
        .await
        .expect_err("incompatible graph");
    assert_authoring_error(&err);
}

#[tokio::test]
async fn rejects_an_incompatible_restored_graph_before_opening_a_transaction() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow_with_draft(tx, true)?;
        old_published(tx, 5)
    })
    .await;
    let version = FlowVersion {
        id: 9,
        created_at: 0,
        updated_at: 0,
        workspace_id: WS,
        flow_id: FLOW,
        nodes: Json(json!([tiktok_video_node()])),
        edges: Json(json!([])),
        is_draft: false,
        is_latest: false,
        start_node_id: 1,
    };
    let err = flow_version::restore(&db, version)
        .await
        .expect_err("incompatible");
    assert_authoring_error(&err);
    assert_eq!(current_version(&db).await, Some(5));
}

#[tokio::test]
async fn restore_enqueues_a_welcome_message_flow_sync_for_the_restored_version() {
    let db = two_workspaces().await;
    let nodes = json!([send_message_node_default_fn(&json!({}))]);
    let stored = nodes.clone();
    setup(&db, move |tx| {
        flow_with_draft(tx, true)?;
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
               VALUES (9, ?1, ?2, ?3, '[]', 0, 0, 1)"#,
            params![WS, FLOW, stored.to_string()],
        )?;
        old_published(tx, 12)
    })
    .await;
    let version = FlowVersion {
        id: 9,
        created_at: 0,
        updated_at: 0,
        workspace_id: WS,
        flow_id: FLOW,
        nodes: Json(nodes.clone()),
        edges: Json(json!([])),
        is_draft: false,
        is_latest: false,
        start_node_id: 1,
    };
    flow_version::restore(&db, version).await.expect("restore");
    assert_eq!(current_version(&db).await, Some(9));
    assert!(
        flow_version::after_publish_if_live(&db, WS, FLOW, 9)
            .await
            .expect("live")
    );
    assert_eq!(version_row(&db, 9).await["isLatest"], json!(1));
    assert_eq!(version_row(&db, 12).await["isLatest"], json!(0));
    let draft = version_row(&db, DRAFT).await;
    assert_eq!(draft["nodes"], json!(nodes.to_string()));
    assert_eq!(draft["startNodeId"], json!(1));
}

#[tokio::test]
async fn throws_not_found_exception_when_the_flow_does_not_exist() {
    let db = two_workspaces().await;
    let err = flow_version::publish(&db, WS, FLOW, vec![], vec![])
        .await
        .expect_err("missing flow");
    assert_eq!(err.message(), "Flow not found");
}

#[tokio::test]
async fn throws_not_found_exception_when_the_flow_has_no_draft_version() {
    let db = two_workspaces().await;
    setup(&db, |tx| flow_with_draft(tx, false)).await;
    let err = flow_version::publish(&db, WS, FLOW, vec![], vec![])
        .await
        .expect_err("no draft");
    assert_eq!(err.message(), "Flow not found");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "flowId" = ?1"#,
            FLOW
        )
        .await,
        Some(0)
    );
}

// describe("flowVersionService.updateDraftByFlowId")

#[tokio::test]
async fn saves_an_in_progress_graph_without_publish_validation() {
    let db = two_workspaces().await;
    setup(&db, |tx| flow_with_draft(tx, true)).await;
    flow_version::update_draft_by_flow_id(
        &db,
        WS,
        FLOW,
        vec![json!({ "id": "unsupported-node" })],
        vec![],
    )
    .await
    .expect("saved");
    assert_eq!(
        version_row(&db, DRAFT).await["nodes"],
        json!(json!([{ "id": "unsupported-node" }]).to_string())
    );
}
