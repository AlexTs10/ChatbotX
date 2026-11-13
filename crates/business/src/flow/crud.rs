//! `flow/service.ts`: `listPublishedOptions`, `list`, `listNodes`,
//! `findById`, `update`, `duplicate`, `deleteMany`, `createPublishedDefault`
//! and `messengerWelcomeMessageFlowService.listCurrentByFlow` (P3.6-D2).
//! No audit records, no template locks / guards, no welcome-message sync
//! jobs: the functions return what the jobs would need.

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::flow::{FlowOption, FlowWithVersions};
use chatbotx_db::rusqlite::Connection;
use serde_json::Value;

use crate::Result;

/// `flowService.listPublishedOptions`.
pub async fn list_published_options(db: &Db, workspace_id: i64) -> Result<Vec<FlowOption>> {
    let _ = (db, workspace_id);
    todo!("P3.6-D2")
}

/// The input of [`list`] (`FlowListInput` + `startType`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFlows {
    pub workspace_id: i64,
    pub name: Option<String>,
    /// The API string (`"0"` = no folder).
    pub folder_id: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
    pub start_type: Option<String>,
    pub integration_whatsapp_id: Option<i64>,
}

/// One page of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlowListPage {
    pub data: Vec<FlowWithVersions>,
    pub page_count: i64,
}

/// `flowService.list`.
pub async fn list(db: &Db, input: ListFlows) -> Result<FlowListPage> {
    let _ = (db, input);
    todo!("P3.6-D2")
}

/// `FlowNodeSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowNodeSummary {
    pub flow_id: i64,
    pub node_id: String,
    pub label: String,
    pub node_type: Option<String>,
}

/// `flowService.listNodes`: the nodes of the published version.
pub async fn list_nodes(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Vec<FlowNodeSummary>> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// `flowService.findById`: the flow with all its versions.
pub async fn find_by_id(db: &Db, workspace_id: i64, id: i64) -> Result<FlowWithVersions> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D2")
}

/// One row of `messengerWelcomeMessageFlowService.listCurrentByFlow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeMessageFlow {
    pub integration_messenger_id: i64,
    pub page_id: String,
    pub welcome_message_flow_id: String,
}

/// `messengerWelcomeMessageFlowService.listCurrentByFlow` (oldest first).
pub async fn list_welcome_message_flows(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<WelcomeMessageFlow>> {
    let _ = (db, workspace_id, flow_id);
    todo!("P3.6-D2")
}

/// The data of [`update`] (`None` = not given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowUpdate {
    pub name: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
}

/// The reason of a welcome-message flow sync (`enqueueSync`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeSync {
    Rename,
    Publish,
}

/// What [`update`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowUpdated {
    /// The keys whose value changed (`changedKeys`), in input order; empty = no write.
    pub changed_keys: Vec<&'static str>,
    /// The sync the original queues after the commit.
    pub sync: Option<WelcomeSync>,
}

/// `flowService.update`: a partial update of name / active / enableInInbox.
pub async fn update(db: &Db, workspace_id: i64, id: i64, data: FlowUpdate) -> Result<FlowUpdated> {
    let _ = (db, workspace_id, id, data);
    todo!("P3.6-D2")
}

/// `flowService.duplicate`: returns the id of the copy.
pub async fn duplicate(db: &Db, workspace_id: i64, id: i64) -> Result<i64> {
    let _ = (db, workspace_id, id);
    todo!("P3.6-D2")
}

/// `flowService.deleteMany`: returns the deleted ids (the ids of the
/// original `enqueueDelete` job; empty = no job).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Vec<i64>> {
    let _ = (db, workspace_id, ids);
    todo!("P3.6-D2")
}

/// The input of [`create_published_default`].
#[derive(Debug, Clone, PartialEq)]
pub struct PublishedDefault {
    pub workspace_id: i64,
    pub name: String,
    pub folder_id: Option<i64>,
    pub start_node_id: i64,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
}

/// The ids [`create_published_default`] inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedDefaultIds {
    pub flow_id: i64,
    pub draft_version_id: i64,
    pub published_version_id: i64,
}

/// `flowService.createPublishedDefault` (template install, caller's
/// transaction): the flow, its analytics session, a draft and a published
/// version with the same graph.
pub fn create_published_default(
    tx: &Connection,
    input: PublishedDefault,
) -> Result<PublishedDefaultIds> {
    let _ = (tx, input);
    todo!("P3.6-D2")
}
