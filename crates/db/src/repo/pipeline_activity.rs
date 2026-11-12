//! Port of `repositories/pipeline/activity.ts` (`pipelineActivityRepository`). Money
//! sums (`toValue`) are x 10,000 integers.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::PipelineItemActivity;
use super::pipeline_item_cursor::NewestFirstKeyset;
use super::pipeline_item_filter::PIPELINE_AUTOMATION_ACTOR_KINDS;
use super::queries::date_bucket::{Granularity, zoned_bucket_key};
use super::pipeline_item_cursor::bind1;
use super::sql::Sql;
use super::{Model, Result, new_id, text_list};

/// `PipelineActivityInsert` (`version` defaults to 1 when 0).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineActivityInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub item_number: i64,
    pub item_title: String,
    pub contact_id: i64,
    pub kind: String,
    pub actor_kind: String,
    pub actor_user_id: Option<i64>,
    pub actor_source_id: Option<String>,
    pub from_stage_id: Option<i64>,
    pub to_stage_id: Option<i64>,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub from_value: Option<i64>,
    pub to_value: Option<i64>,
    pub from_priority: Option<String>,
    pub to_priority: Option<String>,
    pub from_due_date: Option<i64>,
    pub to_due_date: Option<i64>,
    pub from_title: Option<String>,
    pub to_title: Option<String>,
    pub from_pipeline_id: Option<i64>,
    pub version: i64,
    pub occurred_at: i64,
}

/// `PipelineActivityPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineActivityPage {
    pub items: Vec<PipelineItemActivity>,
    pub next_cursor: Option<String>,
}

/// Input of `listForItem`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListActivityInput {
    pub workspace_id: i64,
    pub item_id: i64,
    pub limit: i64,
    pub cursor: Option<String>,
    /// Only these kinds; every kind when `None`.
    pub kinds: Option<Vec<String>>,
}

/// `PipelineAnalyticsRange` (unix ms, inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineAnalyticsRange {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub from: i64,
    pub to: i64,
}

/// One row of `dailySeries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailySeriesRow {
    pub day: String,
    pub kind: String,
    pub to_status: Option<String>,
    pub count: i64,
    pub value: i64,
}

/// One row of `totalsByActor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorTotalsRow {
    pub is_automation: bool,
    pub actor_user_id: Option<i64>,
    pub kind: String,
    pub to_status: Option<String>,
    pub count: i64,
    pub value: i64,
}

/// `deleteByPipelineBatch`: deletes up to `limit` activity rows of a pipeline.
pub fn delete_by_pipeline_batch(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    limit: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// `scrubContactTitles`: clears the item titles kept in a deleted contact's history.
pub fn scrub_contact_titles(conn: &Connection, workspace_id: i64, contact_ids: &[i64]) -> Result<()> {
    todo!("P3.3")
}

/// `insertMany`.
pub fn insert_many(conn: &Connection, rows: &[PipelineActivityInsert]) -> Result<()> {
    todo!("P3.3")
}

/// `listForItem`: newest-first keyset page of an item's activity.
pub fn list_for_item(conn: &Connection, input: &ListActivityInput) -> Result<PipelineActivityPage> {
    todo!("P3.3")
}

const ANALYTICS_WHERE: &str = "\"workspaceId\" = ? AND \"pipelineId\" = ? AND \"kind\" IN ('created', 'statusChanged') AND \"occurredAt\" >= ? AND \"occurredAt\" <= ?";

fn analytics_params(range: &PipelineAnalyticsRange) -> Vec<Value> {
    vec![
        Value::Integer(range.workspace_id),
        Value::Integer(range.pipeline_id),
        Value::Integer(range.from),
        Value::Integer(range.to),
    ]
}

/// `dailySeries`: created / won / lost transitions per local day, with summed value.
pub fn daily_series(
    conn: &Connection,
    range: &PipelineAnalyticsRange,
    timezone: &str,
) -> Result<Vec<DailySeriesRow>> {
    todo!("P3.3")
}

/// `totalsByActor`: the same transitions grouped by user or automation.
pub fn totals_by_actor(conn: &Connection, range: &PipelineAnalyticsRange) -> Result<Vec<ActorTotalsRow>> {
    todo!("P3.3")
}
