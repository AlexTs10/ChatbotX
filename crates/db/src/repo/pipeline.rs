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
    Ok(())
}

/// `findById`.
pub fn find_by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Pipeline>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )
}

/// `findByReferenceName`.
pub fn find_by_reference_name(
    conn: &Connection,
    workspace_id: i64,
    reference_name: &str,
) -> Result<Option<Pipeline>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"referenceName\" = ?2",
        &[&workspace_id, &reference_name],
    )
}

/// `findByReferenceNames`: the workspace's pipelines with any of these reference names.
pub fn find_by_reference_names(
    conn: &Connection,
    workspace_id: i64,
    reference_names: &[String],
) -> Result<Vec<PipelineReferenceName>> {
    if reference_names.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"referenceName\" FROM \"Pipeline\" WHERE \"workspaceId\" = ? AND \"referenceName\" IN (SELECT value FROM json_each(?))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(super::text_list(reference_names)),
        ],
    )
    .query_map(conn, |row| {
        Ok(PipelineReferenceName {
            id: row.get(0)?,
            reference_name: row.get(1)?,
        })
    })
}

/// `findByName`.
pub fn find_by_name(conn: &Connection, workspace_id: i64, name: &str) -> Result<Option<Pipeline>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"name\" = ?2",
        &[&workspace_id, &name],
    )
}

/// `listByWorkspace`: every pipeline of a workspace, oldest first.
pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<Pipeline>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" ASC",
        &[&workspace_id],
    )
}

/// `listIdsPage`: the workspace's pipeline ids in id order after `after_id`.
pub fn list_ids_page(
    conn: &Connection,
    workspace_id: i64,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    Sql::new(
        "SELECT \"id\" FROM \"Pipeline\" WHERE \"workspaceId\" = ? AND (? IS NULL OR \"id\" > ?) ORDER BY \"id\" ASC LIMIT ?",
        vec![
            Value::Integer(workspace_id),
            opt_int(after_id),
            opt_int(after_id),
            Value::Integer(limit),
        ],
    )
    .query_map(conn, |row| row.get(0))
}

/// `countItemsByPipeline`: item count per pipeline; pipelines without items are absent.
pub fn count_items_by_pipeline(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<PipelineItemCount>> {
    bind1(
        "SELECT \"pipelineId\", count(*) FROM \"PipelineItem\" WHERE \"workspaceId\" = ? GROUP BY \"pipelineId\"",
        workspace_id,
    )
    .query_map(conn, |row| {
        Ok(PipelineItemCount {
            pipeline_id: row.get(0)?,
            items_count: row.get(1)?,
        })
    })
}

/// `listNamesStartingWith`: names / reference names that start with `prefix`
/// (case-sensitive `LIKE`).
pub fn list_names_starting_with(
    conn: &Connection,
    workspace_id: i64,
    column: PipelineNameColumn,
    prefix: &str,
) -> Result<Vec<String>> {
    let column = match column {
        PipelineNameColumn::Name => "\"name\"",
        PipelineNameColumn::ReferenceName => "\"referenceName\"",
    };
    Sql::new(
        format!(
            "SELECT {column} FROM \"Pipeline\" WHERE \"workspaceId\" = ? AND like_cs({column}, ?)"
        ),
        vec![
            Value::Integer(workspace_id),
            Value::Text(format!("{}%", escape_like_pattern(prefix))),
        ],
    )
    .query_map(conn, |row| row.get(0))
}

/// `insert`.
pub fn insert(conn: &Connection, values: &PipelineInsert) -> Result<Pipeline> {
    let sql = "INSERT INTO \"Pipeline\" (\"id\", \"workspaceId\", \"name\", \"type\", \"referenceName\", \"currency\", \"settings\", \"createdById\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
    returning::<Pipeline>(
        conn,
        sql,
        &[
            &new_id(),
            &values.workspace_id,
            &values.name,
            &values.r#type,
            &values.reference_name,
            &values.currency,
            &values.settings,
            &values.created_by_id,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| Error::msg("Failed to insert pipeline"))
}

fn with_updated_at(values: &Patch) -> Patch {
    let mut out = values.clone();
    if !out.iter().any(|(c, _)| *c == "updatedAt") {
        out.push(("updatedAt", Value::Integer(now_ms())));
    }
    out
}

fn update_returning<M: Model>(
    conn: &Connection,
    values: &Patch,
    where_sql: Sql,
) -> Result<Option<M>> {
    let set = patch::set_clause(M::COLUMNS, &with_updated_at(values))?;
    let mut sql = Sql::raw(format!("UPDATE \"{}\" SET ", M::TABLE));
    sql.append(set);
    sql.push(" WHERE ");
    sql.append(where_sql);
    sql.push(&format!(" RETURNING {}", M::columns_sql(None)));
    sql.model::<M>(conn)
}

/// `update`: an empty patch answers with the row unchanged.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<Pipeline>> {
    if values.is_empty() {
        return find_by_id(conn, workspace_id, id);
    }
    update_returning::<Pipeline>(
        conn,
        values,
        Sql::new(
            "\"workspaceId\" = ? AND \"id\" = ?",
            vec![Value::Integer(workspace_id), Value::Integer(id)],
        ),
    )
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"Pipeline\" WHERE \"workspaceId\" = ?1 AND \"id\" = ?2",
        (workspace_id, id),
    )? > 0)
}

