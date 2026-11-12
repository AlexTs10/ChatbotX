//! Port of `repositories/pipeline/activity.ts` (`pipelineActivityRepository`). Money
//! sums (`toValue`) are x 10,000 integers.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::PipelineItemActivity;
use super::pipeline_item_cursor::NewestFirstKeyset;
use super::pipeline_item_cursor::bind1;
use super::pipeline_item_filter::PIPELINE_AUTOMATION_ACTOR_KINDS;
use super::queries::date_bucket::{Granularity, zoned_bucket_key};
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
    Ok(conn.execute(
        "DELETE FROM \"PipelineItemActivity\" WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT \"id\" FROM \"PipelineItemActivity\" WHERE \"workspaceId\" = ?1 AND \"pipelineId\" = ?2 LIMIT ?3)",
        (workspace_id, pipeline_id, limit),
    )?)
}

/// `scrubContactTitles`: clears the item titles kept in a deleted contact's history.
pub fn scrub_contact_titles(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<()> {
    if contact_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE \"PipelineItemActivity\" SET \"itemTitle\" = '', \"fromTitle\" = NULL, \"toTitle\" = NULL WHERE \"workspaceId\" = ?1 AND \"contactId\" IN (SELECT value FROM json_each(?2))",
        (workspace_id, super::id_list(contact_ids)),
    )?;
    Ok(())
}

/// `insertMany`.
pub fn insert_many(conn: &Connection, rows: &[PipelineActivityInsert]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"PipelineItemActivity\" (\"id\", \"workspaceId\", \"pipelineId\", \"itemId\", \"itemNumber\", \"itemTitle\", \"contactId\", \"kind\", \"actorKind\", \"actorUserId\", \"actorSourceId\", \"fromStageId\", \"toStageId\", \"fromStatus\", \"toStatus\", \"fromValue\", \"toValue\", \"fromPriority\", \"toPriority\", \"fromDueDate\", \"toDueDate\", \"fromTitle\", \"toTitle\", \"fromPipelineId\", \"version\", \"occurredAt\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
    )?;
    for r in rows {
        let version = if r.version == 0 { 1 } else { r.version };
        stmt.execute(rusqlite::params![
            new_id(),
            r.workspace_id,
            r.pipeline_id,
            r.item_id,
            r.item_number,
            r.item_title,
            r.contact_id,
            r.kind,
            r.actor_kind,
            r.actor_user_id,
            r.actor_source_id,
            r.from_stage_id,
            r.to_stage_id,
            r.from_status,
            r.to_status,
            r.from_value,
            r.to_value,
            r.from_priority,
            r.to_priority,
            r.from_due_date,
            r.to_due_date,
            r.from_title,
            r.to_title,
            r.from_pipeline_id,
            version,
            r.occurred_at,
        ])?;
    }
    Ok(())
}

/// `listForItem`: newest-first keyset page of an item's activity.
pub fn list_for_item(conn: &Connection, input: &ListActivityInput) -> Result<PipelineActivityPage> {
    let keyset = NewestFirstKeyset::new(input.limit, input.cursor.as_deref())?;
    let ts = "\"PipelineItemActivity\".\"occurredAt\"";
    let id = "\"PipelineItemActivity\".\"id\"";
    let where_sql = Sql::and([
        Some(bind1(
            "\"PipelineItemActivity\".\"workspaceId\" = ?",
            input.workspace_id,
        )),
        Some(bind1(
            "\"PipelineItemActivity\".\"itemId\" = ?",
            input.item_id,
        )),
        input.kinds.as_ref().map(|kinds| {
            bind1(
                "\"PipelineItemActivity\".\"kind\" IN (SELECT value FROM json_each(?))",
                text_list(kinds),
            )
        }),
        keyset.after(ts, id),
    ]);
    let mut sql = Sql::raw(format!("{} WHERE ", PipelineItemActivity::select_sql()));
    sql.append(where_sql);
    sql.push(&format!(
        " ORDER BY {} LIMIT ",
        NewestFirstKeyset::order_by(ts, id)
    ));
    sql.push_bind(keyset.fetch_limit());
    let rows = sql.models::<PipelineItemActivity>(conn)?;
    let (items, next_cursor) = keyset.page(rows, |r| (r.occurred_at, r.id));
    Ok(PipelineActivityPage { items, next_cursor })
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
    let day = zoned_bucket_key("\"occurredAt\"", Granularity::Day, timezone);
    let mut sql = Sql::raw("SELECT ");
    sql.append(day);
    sql.push(", \"kind\", \"toStatus\", count(*), coalesce(sum(\"toValue\"), 0) FROM \"PipelineItemActivity\" WHERE ");
    sql.append(Sql::new(ANALYTICS_WHERE, analytics_params(range)));
    sql.push(" GROUP BY 1, \"kind\", \"toStatus\"");
    sql.query_map(conn, |row| {
        Ok(DailySeriesRow {
            day: row.get(0)?,
            kind: row.get(1)?,
            to_status: row.get(2)?,
            count: row.get(3)?,
            value: row.get(4)?,
        })
    })
}

/// `totalsByActor`: the same transitions grouped by user or automation.
pub fn totals_by_actor(
    conn: &Connection,
    range: &PipelineAnalyticsRange,
) -> Result<Vec<ActorTotalsRow>> {
    let kinds = PIPELINE_AUTOMATION_ACTOR_KINDS
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let is_automation = format!("(\"actorKind\" IN ({kinds}))");
    let mut sql = Sql::raw(format!(
        "SELECT {is_automation}, \"actorUserId\", \"kind\", \"toStatus\", count(*), coalesce(sum(\"toValue\"), 0) FROM \"PipelineItemActivity\" WHERE "
    ));
    sql.append(Sql::new(ANALYTICS_WHERE, analytics_params(range)));
    sql.push(&format!(
        " GROUP BY {is_automation}, \"actorUserId\", \"kind\", \"toStatus\""
    ));
    sql.query_map(conn, |row| {
        Ok(ActorTotalsRow {
            is_automation: row.get(0)?,
            actor_user_id: row.get(1)?,
            kind: row.get(2)?,
            to_status: row.get(3)?,
            count: row.get(4)?,
            value: row.get(5)?,
        })
    })
}
