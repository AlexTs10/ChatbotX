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
    todo!("P3.6-D1")
}

/// `ecommerceOpenHoursService.create`: the schedule is parsed (name trimmed,
/// 1–28 valid ranges) before the insert.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    input: &Map<String, Value>,
) -> Result<OpenHoursRow> {
    todo!("P3.6-D1")
}

/// `ecommerceOpenHoursService.update`: only the sent parts change; `ranges`
/// are replaced as a whole.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: &Map<String, Value>,
) -> Result<OpenHoursRow> {
    todo!("P3.6-D1")
}

/// `ecommerceOpenHoursService.delete`: store settings that used it fall back
/// to always open (`openHoursId` set to null by the foreign key).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    todo!("P3.6-D1")
}

/// `ecommerceOpenHoursService.assertBelongsToWorkspace`.
pub async fn assert_belongs_to_workspace(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<OpenHoursRow> {
    todo!("P3.6-D1")
}
