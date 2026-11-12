//! Port of `repositories/pipeline/pipeline.ts` (`pipelineRepository`,
//! `acquirePipelineAdvisoryLock`).
//!
//! Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`), except `stripFieldKeyBatch`
//! (maintenance keeps it). Row locks (`FOR SHARE`) and the advisory lock are plain reads /
//! no-ops under the one SQLite writer.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Pipeline, PipelineField, PipelineMember};
use super::patch::{self, Patch};
use super::pipeline_item_cursor::bind1;
use super::sql::{Sql, bool_val, opt_int};
use super::utils::escape_like_pattern;
use super::{Error, Json, Model, Result, find_many, find_one, id_list, new_id, now_ms, returning};

/// `PipelineInsert`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineInsert {
    pub workspace_id: i64,
    pub name: String,
    pub r#type: String,
    pub reference_name: String,
    pub currency: Option<String>,
    pub settings: Json,
    pub created_by_id: Option<i64>,
}

/// `PipelineFieldInsert`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineFieldInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub name: String,
    pub reference_name: String,
    pub r#type: String,
    pub required: bool,
    pub position: i64,
}

/// `PipelineMemberValues`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineMemberValues {
    pub can_manage_settings: bool,
    pub can_view: bool,
    pub can_edit: bool,
    pub can_delete: bool,
}

/// One row of `insertMembers`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineMemberInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub workspace_member_id: i64,
    pub values: PipelineMemberValues,
}

/// `{ id, referenceName }` of `findByReferenceNames`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReferenceName {
    pub id: i64,
    pub reference_name: String,
}

/// One row of `countItemsByPipeline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineItemCount {
    pub pipeline_id: i64,
    pub items_count: i64,
}

/// The column of `listNamesStartingWith`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineNameColumn {
    Name,
    ReferenceName,
}

/// One row of `listMemberAccess`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineMemberAccess {
    pub pipeline_id: i64,
    pub can_view: bool,
    pub can_edit: bool,
    pub can_delete: bool,
    pub can_manage_settings: bool,
}

/// Input of `stripFieldKeyBatch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StripFieldKeyInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub field_id: String,
    pub limit: i64,
    pub after_id: Option<i64>,
}

/// Result of `stripFieldKeyBatch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripFieldKeyBatch {
    pub scanned: usize,
    pub stripped: usize,
    pub last_id: Option<i64>,
}

/// `acquirePipelineAdvisoryLock`: a no-op under the one SQLite writer (the write job
/// already serialises every critical section).
pub fn acquire_pipeline_advisory_lock(_conn: &Connection, _key: &str) -> Result<()> {
    todo!("P3.3")
}

/// `findById`.
pub fn find_by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Pipeline>> {
    todo!("P3.3")
}

/// `findByReferenceName`.
pub fn find_by_reference_name(
    conn: &Connection,
    workspace_id: i64,
    reference_name: &str,
) -> Result<Option<Pipeline>> {
    todo!("P3.3")
}

/// `findByReferenceNames`: the workspace's pipelines with any of these reference names.
pub fn find_by_reference_names(
    conn: &Connection,
    workspace_id: i64,
    reference_names: &[String],
) -> Result<Vec<PipelineReferenceName>> {
    todo!("P3.3")
}

/// `findByName`.
pub fn find_by_name(conn: &Connection, workspace_id: i64, name: &str) -> Result<Option<Pipeline>> {
    todo!("P3.3")
}

/// `listByWorkspace`: every pipeline of a workspace, oldest first.
pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<Pipeline>> {
    todo!("P3.3")
}

