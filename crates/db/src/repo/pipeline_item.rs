//! Port of `repositories/pipeline/item.ts` (`pipelineItemRepository`).
//!
//! SQLite form: the row lock of `lockForUpdate` and the lock waits of the batch
//! statements do not exist (one writer); each batch still repeats its scope in the outer
//! WHERE. The board first page (`JOIN LATERAL` per stage) is one page query per stage of
//! the pipeline, in stage id order. Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`).

use std::collections::HashSet;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{PipelineItem, PipelineItemAssignee};
use super::patch::{self, Patch};
use super::pipeline_item_cursor::bind1;
use super::pipeline_item_cursor::{
    PipelineItemSort, clamp_page_limit, cursor_for, decode_pipeline_item_cursor,
    pipeline_item_after_cursor, pipeline_item_order_by, pipeline_item_sort_value_sql,
    sort_value_text,
};
use super::pipeline_item_filter::{PipelineItemFilter, build_pipeline_item_where};
use super::sql::{Sql, opt_int};
use super::{Error, Json, Model, Result, find_one, id_list, new_id, now_ms, returning};

/// `PipelineItemInsert`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineItemInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub stage_id: i64,
    pub contact_id: i64,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    /// Money x 10,000.
    pub value: Option<i64>,
    pub status: String,
    pub priority: String,
    pub due_date: Option<i64>,
    pub field_values: Json,
    pub created_by_id: Option<i64>,
    pub closed_by_id: Option<i64>,
    pub stage_entered_at: i64,
    pub status_changed_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub last_comment_at: Option<i64>,
    pub idempotency_key: Option<String>,
}

/// Result of `insertIdempotent`.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertedItem {
    pub row: PipelineItem,
    pub inserted: bool,
}

/// `PipelineItemPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineItemPage {
    pub items: Vec<PipelineItem>,
    pub next_cursor: Option<String>,
}

/// `PipelineBoardStagePage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineBoardStagePage {
    pub stage_id: i64,
    pub items: Vec<PipelineItem>,
    /// Resumes right after the last item of `items`; `None` when the stage fits.
    pub next_cursor: Option<String>,
}

/// `PipelineStageTotals` (`total_value` is money x 10,000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStageTotals {
    pub stage_id: i64,
    pub count: i64,
    pub total_value: i64,
}

/// Input of `listStagePage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListStagePageInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    /// `None` pages across every stage (public list); set for a board column.
    pub stage_id: Option<i64>,
    pub filter: PipelineItemFilter,
    pub sort: PipelineItemSort,
    pub limit: i64,
    pub cursor: Option<String>,
}

/// Input of `listBoardFirstPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardFirstPageInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub filter: PipelineItemFilter,
    pub sort: PipelineItemSort,
    pub per_stage_limit: i64,
}

/// Input of `moveStageItemsBatch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveStageItemsInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub from_stage_id: i64,
    pub to_stage_id: i64,
    pub limit: i64,
    pub now: i64,
}

/// `findById` (`pipeline_id` narrows when given).
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    pipeline_id: Option<i64>,
) -> Result<Option<PipelineItem>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2 AND (?3 IS NULL OR \"pipelineId\" = ?3)",
        &[&workspace_id, &id, &pipeline_id],
    )
}

/// `findByIdempotencyKey`.
pub fn find_by_idempotency_key(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    idempotency_key: &str,
) -> Result<Option<PipelineItem>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"idempotencyKey\" = ?3",
        &[&workspace_id, &pipeline_id, &idempotency_key],
    )
}

/// `lockForUpdate`: the item row (the write job is the lock).
pub fn lock_for_update(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PipelineItem>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )
}