/// `nextItemNumber`: the next item number of the workspace (an upsert on the counter row;
/// run it in the write job that inserts the item, so a rollback gives the number back).
pub fn next_item_number(conn: &Connection, workspace_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "INSERT INTO \"PipelineItemCounter\" (\"workspaceId\", \"lastNumber\") VALUES (?1, 1) ON CONFLICT (\"workspaceId\") DO UPDATE SET \"lastNumber\" = \"PipelineItemCounter\".\"lastNumber\" + 1 RETURNING \"lastNumber\"",
        [workspace_id],
        |row| row.get(0),
    )?)
}

/// `listFields`: in position order (`id` breaks ties).
pub fn list_fields(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
) -> Result<Vec<PipelineField>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 ORDER BY \"position\" ASC, \"id\" ASC",
        &[&workspace_id, &pipeline_id],
    )
}

/// `listFieldsByPipelineIds`.
pub fn list_fields_by_pipeline_ids(
    conn: &Connection,
    workspace_id: i64,
    pipeline_ids: &[i64],
) -> Result<Vec<PipelineField>> {
    if pipeline_ids.is_empty() {
        return Ok(Vec::new());
    }
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"pipelineId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"position\" ASC, \"id\" ASC",
        &[&workspace_id, &id_list(pipeline_ids)],
    )
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
    insert_field_row(conn, values)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::msg("Failed to insert pipeline field"))
}

/// `insertFields`.
pub fn insert_fields(conn: &Connection, rows: &[PipelineFieldInsert]) -> Result<()> {
    for values in rows {
        insert_field_row(conn, values)?;
    }
    Ok(())
}

/// `updateField`: an empty patch answers with the row unchanged.
pub fn update_field(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<PipelineField>> {
    if values.is_empty() {
        return find_one(
            conn,
            "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"id\" = ?3",
            &[&workspace_id, &pipeline_id, &id],
        );
    }
    update_returning::<PipelineField>(
        conn,
        values,
        Sql::new(
            "\"workspaceId\" = ? AND \"pipelineId\" = ? AND \"id\" = ?",
            vec![
                Value::Integer(workspace_id),
                Value::Integer(pipeline_id),
                Value::Integer(id),
            ],
        ),
    )
}

/// `lockFieldsShared`: the named field ids that still exist, in id order (the share lock
/// is the write job itself).
pub fn lock_fields_shared(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\" FROM \"PipelineField\" WHERE \"workspaceId\" = ? AND \"pipelineId\" = ? AND \"id\" IN (SELECT value FROM json_each(?)) ORDER BY \"id\"",
        vec![
            Value::Integer(workspace_id),
            Value::Integer(pipeline_id),
            Value::Text(id_list(ids)),
        ],
    )
    .query_map(conn, |row| row.get(0))
}

/// `deleteField`.
pub fn delete_field(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    id: i64,
) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineField\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"id\" = ?3",
        (workspace_id, pipeline_id, id),
    )? > 0)
}

