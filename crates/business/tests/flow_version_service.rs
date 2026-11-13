//! Port of `packages/business/__tests__/flow-version.service.test.ts`
//! (P3.6-D2). Adapted: the mocked `findFirst` / `update` / `$count` become
//! real `Flow` / `FlowVersion` rows; the asserted queries become the
//! returned and stored rows. Not applicable: the two
//! `getMessengerAdsStartNode` cases (the messenger-ads start node read and
//! its cache are not ported).

mod common;

use chatbotx_business::flow_version;
use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, rows, setup, two_workspaces};
use serde_json::{Value, json};

const FLOW: i64 = 1;

fn flow(tx: &Connection, id: i64, workspace_id: i64, current: Option<i64>) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "currentVersionId") VALUES (?1, ?2, 'Flow', ?3)"#,
        params![id, workspace_id, current],
    )?;
    Ok(())
}

fn version(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    flow_id: i64,
    (is_draft, is_latest): (bool, bool),
    start: i64,
    nodes: &Value,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            id,
            workspace_id,
            flow_id,
            nodes.to_string(),
            json!([{ "id": format!("edge-{id}") }]).to_string(),
            is_draft,
            is_latest,
            start
        ],
    )?;
    Ok(())
}

/// Flow 1 of `WS` with a draft (10) and a published latest version (20);
/// flow 9 of `OTHER_WS` with its own versions.
async fn published_flow() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, Some(20))?;
        version(
            tx,
            10,
            WS,
            FLOW,
            (true, false),
            1,
            &json!([{ "id": "draft-node" }]),
        )?;
        version(
            tx,
            20,
            WS,
            FLOW,
            (false, true),
            2,
            &json!([{ "id": "live-node" }]),
        )?;
        flow(tx, 9, OTHER_WS, Some(91))?;
        version(tx, 90, OTHER_WS, 9, (true, false), 1, &json!([]))?;
        version(tx, 91, OTHER_WS, 9, (false, true), 1, &json!([]))
    })
    .await;
    db
}

// describe("flowVersionService.findDraft")

#[tokio::test]
async fn finds_the_draft_version_within_the_requested_workspace() {
    let db = published_flow().await;
    let draft = flow_version::find_draft(&db, WS, FLOW).await.expect("read");
    assert_eq!(draft.map(|v| (v.id, v.is_draft)), Some((10, true)));
    assert!(
        flow_version::find_draft(&db, OTHER_WS, FLOW)
            .await
            .expect("read")
            .is_none()
    );
}

// describe("flowVersionService.findPublished")

#[tokio::test]
async fn finds_the_latest_published_version_within_the_requested_workspace() {
    let db = published_flow().await;
    let live = flow_version::find_published(&db, WS, FLOW)
        .await
        .expect("read");
    assert_eq!(
        live.map(|v| (v.id, v.is_draft, v.is_latest)),
        Some((20, false, true))
    );
    assert!(
        flow_version::find_published(&db, OTHER_WS, FLOW)
            .await
            .expect("read")
            .is_none()
    );
}

#[tokio::test]
async fn find_published_returns_undefined_when_the_flow_has_never_been_published() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, None)?;
        version(tx, 10, WS, FLOW, (true, false), 1, &json!([]))
    })
    .await;
    assert!(
        flow_version::find_published(&db, WS, FLOW)
            .await
            .expect("read")
            .is_none()
    );
}

// describe("flowVersionService.revertDraftToPublished")

