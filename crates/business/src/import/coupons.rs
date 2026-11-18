//! `couponService.startImport` and the worker's coupons import handler
//! (`handler/coupons/{handler,extractor}.ts`) on `runImportPipeline`.

use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::params;
use chatbotx_files::{FilesError, Uploader};
use serde_json::{Map, Value, json};

use super::contacts::{Counters, RunRow, complete, fail, mark_processing};
use super::{find_import_file, repo_err};
use crate::coupon::{self, MAX_COUPON_IMPORT_BYTES};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `IMPORT_BATCH_SIZE`.
const BATCH_SIZE: usize = 1000;
const MAX_CODE_LENGTH: usize = 1000;

fn file_not_found(message: &str) -> BusinessError {
    BusinessError::exception(message, "couponImportFileNotFound", 400)
}

fn topic_inactive() -> BusinessError {
    BusinessError::exception("Coupon topic is not active", "couponTopicInactive", 400)
}

/// `startImport`: a coupon CSV that was uploaded (at most 10 MiB) into an
/// active topic; the import row is pending.
pub async fn start(
    db: &Db,
    uploader: &Uploader,
    workspace_id: i64,
    user_id: Option<i64>,
    file_id: i64,
    topic_id: i64,
) -> Result<i64> {
    let (file, topic) = tx::read(db, move |conn| {
        Ok((
            find_import_file(conn, workspace_id, file_id)?,
            coupon::get_topic_in(conn, workspace_id, topic_id)?,
        ))
    })
    .await?;
    let Some((_, file_name, mime_type, path, _)) =
        file.filter(|f| f.0.as_deref() == Some("coupons"))
    else {
        return Err(file_not_found("Coupon import file not found"));
    };
    if !file_name.to_lowercase().ends_with(".csv") || mime_type != "text/csv" {
        return Err(BusinessError::exception(
            "Unsupported coupon import file",
            "couponImportUnsupportedFile",
            400,
        ));
    }
    let size = match uploader.head_object(&path).await {
        Ok(head) => i64::try_from(head.content_length).unwrap_or(i64::MAX),
        Err(_) => return Err(file_not_found("Coupon import file was not uploaded")),
    };
    if size > MAX_COUPON_IMPORT_BYTES {
        return Err(BusinessError::exception(
            "Coupon import file exceeds the maximum allowed size",
            "couponImportFileTooLarge",
            400,
        ));
    }
    if topic.is_none_or(|t| t.status != "active") {
        return Err(topic_inactive());
    }
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "File" SET "status" = 'uploaded', "uploadedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![file_id, workspace_id, now],
        )
        .map_err(repo_err)?;
        let import_id = new_id();
        conn.execute(
            r#"INSERT INTO "Import" ("id", "workspaceId", "userId", "fileId", "type", "format", "status", "meta", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, 'coupons', 'csv', 'pending', ?5, ?6, ?6)"#,
            params![import_id, workspace_id, user_id, file_id, json!({ "topicId": topic_id.to_string() }).to_string(), now],
        )
        .map_err(repo_err)?;
        Ok(import_id)
    })
    .await
}

/// `findCouponColumn`: `coupon`, `coupon code` or `code` (any case, any
/// punctuation).
fn coupon_column(row: &Map<String, Value>) -> Option<&String> {
    row.keys().find(|key| {
        let normalized: String = key
            .trim()
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        matches!(normalized.as_str(), "coupon" | "couponcode" | "code")
    })
}

/// `extractCouponRow`.
fn extract_code(row: &Map<String, Value>) -> Option<String> {
    let text = row.get(coupon_column(row)?)?.as_str()?.trim();
    let code: String = text.chars().take(MAX_CODE_LENGTH).collect();
    (!code.is_empty()).then_some(code)
}

/// `processCouponBatch`: one `importBatch`; a refused batch fails its rows.
async fn process_batch(db: &Db, ws: i64, topic_id: i64, codes: Vec<String>) -> (u64, u64) {
    let total = codes.len() as u64;
    let mut unique: Vec<String> = Vec::new();
    for code in codes {
        if !unique.contains(&code) {
            unique.push(code);
        }
    }
    match coupon::import_batch(db, ws, topic_id, unique).await {
        Ok(result) => {
            let created = result["created"].as_u64().unwrap_or(0);
            (created, total - created)
        }
        Err(error) => {
            tracing::error!(%error, "Import coupon batch failed");
            (0, total)
        }
    }
}

/// `runImportPipeline` with the coupons handler.
pub(super) async fn run_coupons(
    db: &Db,
    uploader: &Uploader,
    import_id: i64,
    row: &RunRow,
    path: &str,
) -> Result<()> {
    let topic_id = serde_json::from_str::<Value>(&row.meta)
        .ok()
        .and_then(|m| m["topicId"].as_str().and_then(|t| t.parse::<i64>().ok()));
    let Some(topic_id) = topic_id else {
        return fail(db, import_id, "Invalid coupon import meta", None, None).await;
    };
    let ws = row.workspace_id;
    // `prepareCoupons`.
    let active = tx::read(db, move |conn| coupon::get_topic_in(conn, ws, topic_id))
        .await?
        .is_some_and(|t| t.status == "active");
    if !active {
        return fail(db, import_id, "Coupon topic is not active", None, None).await;
    }
    mark_processing(db, import_id).await?;
    let bytes = match uploader.get_object(path).await {
        Ok(bytes) => bytes,
        Err(FilesError::NotFound(_)) => {
            return fail(db, import_id, "The file was not found", None, None).await;
        }
        Err(error) => return fail(db, import_id, &error.to_string(), None, None).await,
    };
    let mut counters = Counters::default();
    let rows = match super::products::read_rows(&row.format, &bytes) {
        Ok(rows) => rows,
        Err(error) => return fail(db, import_id, &error, Some(counters), None).await,
    };
    let mut buffer = Vec::new();
    for raw in rows {
        counters.processed += 1;
        match extract_code(&raw) {
            Some(code) => buffer.push(code),
            None => counters.failed += 1,
        }
        if buffer.len() >= BATCH_SIZE {
            let (success, failed) =
                process_batch(db, ws, topic_id, std::mem::take(&mut buffer)).await;
            counters.success += success;
            counters.failed += failed;
        }
    }
    if !buffer.is_empty() {
        let (success, failed) = process_batch(db, ws, topic_id, buffer).await;
        counters.success += success;
        counters.failed += failed;
    }
    complete(db, import_id, counters, Vec::new()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_code_column() {
        let row = |k: &str, v: &str| {
            let mut m = Map::new();
            m.insert("Name".into(), json!("x"));
            m.insert(k.into(), json!(v));
            m
        };
        assert_eq!(
            extract_code(&row("Coupon Code", " A-1 ")).as_deref(),
            Some("A-1")
        );
        assert_eq!(extract_code(&row("COUPON", "B")).as_deref(), Some("B"));
        assert_eq!(extract_code(&row("code", "")), None);
        assert_eq!(extract_code(&row("Other", "C")), None);
    }
}
