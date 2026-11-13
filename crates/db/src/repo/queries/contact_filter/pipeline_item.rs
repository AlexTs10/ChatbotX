//! Port of `queries/contact-filter/pipeline-item.ts`: the "pipeline item" condition. Every
//! shape is a workspace-scoped correlated `EXISTS` over `"PipelineItem"` (positive operators)
//! or its `NOT EXISTS` twin (`isEmpty`, `notIn`); a condition that cannot be evaluated fails
//! closed (`FALSE`).

use super::FilterCondition;
use crate::repo::sql::Sql;

/// `buildPipelineItemConditionWhere`.
pub(super) fn pipeline_item_where(_c: &FilterCondition, _workspace_id: Option<i64>) -> Sql {
    todo!("P3.2b")
}
