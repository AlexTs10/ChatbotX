//! Port of `repositories/flow/repository.ts` (`flowRepository`, P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::{Flow, FlowVersion};
use super::utils::SortItem;

/// `rootFolderId` (`partials/folder.ts`).
pub const ROOT_FOLDER_ID: &str = "0";

/// `FlowListInput` (`folder_id` is the API string: `"0"` = root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowListInput {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub folder_id: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// A flow with its `flowVersions` (by id).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowWithVersions {
    pub flow: Flow,
    pub flow_versions: Vec<FlowVersion>,
}

/// `{ id, name }` of `listPublishedOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowOption {
    pub id: i64,
    pub name: String,
}

/// One row of `listCurrentVersionNodesByFlowIds`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowCurrentVersionNodes {
    pub flow_id: i64,
    pub flow_name: String,
    pub nodes: Vec<serde_json::Value>,
}

/// `listNamesWithPrefix`: names of the workspace's flows that start with `prefix`
/// (plain text, case-sensitive).
pub fn list_names_with_prefix(
    _conn: &Connection,
    _workspace_id: i64,
    _prefix: &str,
) -> Result<Vec<String>> {
    todo!("P3.2b")
}

/// `existsInWorkspace`.
pub fn exists_in_workspace(_conn: &Connection, _workspace_id: i64, _id: i64) -> Result<bool> {
    todo!("P3.2b")
}

/// `listPublishedOptions`: active flows with a published version (by id).
pub fn list_published_options(_conn: &Connection, _workspace_id: i64) -> Result<Vec<FlowOption>> {
    todo!("P3.2b")
}

/// `listWithVersions`: the page of flows, each with its draft and latest versions.
pub fn list_with_versions(
    _conn: &Connection,
    _input: &FlowListInput,
) -> Result<Vec<FlowWithVersions>> {
    todo!("P3.2b")
}

/// `count` (same filter as `listWithVersions`).
pub fn count(_conn: &Connection, _input: &FlowListInput) -> Result<i64> {
    todo!("P3.2b")
}

/// `findWithVersions`: the flow with all its versions.
pub fn find_with_versions(
    _conn: &Connection,
    _workspace_id: i64,
    _id: i64,
) -> Result<Option<FlowWithVersions>> {
    todo!("P3.2b")
}

/// `findCurrentVersionNodes`: `None` when the flow is not in the workspace, an empty
/// list when it has no published version.
pub fn find_current_version_nodes(
    _conn: &Connection,
    _workspace_id: i64,
    _flow_id: i64,
) -> Result<Option<Vec<serde_json::Value>>> {
    todo!("P3.2b")
}

/// `listCurrentVersionNodesByFlowIds` (flows of another workspace are absent).
pub fn list_current_version_nodes_by_flow_ids(
    _conn: &Connection,
    _workspace_id: i64,
    _flow_ids: &[i64],
) -> Result<Vec<FlowCurrentVersionNodes>> {
    todo!("P3.2b")
}

/// `listIdsByIds`: the ids of `ids` that exist in the workspace.
pub fn list_ids_by_ids(_conn: &Connection, _workspace_id: i64, _ids: &[i64]) -> Result<Vec<i64>> {
    todo!("P3.2b")
}