/// `insertIdempotent`: inserts the item, or returns the one an earlier call with the same
/// idempotency key stored. Only that key's conflict is absorbed.
pub fn insert_idempotent(conn: &Connection, values: &PipelineItemInsert) -> Result<InsertedItem> {
    let conflict = if values.idempotency_key.is_some() {
        " ON CONFLICT (\"workspaceId\", \"pipelineId\", \"idempotencyKey\") WHERE \"idempotencyKey\" IS NOT NULL DO NOTHING"
    } else {
        ""
    };
    let sql = format!(
        "INSERT INTO \"PipelineItem\" (\"id\", \"workspaceId\", \"pipelineId\", \"stageId\", \"contactId\", \"number\", \"title\", \"description\", \"value\", \"status\", \"priority\", \"dueDate\", \"fieldValues\", \"createdById\", \"closedById\", \"stageEnteredAt\", \"statusChangedAt\", \"closedAt\", \"lastCommentAt\", \"idempotencyKey\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20){conflict}"
    );
    let row = returning::<PipelineItem>(
        conn,
        &sql,
        &[
            &new_id(),
            &values.workspace_id,
            &values.pipeline_id,
            &values.stage_id,
            &values.contact_id,
            &values.number,
            &values.title,
            &values.description,
            &values.value,
            &values.status,
            &values.priority,
            &values.due_date,
            &values.field_values,
            &values.created_by_id,
            &values.closed_by_id,
            &values.stage_entered_at,
            &values.status_changed_at,
            &values.closed_at,
            &values.last_comment_at,
            &values.idempotency_key,
        ],
    )?
    .into_iter()
    .next();
    if let Some(row) = row {
        return Ok(InsertedItem {
            row,
            inserted: true,
        });
    }
    let Some(key) = values.idempotency_key.as_deref() else {
        return Err(Error::msg("Failed to insert pipeline item"));
    };
    let existing = find_by_idempotency_key(conn, values.workspace_id, values.pipeline_id, key)?
        .ok_or_else(|| Error::msg("Pipeline item idempotency conflict without a row"))?;
    Ok(InsertedItem {
        row: existing,
        inserted: false,
    })
}

fn update_where(
    conn: &Connection,
    set: &Patch,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PipelineItem>> {
    let mut sql = Sql::raw("UPDATE \"PipelineItem\" SET ");
    sql.append(patch::set_clause(PipelineItem::COLUMNS, set)?);
    sql.append(Sql::new(
        format!(
            " WHERE \"workspaceId\" = ? AND \"id\" = ? RETURNING {}",
            PipelineItem::columns_sql(None)
        ),
        vec![Value::Integer(workspace_id), Value::Integer(id)],
    ));
    sql.model::<PipelineItem>(conn)
}

/// `update`: sets the patch columns (and `updatedAt`).
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<PipelineItem>> {
    let mut set = values.clone();
    if !set.iter().any(|(c, _)| *c == "updatedAt") {
        set.push(("updatedAt", Value::Integer(now_ms())));
    }
    update_where(conn, &set, workspace_id, id)
}

/// `touch`: marks the item as changed without touching other columns.
pub fn touch(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineItem>> {
    update_where(
        conn,
        &vec![("updatedAt", Value::Integer(now_ms()))],
        workspace_id,
        id,
    )
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineItem\" WHERE \"workspaceId\" = ?1 AND \"id\" = ?2",
        (workspace_id, id),
    )? > 0)
}

/// `findLatestOpenForContact`: the contact's most recently created open item.
pub fn find_latest_open_for_contact(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    contact_id: i64,
) -> Result<Option<PipelineItem>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"contactId\" = ?3 AND \"status\" = 'open' ORDER BY \"createdAt\" DESC, \"id\" DESC",
        &[&workspace_id, &pipeline_id, &contact_id],
    )
}

/// `countOpenForContact`: a contact's open items in a pipeline, optionally leaving one out.
pub fn count_open_for_contact(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    contact_id: i64,
    except_item_id: Option<i64>,
) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"PipelineItem\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"contactId\" = ?3 AND \"status\" = 'open' AND (?4 IS NULL OR \"id\" <> ?4)",
        (workspace_id, pipeline_id, contact_id, except_item_id),
        |row| row.get(0),
    )?)
}

/// `listForContact`: every pipeline's items of one contact, newest first.
pub fn list_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    pipeline_ids: Option<&[i64]>,
    limit: i64,
) -> Result<Vec<PipelineItem>> {
    Sql::new(
        format!(
            "{} WHERE \"workspaceId\" = ? AND \"contactId\" = ? AND (? IS NULL OR \"pipelineId\" IN (SELECT value FROM json_each(?))) ORDER BY \"createdAt\" DESC LIMIT ?",
            PipelineItem::select_sql()
        ),
        {
            let ids = pipeline_ids.map_or(Value::Null, |ids| Value::Text(id_list(ids)));
            vec![
                Value::Integer(workspace_id),
                Value::Integer(contact_id),
                ids.clone(),
                ids,
                Value::Integer(limit),
            ]
        },
    )
    .models::<PipelineItem>(conn)
}

