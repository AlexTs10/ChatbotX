//! `pipeline/flow-publish-check.ts`: a flow cannot be published while an
//! "Add to pipeline" step leaves a required pipeline field empty.

use chatbotx_db::Db;
use serde_json::Value;

use crate::errors::Result;

/// `assertFlowPipelineStepsPublishable` (`nodes` = the flow version nodes).
pub async fn assert_flow_pipeline_steps_publishable(
    _db: &Db,
    _workspace_id: i64,
    _nodes: &Value,
) -> Result<()> {
    todo!("P3.6-D1p")
}
