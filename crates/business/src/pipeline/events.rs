//! The `pipelineItemChanged` realtime events the pipeline services queue
//! (`realtimeEffect`). The business crate has no realtime handle: services
//! return the events and the caller publishes them.

/// `{ eventType: "pipelineItemChanged", data }` for one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineItemChanged {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: Option<i64>,
    pub contact_id: Option<i64>,
    /// `created` / `updated` / `moved` / `deleted`.
    pub change: &'static str,
}