#[tokio::test]
async fn copies_the_published_content_into_the_draft_row() {
    let db = published_flow().await;
    let (nodes, edges) = flow_version::revert_draft_to_published(&db, WS, FLOW)
        .await
        .expect("revert");
    assert_eq!(nodes, json!([{ "id": "live-node" }]));
    assert_eq!(edges, json!([{ "id": "edge-20" }]));
    let draft = rows(&db, r#"SELECT * FROM "FlowVersion" WHERE "id" = ?1"#, 10)
        .await
        .remove(0);
    assert_eq!(
        draft["nodes"],
        json!(json!([{ "id": "live-node" }]).to_string())
    );
    assert_eq!(
        draft["edges"],
        json!(json!([{ "id": "edge-20" }]).to_string())
    );
    assert_eq!(draft["startNodeId"], json!(2));
    assert_eq!(draft["isDraft"], json!(1));
    // The other workspace's draft is not touched.
    let foreign = rows(&db, r#"SELECT * FROM "FlowVersion" WHERE "id" = ?1"#, 90)
        .await
        .remove(0);
    assert_eq!(foreign["nodes"], json!("[]"));
}

#[tokio::test]
async fn revert_throws_when_no_published_version_exists() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, None)?;
        version(tx, 10, WS, FLOW, (true, false), 1, &json!([]))
    })
    .await;
    let err = flow_version::revert_draft_to_published(&db, WS, FLOW)
        .await
        .expect_err("no published version");
    assert_eq!(err.message(), "Flow version not found");
}

// describe("flowVersionService.findForButtonPayload")

#[tokio::test]
async fn reads_the_pinned_version_when_the_payload_carries_a_version_id() {
    let db = published_flow().await;
    let pinned = flow_version::find_for_button_payload(&db, WS, FLOW, Some(10))
        .await
        .expect("read");
    assert_eq!(pinned.map(|v| v.id), Some(10));
    // Scoped to the workspace.
    let foreign = flow_version::find_for_button_payload(&db, WS, FLOW, Some(91))
        .await
        .expect("read");
    assert!(foreign.is_none());
}

#[tokio::test]
async fn resolves_the_flow_s_published_version_when_the_payload_carries_no_version_id() {
    let db = published_flow().await;
    let live = flow_version::find_for_button_payload(&db, WS, FLOW, None)
        .await
        .expect("read");
    assert_eq!(live.map(|v| v.id), Some(20));
}

#[tokio::test]
async fn never_queries_versions_by_an_undefined_id() {
    // A flow with no published version never widens to "any version".
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, None)?;
        version(tx, 10, WS, FLOW, (true, false), 1, &json!([]))?;
        flow(tx, 2, WS, Some(21))?;
        version(tx, 21, WS, 2, (false, true), 1, &json!([]))
    })
    .await;
    let out = flow_version::find_for_button_payload(&db, WS, FLOW, None)
        .await
        .expect("read");
    assert!(out.is_none());
}

#[tokio::test]
async fn find_for_button_payload_returns_undefined_when_the_flow_has_never_been_published() {
    let db = two_workspaces().await;
    setup(&db, |tx| flow(tx, FLOW, WS, None)).await;
    let out = flow_version::find_for_button_payload(&db, WS, FLOW, None)
        .await
        .expect("read");
    assert!(out.is_none());
}

#[tokio::test]
async fn returns_undefined_when_the_flow_no_longer_exists() {
    let db = two_workspaces().await;
    let out = flow_version::find_for_button_payload(&db, WS, FLOW, None)
        .await
        .expect("read");
    assert!(out.is_none());
}

// describe("flowVersionService.publishedVersionNumber")

#[tokio::test]
async fn is_0_for_a_flow_that_was_never_published() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, None)?;
        version(tx, 10, WS, FLOW, (true, false), 1, &json!([]))
    })
    .await;
    assert_eq!(
        flow_version::published_version_number(&db, WS, FLOW)
            .await
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn counts_published_versions_up_to_and_including_the_live_one() {
    // v1..v5 published and v2 restored: only v1 and v2 are <= v2's id.
    let db = two_workspaces().await;
    setup(&db, |tx| {
        flow(tx, FLOW, WS, Some(200))?;
        version(tx, 10, WS, FLOW, (true, false), 1, &json!([]))?;
        for id in [100, 200, 300, 400, 500] {
            version(tx, id, WS, FLOW, (false, id == 200), 1, &json!([]))?;
        }
        flow(tx, 9, OTHER_WS, None)?;
        version(tx, 150, OTHER_WS, 9, (false, false), 1, &json!([]))
    })
    .await;
    assert_eq!(
        flow_version::published_version_number(&db, WS, FLOW)
            .await
            .expect("count"),
        2
    );
}
