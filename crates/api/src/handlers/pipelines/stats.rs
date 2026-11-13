//! `pipelines.getStats` (`pipelineStatsPublicRequest`).

use crate::orpc::OrpcError;

/// `pipelineStatsPublicRequest.parse`: (`from`, `to`, `timezone`).
pub fn stats_query(
    _params: &[(&'static str, String)],
    _query: Option<&str>,
) -> Result<(String, String, Option<String>), OrpcError> {
    todo!("P3.6-D1p")
}
