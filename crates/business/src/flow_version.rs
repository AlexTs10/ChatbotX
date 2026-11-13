//! `flow-version/service.ts` (P3.6-D2): `findDraft`, `findPublished`,
//! `publishedVersionNumber`, `list`, `findForButtonPayload`,
//! `revertDraftToPublished`, `restore`, `updateDraftByFlowId`, `publish`,
//! `afterPublishIfLive`. No versions-list cache, no template locks, no
//! welcome-message sync / quick-reply cleanup jobs, no audit records, no
//! decision-profile / pipeline publish checks.

use chatbotx_core::flow_config::authoring::normalize_flow_graph_decision_mappings;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Flow, FlowVersion};
use chatbotx_db::repo::{find_many, find_one};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value;

use crate::flow::assert_flow_graph_publishable;
use crate::{BusinessError, Result, tx};

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn new_id() -> i64 {
    chatbotx_core::utils::id::create_id()
        .parse()
        .unwrap_or_default()
}

fn draft_of(conn: &Connection, workspace_id: i64, flow_id: i64) -> Result<Option<FlowVersion>> {
    Ok(find_one::<FlowVersion>(
        conn,
        "\"flowId\" = ?1 AND \"workspaceId\" = ?2 AND \"isDraft\" = 1",
        &[&flow_id, &workspace_id],
    )?)
}

fn published_of(conn: &Connection, workspace_id: i64, flow_id: i64) -> Result<Option<FlowVersion>> {
    Ok(find_one::<FlowVersion>(
        conn,
        "\"flowId\" = ?1 AND \"workspaceId\" = ?2 AND \"isDraft\" = 0 AND \"isLatest\" = 1",
        &[&flow_id, &workspace_id],
    )?)
}

fn by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<FlowVersion>> {
    Ok(find_one::<FlowVersion>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?)
}

fn flow_of(conn: &Connection, workspace_id: i64, flow_id: i64) -> Result<Option<Flow>> {
    Ok(find_one::<Flow>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&flow_id, &workspace_id],
    )?)
}

