//! `flow/service.ts` (`createDraft`, `createPublished`,
//! `createPublishedDefault`), `flow-version/assert-publishable.ts`,
//! `capabilities/service.ts` `getFlowAuthoringContext` (flows and custom
//! fields) and the builder's `lib/resolve-flow-graph-input.ts` /
//! `lib/compile-spec-to-graph.ts` (P3.6-D2: the create path).

use chatbotx_db::Db;
use serde_json::Value;

use crate::Result;

/// `ResolvedFlowGraph`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowGraph {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub start_node_id: String,
    /// Authored id -> internal id (`nodeIds`; raw graphs only).
    pub node_ids: Option<Vec<(String, String)>>,
}

/// The graph part of a create / publish body.
#[derive(Debug, Clone, Default)]
pub struct FlowGraphInput {
    pub spec: Option<Value>,
    pub nodes: Option<Vec<Value>>,
    pub edges: Option<Vec<Value>>,
}

/// `resolveFlowGraphInput`: compile a spec (with the workspace authoring
/// context) or normalize a raw graph; `validate` = the publish schema check.
/// A rejected graph is `422 invalidRequestData` whose `data` is the
/// `FlowAuthoringException` errors.
pub async fn resolve_graph_input(
    _db: &Db,
    _workspace_id: i64,
    _input: FlowGraphInput,
    _validate: bool,
) -> Result<Option<FlowGraph>> {
    todo!("P3.6-D2")
}

/// `flowService.createDraft`: the folder check, the flow, its analytics
/// session and one draft version (the default one-node graph when `graph`
/// is `None`). Returns the flow id.
pub async fn create_draft(
    _db: &Db,
    _workspace_id: i64,
    _name: &str,
    _folder_id: Option<i64>,
    _graph: Option<FlowGraph>,
) -> Result<i64> {
    todo!("P3.6-D2")
}

/// `flowService.createPublished`: the publish check, the folder check, the
/// flow with a draft and a published (latest) version. Returns the flow id.
pub async fn create_published(
    _db: &Db,
    _workspace_id: i64,
    _name: &str,
    _folder_id: Option<i64>,
    _graph: FlowGraph,
) -> Result<i64> {
    todo!("P3.6-D2")
}
