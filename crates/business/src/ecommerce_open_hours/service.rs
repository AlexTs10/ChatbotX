//! `ecommerce-open-hours/service.ts` (`ecommerceOpenHoursService`), with
//! `ecommerceOpenHoursRepository` as private queries (they move to `crates/db`
//! later). The original drops the `ecommerce-open-hours:` / `ecommerce-settings:`
//! cache tags after each write; the Rust port has no read cache.

use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Map, Value, json};

use super::schedule::{OpenHoursRange, OpenHoursSchedule, parse_schedule, ranges_from_json};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// One `"EcommerceOpenHours"` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHoursRow {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    pub ranges: Vec<OpenHoursRange>,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLUMNS: &str = "\"id\", \"workspaceId\", \"name\", \"ranges\", \"createdAt\", \"updatedAt\"";

fn map_row(r: &chatbotx_db::rusqlite::Row<'_>) -> chatbotx_db::rusqlite::Result<OpenHoursRow> {
    let ranges: String = r.get(3)?;
    Ok(OpenHoursRow {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        name: r.get(2)?,
        ranges: ranges_from_json(&ranges),
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn repo_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::from(error))
}

fn ranges_text(ranges: &[OpenHoursRange]) -> String {
    Value::Array(ranges.iter().map(OpenHoursRange::to_json).collect()).to_string()
}

/// A zod parse failure (`ZodError` → `422 invalidRequestData` at the API).
fn invalid(issues: Vec<Value>) -> BusinessError {
    BusinessError::Exception {
        message: "Input validation failed".to_string(),
        code: "invalidRequestData".to_string(),
        status: 422,
        field: None,
        data: Some(json!({ "issues": issues })),
    }
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Open-hours schedule not found")
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<OpenHoursRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM \"EcommerceOpenHours\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2"),
        [id, workspace_id],
        map_row,
    )
    .optional()
    .map_err(repo_error)
}

/// `ecommerceOpenHoursService.list`: by name, then id.
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<OpenHoursRow>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT {COLUMNS} FROM \"EcommerceOpenHours\" WHERE \"workspaceId\" = ?1 ORDER BY \"name\" ASC, \"id\" ASC"
            ))
            .map_err(repo_error)?;
        let rows = stmt
            .query_map([workspace_id], map_row)
            .map_err(repo_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_error)?;
        Ok(rows)
    })
    .await
}

/// `ecommerceOpenHoursService.create`: the schedule is parsed (name trimmed,
/// 1–28 valid ranges) before the insert.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    input: &Map<String, Value>,
) -> Result<OpenHoursRow> {
    let schedule = parse_schedule(input, false).map_err(invalid)?;
    let OpenHoursSchedule {
        name: Some(name),
        ranges: Some(ranges),
    } = schedule
    else {
        return Err(invalid(Vec::new()));
    };
    tx::write(db, move |conn| {
        let (id, now) = (new_id(), now_ms());
        conn.execute(
            "INSERT INTO \"EcommerceOpenHours\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\", \"ranges\") \
             VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
            chatbotx_db::rusqlite::params![id, now, workspace_id, name, ranges_text(&ranges)],
        )
        .map_err(repo_error)?;
        find_in(conn, workspace_id, id)?
            .ok_or_else(|| BusinessError::exception("Failed to create the open-hours schedule", "systemError", 400))
    })
    .await
}

/// `ecommerceOpenHoursService.update`: only the sent parts change; `ranges`
/// are replaced as a whole.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: &Map<String, Value>,
) -> Result<OpenHoursRow> {
    let schedule = parse_schedule(data, true).map_err(invalid)?;
    tx::write(db, move |conn| {
        let mut sets = vec!["\"updatedAt\" = ?1".to_string()];
        let mut args: Vec<SqlValue> = vec![SqlValue::Integer(now_ms())];
        if let Some(name) = schedule.name {
            args.push(SqlValue::Text(name));
            sets.push(format!("\"name\" = ?{}", args.len()));
        }
        if let Some(ranges) = &schedule.ranges {
            args.push(SqlValue::Text(ranges_text(ranges)));
            sets.push(format!("\"ranges\" = ?{}", args.len()));
        }
        args.push(SqlValue::Integer(id));
        let id_at = args.len();
        args.push(SqlValue::Integer(workspace_id));
        let sql = format!(
            "UPDATE \"EcommerceOpenHours\" SET {} WHERE \"id\" = ?{id_at} AND \"workspaceId\" = ?{}",
            sets.join(", "),
            id_at + 1
        );
        let changed = conn.execute(&sql, params_from_iter(args)).map_err(repo_error)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_in(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `ecommerceOpenHoursService.delete`: store settings that used it fall back
/// to always open (`openHoursId` set to null by the foreign key).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        let removed = conn
            .execute(
                "DELETE FROM \"EcommerceOpenHours\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
                [id, workspace_id],
            )
            .map_err(repo_error)?;
        if removed == 0 {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}

/// `ecommerceOpenHoursService.assertBelongsToWorkspace`.
pub async fn assert_belongs_to_workspace(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<OpenHoursRow> {
    tx::read(db, move |conn| {
        find_in(conn, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("Open-hours schedule does not exist."))
    })
    .await
}
