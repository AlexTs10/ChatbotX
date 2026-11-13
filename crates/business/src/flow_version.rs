//! `flow-version/service.ts` (P3.6-D2): `findDraft`, `findPublished`,
//! `publishedVersionNumber`, `list`, `findForButtonPayload`,
//! `revertDraftToPublished`, `restore`, `updateDraftByFlowId`, `publish`,
//! `afterPublishIfLive`. No versions-list cache, no template locks, no
//! welcome-message sync / quick-reply cleanup jobs, no audit records.

use chatbotx_db::Db;
use chatbotx_db::repo::models::FlowVersion;
use serde_json::Value;

use crate::Result;

/// `findDraft`.
pub async fn find_draft(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Option<FlowVersion>> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `findPublished`: the latest published version.
pub async fn find_published(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Option<FlowVersion>> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `publishedVersionNumber`: the published versions up to the live one (0 = never published).
pub async fn published_version_number(db: &Db, workspace_id: i64, flow_id: i64) -> Result<i64> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `list`: the published versions, latest first.
pub async fn list(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Vec<FlowVersion>> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `findForButtonPayload`: the pinned version, else the flow's published one.
pub async fn find_for_button_payload(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    version_id: Option<i64>,
) -> Result<Option<FlowVersion>> {
    let _ = (db, workspace_id, flow_id, version_id);
    todo!("P3.6-D2")
}

/// `revertDraftToPublished`: copies the published graph into the draft;
/// returns `(nodes, edges)`.
pub async fn revert_draft_to_published(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<(Value, Value)> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `restore`: makes `version` the live one and copies it into the draft.
pub async fn restore(db: &Db, version: FlowVersion) -> Result<()> {
    let _ = (db, version);
    todo!("P3.6-D2")
}

/// `updateDraftByFlowId`: overwrites the draft graph (not validated).
pub async fn update_draft_by_flow_id(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    nodes: Vec<Value>,
    edges: Vec<Value>,
) -> Result<()> {
    let _ = (db, workspace_id, flow_id, nodes, edges);
    todo!("P3.6-D2")
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
    let _ = (db, workspace_id, flow_id, nodes, edges);
    todo!("P3.6-D2")
}

/// `afterPublishIfLive`: whether the after-publish effects run (the version
/// is still the flow's live one).
pub async fn after_publish_if_live(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
    version_id: i64,
) -> Result<bool> {
    let _ = (db, workspace_id, flow_id, version_id);
    todo!("P3.6-D2")
}
