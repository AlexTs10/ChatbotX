//! Port of `repositories/pipeline/stage.ts` (`pipelineStageRepository`). Row locks
//! (`FOR SHARE` / `FOR UPDATE`) are plain reads under the one SQLite writer. Every UPDATE
//! sets `"updatedAt"` (Drizzle `$onUpdate`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::PipelineStage;
use super::patch::{self, Patch};
use super::sql::Sql;
use super::{Error, Model, Result, find_many, find_one, id_list, new_id, now_ms, returning};

/// `PipelineStageInsert`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineStageInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub text_color: Option<String>,
    pub background_color: Option<String>,
    pub position: i64,
    pub bulk_job_id: Option<String>,
}

/// One row of `listNamesByIds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStageName {
    pub id: i64,
    pub name: String,
    pub pipeline_id: i64,
}

/// `listByPipeline`: in position order.
pub fn list_by_pipeline(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<Vec<PipelineStage>> {
    todo!("P3.3")
}

/// `listByPipelineIds`: stages of several pipelines, in board order.
pub fn list_by_pipeline_ids(
    conn: &Connection,
    workspace_id: i64,
    pipeline_ids: &[i64],
) -> Result<Vec<PipelineStage>> {
    todo!("P3.3")
}

/// `listNamesByIds`: names (and pipelines) of the given stages; deleted ones are absent.
pub fn list_names_by_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<PipelineStageName>> {
    todo!("P3.3")
}

/// `findById` (`pipeline_id` narrows when given).
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: Option<i64>,
    id: i64,
) -> Result<Option<PipelineStage>> {
    todo!("P3.3")
}

/// `lockShared`: the stage row (the write job is the lock).
pub fn lock_shared(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineStage>> {
    todo!("P3.3")
}

/// `lockExclusive`: the stage row (the write job is the lock).
pub fn lock_exclusive(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineStage>> {
    todo!("P3.3")
}

fn insert_row(conn: &Connection, values: &PipelineStageInsert) -> Result<Option<PipelineStage>> {
    Ok(returning::<PipelineStage>(
        conn,
        "INSERT INTO \"PipelineStage\" (\"id\", \"workspaceId\", \"pipelineId\", \"name\", \"description\", \"textColor\", \"backgroundColor\", \"position\", \"bulkJobId\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        &[
            &new_id(),
            &values.workspace_id,
            &values.pipeline_id,
            &values.name,
            &values.description,
            &values.text_color,
            &values.background_color,
            &values.position,
            &values.bulk_job_id,
        ],
    )?
    .into_iter()
    .next())
}

/// `insert`.
pub fn insert(conn: &Connection, values: &PipelineStageInsert) -> Result<PipelineStage> {
    todo!("P3.3")
}

/// `insertMany`: the inserted rows in input order.
pub fn insert_many(conn: &Connection, rows: &[PipelineStageInsert]) -> Result<Vec<PipelineStage>> {
    todo!("P3.3")
}

/// `update`: an empty patch answers with the row unchanged.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<PipelineStage>> {
    todo!("P3.3")
}

/// `releaseBulkClaim`: clears the claim only while it is still `bulk_job_id`.
pub fn release_bulk_claim(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    bulk_job_id: &str,
) -> Result<bool> {
    todo!("P3.3")
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, pipeline_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `nextPosition`: `max(position) + 1`, or 0.
pub fn next_position(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<i64> {
    todo!("P3.3")
}

/// `reorder`: `position = index` for the given ids in one statement.
pub fn reorder(conn: &Connection, workspace_id: i64, pipeline_id: i64, stage_ids: &[i64]) -> Result<()> {
    todo!("P3.3")
}