/// `listIdsPage`: the workspace's pipeline ids in id order after `after_id`.
pub fn list_ids_page(
    conn: &Connection,
    workspace_id: i64,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `countItemsByPipeline`: item count per pipeline; pipelines without items are absent.
pub fn count_items_by_pipeline(conn: &Connection, workspace_id: i64) -> Result<Vec<PipelineItemCount>> {
    todo!("P3.3")
}

/// `listNamesStartingWith`: names / reference names that start with `prefix`
/// (case-sensitive `LIKE`).
pub fn list_names_starting_with(
    conn: &Connection,
    workspace_id: i64,
    column: PipelineNameColumn,
    prefix: &str,
) -> Result<Vec<String>> {
    todo!("P3.3")
}

/// `insert`.
pub fn insert(conn: &Connection, values: &PipelineInsert) -> Result<Pipeline> {
    todo!("P3.3")
}

fn with_updated_at(values: &Patch) -> Patch {
    let mut out = values.clone();
    if !out.iter().any(|(c, _)| *c == "updatedAt") {
        out.push(("updatedAt", Value::Integer(now_ms())));
    }
    out
}

fn update_returning<M: Model>(conn: &Connection, values: &Patch, where_sql: Sql) -> Result<Option<M>> {
    let set = patch::set_clause(M::COLUMNS, &with_updated_at(values))?;
    let mut sql = Sql::raw(format!("UPDATE \"{}\" SET ", M::TABLE));
    sql.append(set);
    sql.push(" WHERE ");
    sql.append(where_sql);
    sql.push(&format!(" RETURNING {}", M::columns_sql(None)));
    Ok(sql.model::<M>(conn)?)
}

/// `update`: an empty patch answers with the row unchanged.
pub fn update(conn: &Connection, workspace_id: i64, id: i64, values: &Patch) -> Result<Option<Pipeline>> {
    todo!("P3.3")
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `nextItemNumber`: the next item number of the workspace (an upsert on the counter row;
/// run it in the write job that inserts the item, so a rollback gives the number back).
pub fn next_item_number(conn: &Connection, workspace_id: i64) -> Result<i64> {
    todo!("P3.3")
}

/// `listFields`: in position order (`id` breaks ties).
pub fn list_fields(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<Vec<PipelineField>> {
    todo!("P3.3")
}

/// `listFieldsByPipelineIds`.
pub fn list_fields_by_pipeline_ids(
    conn: &Connection,
    workspace_id: i64,
    pipeline_ids: &[i64],
) -> Result<Vec<PipelineField>> {
    todo!("P3.3")
}

fn insert_field_row(conn: &Connection, values: &PipelineFieldInsert) -> Result<Vec<PipelineField>> {
    returning::<PipelineField>(
        conn,
        "INSERT INTO \"PipelineField\" (\"id\", \"workspaceId\", \"pipelineId\", \"name\", \"referenceName\", \"type\", \"required\", \"position\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        &[
            &new_id(),
            &values.workspace_id,
            &values.pipeline_id,
            &values.name,
            &values.reference_name,
            &values.r#type,
            &values.required,
            &values.position,
        ],
    )
}

/// `insertField`.
pub fn insert_field(conn: &Connection, values: &PipelineFieldInsert) -> Result<PipelineField> {
    todo!("P3.3")
}

/// `insertFields`.
pub fn insert_fields(conn: &Connection, rows: &[PipelineFieldInsert]) -> Result<()> {
    todo!("P3.3")
}

/// `updateField`: an empty patch answers with the row unchanged.
pub fn update_field(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<PipelineField>> {
    todo!("P3.3")
}

/// `lockFieldsShared`: the named field ids that still exist, in id order (the share lock
/// is the write job itself).
pub fn lock_fields_shared(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `deleteField`.
pub fn delete_field(conn: &Connection, workspace_id: i64, pipeline_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `stripFieldKeyBatch`: removes a deleted field's key from the next `limit` items (by id,
/// after `after_id`) that still hold it. `updatedAt` is kept.
pub fn strip_field_key_batch(conn: &Connection, input: &StripFieldKeyInput) -> Result<StripFieldKeyBatch> {
    todo!("P3.3")
}

/// `listMembers`.
pub fn list_members(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<Vec<PipelineMember>> {
    todo!("P3.3")
}

/// `findMember`.
pub fn find_member(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
) -> Result<Option<PipelineMember>> {
    todo!("P3.3")
}

/// `listMemberAccess`: a member's ACL rows across the workspace.
pub fn list_member_access(
    conn: &Connection,
    workspace_id: i64,
    workspace_member_id: i64,
) -> Result<Vec<PipelineMemberAccess>> {
    todo!("P3.3")
}

/// `upsertMember`: insert, or on `(pipelineId, workspaceMemberId)` overwrite the flags.
pub fn upsert_member(conn: &Connection, input: &PipelineMemberInsert) -> Result<PipelineMember> {
    todo!("P3.3")
}

/// `deleteMember`.
pub fn delete_member(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
) -> Result<bool> {
    todo!("P3.3")
}

/// `insertMembers`.
pub fn insert_members(conn: &Connection, rows: &[PipelineMemberInsert]) -> Result<()> {
    todo!("P3.3")
}