/// One page query: items matching `where_sql`, sorted, at most `fetch` rows, with the
/// sort value text of each row.
fn page_query(
    conn: &Connection,
    where_sql: Sql,
    sort: PipelineItemSort,
    fetch: i64,
) -> Result<Vec<(PipelineItem, Option<String>)>> {
    let n = PipelineItem::COLUMNS.len();
    let mut sql = Sql::raw(format!(
        "SELECT {}, {} FROM \"PipelineItem\" WHERE ",
        PipelineItem::columns_sql(Some("\"PipelineItem\"")),
        pipeline_item_sort_value_sql(sort)
    ));
    sql.append(where_sql);
    sql.push(&format!(
        " ORDER BY {} LIMIT ",
        pipeline_item_order_by(sort).join(", ")
    ));
    sql.push_bind(fetch);
    sql.query_map(conn, |row| {
        let item = PipelineItem::from_row(row)?;
        let raw: Option<i64> = row.get(n)?;
        Ok((item, sort_value_text(sort.key, raw)))
    })
}

fn cut_page(
    rows: Vec<(PipelineItem, Option<String>)>,
    limit: i64,
    sort: PipelineItemSort,
) -> (Vec<PipelineItem>, Option<String>) {
    let keep = usize::try_from(limit).unwrap_or(0);
    let more = rows.len() > keep;
    let mut rows = rows;
    rows.truncate(keep);
    let next = match rows.last() {
        Some((item, value)) if more => Some(cursor_for(sort, value.as_deref(), item.id)),
        _ => None,
    };
    (rows.into_iter().map(|(item, _)| item).collect(), next)
}

/// `listStagePage`: one keyset page of a stage, or of the whole pipeline when `stage_id`
/// is `None`. A malformed cursor fails with `InvalidPipelineCursorError`.
pub fn list_stage_page(conn: &Connection, input: &ListStagePageInput) -> Result<PipelineItemPage> {
    let limit = clamp_page_limit(input.limit);
    let cursor = match input.cursor.as_deref() {
        Some(raw) => Some(decode_pipeline_item_cursor(raw, input.sort)?),
        None => None,
    };
    let mut filter = input.filter.clone();
    if let Some(stage_id) = input.stage_id {
        filter.stage_ids = vec![stage_id];
    }
    let where_sql = Sql::and([
        Some(build_pipeline_item_where(
            input.workspace_id,
            Some(input.pipeline_id),
            &filter,
        )),
        cursor.map(|c| pipeline_item_after_cursor(input.sort, &c)),
    ]);
    let rows = page_query(conn, where_sql, input.sort, limit + 1)?;
    let (items, next_cursor) = cut_page(rows, limit, input.sort);
    Ok(PipelineItemPage { items, next_cursor })
}

/// `listBoardFirstPage`: the first page of every stage of a pipeline (at most
/// `per_stage_limit` items each); stages with no matching item are absent.
pub fn list_board_first_page(
    conn: &Connection,
    input: &BoardFirstPageInput,
) -> Result<Vec<PipelineBoardStagePage>> {
    let limit = clamp_page_limit(input.per_stage_limit);
    let stage_ids: Vec<i64> = Sql::new(
        "SELECT \"id\" FROM \"PipelineStage\" WHERE \"workspaceId\" = ? AND \"pipelineId\" = ? ORDER BY \"id\"",
        vec![
            Value::Integer(input.workspace_id),
            Value::Integer(input.pipeline_id),
        ],
    )
    .query_map(conn, |row| row.get(0))?;
    let mut out = Vec::new();
    for stage_id in stage_ids {
        let where_sql = Sql::and([
            Some(bind1("\"PipelineItem\".\"stageId\" = ?", stage_id)),
            Some(build_pipeline_item_where(
                input.workspace_id,
                Some(input.pipeline_id),
                &input.filter,
            )),
        ]);
        let rows = page_query(conn, where_sql, input.sort, limit + 1)?;
        if rows.is_empty() {
            continue;
        }
        let (items, next_cursor) = cut_page(rows, limit, input.sort);
        out.push(PipelineBoardStagePage {
            stage_id,
            items,
            next_cursor,
        });
    }
    Ok(out)
}

/// `listStageTotals`: count and summed value per stage.
pub fn list_stage_totals(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    filter: &PipelineItemFilter,
) -> Result<Vec<PipelineStageTotals>> {
    let mut sql = Sql::raw(
        "SELECT \"PipelineItem\".\"stageId\", count(*), coalesce(sum(\"PipelineItem\".\"value\"), 0) FROM \"PipelineItem\" WHERE ",
    );
    sql.append(build_pipeline_item_where(
        workspace_id,
        Some(pipeline_id),
        filter,
    ));
    sql.push(" GROUP BY \"PipelineItem\".\"stageId\"");
    sql.query_map(conn, |row| {
        Ok(PipelineStageTotals {
            stage_id: row.get(0)?,
            count: row.get(1)?,
            total_value: row.get(2)?,
        })
    })
}

