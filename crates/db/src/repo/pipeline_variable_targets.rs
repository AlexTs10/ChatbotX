//! Port of `repositories/pipeline/variable-targets.ts`
//! (`pipelineVariableTargetRepository`). `DISTINCT ON (pipelineId)` is a
//! `row_number() OVER (PARTITION BY ...)` window; `array_agg` is `json_group_array` over an
//! ordered subquery (`None` when the item has no assignee, like Postgres `NULL`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Json, Result};

/// One row of `listForContact`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineVariableTarget {
    pub item_id: i64,
    pub title: String,
    pub value: Option<i64>,
    pub status: String,
    pub priority: String,
    pub due_date: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub pipeline_id: i64,
    pub reference_name: String,
    pub currency: Option<String>,
    pub settings: Json,
    pub stage_name: String,
    pub target_country: Option<String>,
    /// Display names (member name, else team name), oldest assignee first.
    pub assignees: Option<Vec<Option<String>>>,
}

/// `listForContact`: one row per pipeline the contact has items in (its most recently
/// created open item, else its most recently created item), in pipeline id order.
pub fn list_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<PipelineVariableTarget>> {
    todo!("P3.3")
}

/// `findLastTouchedItemId`: the contact's most recently created or updated item in any
/// pipeline.
pub fn find_last_touched_item_id(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<Option<i64>> {
    todo!("P3.3")
}
