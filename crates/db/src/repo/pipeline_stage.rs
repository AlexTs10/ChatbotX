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
pub fn list_by_pipeline(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
) -> Result<Vec<PipelineStage>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 ORDER BY \"position\" ASC",
        &[&workspace_id, &pipeline_id],
    )
}

/// `listByPipelineIds`: stages of several pipelines, in board order.
pub fn list_by_pipeline_ids(
    conn: &Connection,
    workspace_id: i64,
    pipeline_ids: &[i64],
) -> Result<Vec<PipelineStage>> {
    if pipeline_ids.is_empty() {
        return Ok(Vec::new());
    }
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"pipelineId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"pipelineId\", \"position\"",
        &[&workspace_id, &id_list(pipeline_ids)],
    )
}

/// `listNamesByIds`: names (and pipelines) of the given stages; deleted ones are absent.
pub fn list_names_by_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<PipelineStageName>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"name\", \"pipelineId\" FROM \"PipelineStage\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?))",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |row| {
        Ok(PipelineStageName {
            id: row.get(0)?,
            name: row.get(1)?,
            pipeline_id: row.get(2)?,
        })
    })
}

/// `findById` (`pipeline_id` narrows when given).
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: Option<i64>,
    id: i64,
) -> Result<Option<PipelineStage>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2 AND (?3 IS NULL OR \"pipelineId\" = ?3)",
        &[&workspace_id, &id, &pipeline_id],
    )
}

/// `lockShared`: the stage row (the write job is the lock).
pub fn lock_shared(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineStage>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )
}

/// `lockExclusive`: the stage row (the write job is the lock).
pub fn lock_exclusive(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PipelineStage>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )
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
    insert_row(conn, values)?.ok_or_else(|| Error::msg("Failed to insert pipeline stage"))
}

/// `insertMany`: the inserted rows in input order.
pub fn insert_many(conn: &Connection, rows: &[PipelineStageInsert]) -> Result<Vec<PipelineStage>> {
    let mut out = Vec::with_capacity(rows.len());
    for values in rows {
        out.extend(insert_row(conn, values)?);
    }
    Ok(out)
}

/// `update`: an empty patch answers with the row unchanged.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<PipelineStage>> {
    if values.is_empty() {
        return find_by_id(conn, workspace_id, Some(pipeline_id), id);
    }
    let mut set_values = values.clone();
    if !set_values.iter().any(|(c, _)| *c == "updatedAt") {
        set_values.push(("updatedAt", Value::Integer(now_ms())));
    }
    let mut sql = Sql::raw("UPDATE \"PipelineStage\" SET ");
    sql.append(patch::set_clause(PipelineStage::COLUMNS, &set_values)?);
    sql.append(Sql::new(
        format!(
            " WHERE \"workspaceId\" = ? AND \"pipelineId\" = ? AND \"id\" = ? RETURNING {}",
            PipelineStage::columns_sql(None)
        ),
        vec![
            Value::Integer(workspace_id),
            Value::Integer(pipeline_id),
            Value::Integer(id),
        ],
    ));
    sql.model::<PipelineStage>(conn)
}

/// `releaseBulkClaim`: clears the claim only while it is still `bulk_job_id`.
pub fn release_bulk_claim(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    bulk_job_id: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE \"PipelineStage\" SET \"bulkJobId\" = NULL, \"updatedAt\" = ?5 WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"id\" = ?3 AND \"bulkJobId\" = ?4",
        (workspace_id, pipeline_id, id, bulk_job_id, now_ms()),
    )? > 0)
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, pipeline_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineStage\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"id\" = ?3",
        (workspace_id, pipeline_id, id),
    )? > 0)
}

/// `nextPosition`: `max(position) + 1`, or 0.
pub fn next_position(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<i64> {
    let max: Option<i64> = conn.query_row(
        "SELECT max(\"position\") FROM \"PipelineStage\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2",
        (workspace_id, pipeline_id),
        |row| row.get(0),
    )?;
    Ok(max.unwrap_or(-1) + 1)
}

/// `reorder`: `position = index` for the given ids in one statement.
pub fn reorder(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    stage_ids: &[i64],
) -> Result<()> {
    if stage_ids.is_empty() {
        return Ok(());
    }
    let mut sql = Sql::raw("UPDATE \"PipelineStage\" SET \"position\" = case \"id\"");
    for (index, stage_id) in stage_ids.iter().enumerate() {
        sql.push(" when ");
        sql.push_bind(*stage_id);
        sql.push(" then ");
        sql.push_bind(i64::try_from(index).unwrap_or(i64::MAX));
    }
    sql.push(" end, \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE \"workspaceId\" = ");
    sql.push_bind(workspace_id);
    sql.push(" AND \"pipelineId\" = ");
    sql.push_bind(pipeline_id);
    sql.push(" AND \"id\" IN (SELECT value FROM json_each(");
    sql.push_bind(id_list(stage_ids));
    sql.push("))");
    sql.execute(conn)?;
    Ok(())
}