/// `findDraft`.
pub async fn find_draft(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Option<FlowVersion>> {
    tx::read(db, move |conn| draft_of(conn, workspace_id, flow_id)).await
}

/// `findPublished`: the latest published version.
pub async fn find_published(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Option<FlowVersion>> {
    tx::read(db, move |conn| published_of(conn, workspace_id, flow_id)).await
}

/// `publishedVersionNumber`: the published versions up to the live one (0 = never published).
pub async fn published_version_number(db: &Db, workspace_id: i64, flow_id: i64) -> Result<i64> {
    tx::read(db, move |conn| {
        let Some(live) = published_of(conn, workspace_id, flow_id)? else {
            return Ok(0);
        };
        conn.query_row(
            r#"SELECT COUNT(*) FROM "FlowVersion" WHERE "workspaceId" = ?1 AND "flowId" = ?2
                 AND "isDraft" = 0 AND "id" <= ?3"#,
            params![workspace_id, flow_id, live.id],
            |r| r.get(0),
        )
        .map_err(repo_err)
    })
    .await
}

/// `list`: the published versions, latest first.
pub async fn list(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Vec<FlowVersion>> {
    tx::read(db, move |conn| {
        Ok(find_many::<FlowVersion>(
            conn,
            "WHERE \"flowId\" = ?1 AND \"workspaceId\" = ?2 AND \"isDraft\" = 0 \
             ORDER BY \"isLatest\" DESC, \"createdAt\" DESC, \"id\" DESC",
            &[&flow_id, &workspace_id],
        )?)
    })
    .await
}

/// `findForButtonPayload`: the pinned version, else the flow's published one.
pub async fn find_for_button_payload(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    version_id: Option<i64>,
) -> Result<Option<FlowVersion>> {
    tx::read(db, move |conn| {
        if let Some(version_id) = version_id {
            return by_id(conn, workspace_id, version_id);
        }
        let Some(current) =
            flow_of(conn, workspace_id, flow_id)?.and_then(|f| f.current_version_id)
        else {
            return Ok(None);
        };
        by_id(conn, workspace_id, current)
    })
    .await
}

fn set_draft_graph(
    conn: &Connection,
    flow_id: i64,
    (nodes, edges): (&Value, &Value),
    start_node_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        r#"UPDATE "FlowVersion" SET "nodes" = ?1, "edges" = ?2,
               "startNodeId" = COALESCE(?3, "startNodeId"),
               "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
           WHERE "flowId" = ?4 AND "isDraft" = 1"#,
        params![nodes.to_string(), edges.to_string(), start_node_id, flow_id],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `revertDraftToPublished`: copies the published graph into the draft;
/// returns `(nodes, edges)`.
pub async fn revert_draft_to_published(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<(Value, Value)> {
    tx::write(db, move |tx| {
        let published = published_of(tx, workspace_id, flow_id)?
            .ok_or_else(|| BusinessError::not_found("Flow version not found"))?;
        set_draft_graph(
            tx,
            flow_id,
            (&published.nodes.0, &published.edges.0),
            Some(published.start_node_id),
        )?;
        Ok((published.nodes.0, published.edges.0))
    })
    .await
}

fn make_latest(conn: &Connection, flow_id: i64, version_id: i64) -> Result<()> {
    conn.execute(
        r#"UPDATE "FlowVersion" SET "isLatest" = 0 WHERE "flowId" = ?1 AND "isLatest" = 1"#,
        params![flow_id],
    )
    .map_err(repo_err)?;
    conn.execute(
        r#"UPDATE "FlowVersion" SET "isLatest" = 1 WHERE "id" = ?1"#,
        params![version_id],
    )
    .map_err(repo_err)?;
    conn.execute(
        r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
        params![version_id, flow_id],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `restore`: makes `version` the live one and copies it into the draft.
pub async fn restore(db: &Db, version: FlowVersion) -> Result<()> {
    assert_flow_graph_publishable(&version.nodes.0, &version.edges.0)?;
    tx::write(db, move |tx| {
        make_latest(tx, version.flow_id, version.id)?;
        set_draft_graph(
            tx,
            version.flow_id,
            (&version.nodes.0, &version.edges.0),
            Some(version.start_node_id),
        )
    })
    .await
}

/// `updateDraftByFlowId`: overwrites the draft graph (not validated).
pub async fn update_draft_by_flow_id(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    nodes: Vec<Value>,
    edges: Vec<Value>,
) -> Result<()> {
    let nodes = Value::Array(normalize_flow_graph_decision_mappings(&nodes));
    let edges = Value::Array(edges);
    tx::write(db, move |tx| {
        let draft = draft_of(tx, workspace_id, flow_id)?
            .ok_or_else(|| BusinessError::not_found("Draft flow version not found"))?;
        tx.execute(
            r#"UPDATE "FlowVersion" SET "nodes" = ?1, "edges" = ?2,
                   "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) WHERE "id" = ?3"#,
            params![nodes.to_string(), edges.to_string(), draft.id],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `publish`: validates the graph, syncs the draft and adds the new latest
/// published version (the flow's `currentVersionId`). Returns its id.
pub async fn publish(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    nodes: Vec<Value>,
    edges: Vec<Value>,
) -> Result<i64> {
    let nodes = Value::Array(normalize_flow_graph_decision_mappings(&nodes));
    let edges = Value::Array(edges);
    assert_flow_graph_publishable(&nodes, &edges)?;
    tx::write(db, move |tx| {
        flow_of(tx, workspace_id, flow_id)?
            .ok_or_else(|| BusinessError::not_found("Flow not found"))?;
        let draft = draft_of(tx, workspace_id, flow_id)?
            .ok_or_else(|| BusinessError::not_found("Flow not found"))?;
        let version_id = new_id();
        tx.execute(
            r#"UPDATE "FlowVersion" SET "isLatest" = 0 WHERE "flowId" = ?1 AND "isLatest" = 1"#,
            params![flow_id],
        )
        .map_err(repo_err)?;
        set_draft_graph(tx, flow_id, (&nodes, &edges), None)?;
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft",
                   "isLatest", "startNodeId")
               VALUES (?1, ?2, ?3, ?4, ?5, 0, 1, ?6)"#,
            params![
                version_id,
                workspace_id,
                flow_id,
                nodes.to_string(),
                edges.to_string(),
                draft.start_node_id
            ],
        )
        .map_err(repo_err)?;
        tx.execute(
            r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
            params![version_id, flow_id],
        )
        .map_err(repo_err)?;
        Ok(version_id)
    })
    .await
}

/// `afterPublishIfLive`: whether the after-publish effects run (the version
/// is still the flow's live one).
pub async fn after_publish_if_live(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    version_id: i64,
) -> Result<bool> {
    tx::read(db, move |conn| {
        Ok(flow_of(conn, workspace_id, flow_id)?
            .is_some_and(|flow| flow.current_version_id == Some(version_id)))
    })
    .await
}