/// `countByStage`.
pub fn count_by_stage(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"PipelineItem\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND \"stageId\" = ?3",
        (workspace_id, pipeline_id, stage_id),
        |row| row.get(0),
    )?)
}

const BATCH_SCOPE: &str =
    "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND (?3 IS NULL OR \"stageId\" = ?3)";

fn batch_ids_sql() -> String {
    format!(
        "\"id\" IN (SELECT \"id\" FROM \"PipelineItem\" WHERE {BATCH_SCOPE} LIMIT ?4) AND {BATCH_SCOPE}"
    )
}

/// `moveStageItemsBatch`: moves up to `limit` items of a stage to another; returns them.
pub fn move_stage_items_batch(
    conn: &Connection,
    input: &MoveStageItemsInput,
) -> Result<Vec<PipelineItem>> {
    let sql = format!(
        "UPDATE \"PipelineItem\" SET \"stageId\" = ?5, \"stageEnteredAt\" = ?6, \"updatedAt\" = ?7 WHERE {}",
        batch_ids_sql()
    );
    returning::<PipelineItem>(
        conn,
        &sql,
        &[
            &input.workspace_id,
            &input.pipeline_id,
            &Some(input.from_stage_id),
            &input.limit,
            &input.to_stage_id,
            &input.now,
            &now_ms(),
        ],
    )
}

/// `deleteStageItemsBatch`: deletes up to `limit` items of a stage; returns them.
pub fn delete_stage_items_batch(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    limit: i64,
) -> Result<Vec<PipelineItem>> {
    let sql = format!("DELETE FROM \"PipelineItem\" WHERE {}", batch_ids_sql());
    returning::<PipelineItem>(
        conn,
        &sql,
        &[&workspace_id, &pipeline_id, &Some(stage_id), &limit],
    )
}

/// `deletePipelineItemsBatch`: deletes up to `limit` items of a pipeline; returns how many.
pub fn delete_pipeline_items_batch(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    limit: i64,
) -> Result<usize> {
    let sql = format!("DELETE FROM \"PipelineItem\" WHERE {}", batch_ids_sql());
    Ok(conn.execute(&sql, (workspace_id, pipeline_id, None::<i64>, limit))?)
}

/// `listAssigneesForItems`: oldest first.
pub fn list_assignees_for_items(
    conn: &Connection,
    workspace_id: i64,
    item_ids: &[i64],
) -> Result<Vec<PipelineItemAssignee>> {
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"itemId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"createdAt\", \"id\"",
        &[&workspace_id, &id_list(item_ids)],
    )
}

/// `replaceAssignees`: makes the assignee set exactly the given targets; rows that stay
/// are untouched (their `createdAt` keeps ordering the primary assignee).
pub fn replace_assignees(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    workspace_member_ids: &[i64],
    inbox_team_ids: &[i64],
) -> Result<()> {
    let current: Vec<PipelineItemAssignee> = super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2",
        &[&workspace_id, &item_id],
    )?;
    let wanted_members: HashSet<i64> = workspace_member_ids.iter().copied().collect();
    let wanted_teams: HashSet<i64> = inbox_team_ids.iter().copied().collect();
    let mut kept_members = HashSet::new();
    let mut kept_teams = HashSet::new();
    let mut removed = Vec::new();
    for row in &current {
        match (row.workspace_member_id, row.inbox_team_id) {
            (Some(m), _) if wanted_members.contains(&m) => {
                kept_members.insert(m);
            }
            (_, Some(t)) if wanted_teams.contains(&t) => {
                kept_teams.insert(t);
            }
            _ => removed.push(row.id),
        }
    }
    if !removed.is_empty() {
        conn.execute(
            "DELETE FROM \"PipelineItemAssignee\" WHERE \"id\" IN (SELECT value FROM json_each(?1))",
            [id_list(&removed)],
        )?;
    }
    let added = workspace_member_ids
        .iter()
        .filter(|id| !kept_members.contains(*id))
        .map(|id| (Some(*id), None))
        .chain(
            inbox_team_ids
                .iter()
                .filter(|id| !kept_teams.contains(*id))
                .map(|id| (None, Some(*id))),
        );
    for (member, team) in added {
        conn.execute(
            "INSERT INTO \"PipelineItemAssignee\" (\"id\", \"workspaceId\", \"itemId\", \"workspaceMemberId\", \"inboxTeamId\") VALUES (?1, ?2, ?3, ?4, ?5)",
            (new_id(), workspace_id, item_id, opt_int(member), opt_int(team)),
        )?;
    }
    Ok(())
}