/// `stripFieldKeyBatch`: removes a deleted field's key from the next `limit` items (by id,
/// after `after_id`) that still hold it. `updatedAt` is kept.
pub fn strip_field_key_batch(
    conn: &Connection,
    input: &StripFieldKeyInput,
) -> Result<StripFieldKeyBatch> {
    let Some(path) = super::pipeline_field_condition::field_key_path(&input.field_id) else {
        return Ok(StripFieldKeyBatch {
            scanned: 0,
            stripped: 0,
            last_id: input.after_id,
        });
    };
    let has_key = "json_type(\"fieldValues\", ?) IS NOT NULL";
    let candidates: Vec<i64> = Sql::new(
        format!(
            "SELECT \"id\" FROM \"PipelineItem\" WHERE \"workspaceId\" = ? AND \"pipelineId\" = ? AND {has_key} AND (? IS NULL OR \"id\" > ?) ORDER BY \"id\" LIMIT ?"
        ),
        vec![
            Value::Integer(input.workspace_id),
            Value::Integer(input.pipeline_id),
            Value::Text(path.clone()),
            opt_int(input.after_id),
            opt_int(input.after_id),
            Value::Integer(input.limit),
        ],
    )
    .query_map(conn, |row| row.get(0))?;
    if candidates.is_empty() {
        return Ok(StripFieldKeyBatch {
            scanned: 0,
            stripped: 0,
            last_id: input.after_id,
        });
    }
    let stripped = Sql::new(
        format!(
            "UPDATE \"PipelineItem\" SET \"fieldValues\" = json_remove(\"fieldValues\", ?) WHERE \"id\" IN (SELECT value FROM json_each(?)) AND \"workspaceId\" = ? AND \"pipelineId\" = ? AND {has_key}"
        ),
        vec![
            Value::Text(path.clone()),
            Value::Text(id_list(&candidates)),
            Value::Integer(input.workspace_id),
            Value::Integer(input.pipeline_id),
            Value::Text(path),
        ],
    )
    .execute(conn)?;
    Ok(StripFieldKeyBatch {
        scanned: candidates.len(),
        stripped,
        last_id: candidates.last().copied(),
    })
}

/// `listMembers`.
pub fn list_members(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
) -> Result<Vec<PipelineMember>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2",
        &[&workspace_id, &pipeline_id],
    )
}

/// `findMember`.
pub fn find_member(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
) -> Result<Option<PipelineMember>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"workspaceMemberId\" = ?3",
        &[&workspace_id, &pipeline_id, &workspace_member_id],
    )
}

/// `listMemberAccess`: a member's ACL rows across the workspace.
pub fn list_member_access(
    conn: &Connection,
    workspace_id: i64,
    workspace_member_id: i64,
) -> Result<Vec<PipelineMemberAccess>> {
    Sql::new(
        "SELECT \"pipelineId\", \"canView\", \"canEdit\", \"canDelete\", \"canManageSettings\" FROM \"PipelineMember\" WHERE \"workspaceId\" = ? AND \"workspaceMemberId\" = ?",
        vec![Value::Integer(workspace_id), Value::Integer(workspace_member_id)],
    )
    .query_map(conn, |row| {
        Ok(PipelineMemberAccess {
            pipeline_id: row.get(0)?,
            can_view: row.get(1)?,
            can_edit: row.get(2)?,
            can_delete: row.get(3)?,
            can_manage_settings: row.get(4)?,
        })
    })
}

/// `upsertMember`: insert, or on `(pipelineId, workspaceMemberId)` overwrite the flags.
pub fn upsert_member(conn: &Connection, input: &PipelineMemberInsert) -> Result<PipelineMember> {
    let v = input.values;
    Sql::new(
        format!(
            "INSERT INTO \"PipelineMember\" (\"id\", \"workspaceId\", \"pipelineId\", \"workspaceMemberId\", \"canManageSettings\", \"canView\", \"canEdit\", \"canDelete\") VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (\"pipelineId\", \"workspaceMemberId\") DO UPDATE SET \"canManageSettings\" = excluded.\"canManageSettings\", \"canView\" = excluded.\"canView\", \"canEdit\" = excluded.\"canEdit\", \"canDelete\" = excluded.\"canDelete\", \"updatedAt\" = ? RETURNING {}",
            PipelineMember::columns_sql(None)
        ),
        vec![
            Value::Integer(new_id()),
            Value::Integer(input.workspace_id),
            Value::Integer(input.pipeline_id),
            Value::Integer(input.workspace_member_id),
            bool_val(v.can_manage_settings),
            bool_val(v.can_view),
            bool_val(v.can_edit),
            bool_val(v.can_delete),
            Value::Integer(now_ms()),
        ],
    )
    .model::<PipelineMember>(conn)?
    .ok_or_else(|| Error::msg("Failed to upsert pipeline member"))
}

/// `deleteMember`.
pub fn delete_member(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineMember\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"workspaceMemberId\" = ?3",
        (workspace_id, pipeline_id, workspace_member_id),
    )? > 0)
}

/// `insertMembers`.
pub fn insert_members(conn: &Connection, rows: &[PipelineMemberInsert]) -> Result<()> {
    for row in rows {
        let v = row.values;
        conn.execute(
            "INSERT INTO \"PipelineMember\" (\"id\", \"workspaceId\", \"pipelineId\", \"workspaceMemberId\", \"canManageSettings\", \"canView\", \"canEdit\", \"canDelete\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                new_id(),
                row.workspace_id,
                row.pipeline_id,
                row.workspace_member_id,
                v.can_manage_settings,
                v.can_view,
                v.can_edit,
                v.can_delete,
            ),
        )?;
    }
    Ok(())
}
